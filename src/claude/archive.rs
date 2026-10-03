//! `.chatkeep` archives of a Claude Code project.
//!
//! Layout: `manifest.json`, `project/` (the folder under `~/.claude/projects`: transcripts,
//! subagents, tool output, memory), `file-history/<session>/` (the checkpoints behind
//! rewind), `todos/`, `settings.json` (the project's entry in `~/.claude.json`),
//! `prompts.jsonl` (its lines of the prompt history), and
//! `desktop/<account>/<organization>/local_<id>.json` (the desktop app's entries).

use anyhow::{Context, Result, bail};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use tar::Builder;

use super::desktop::{self, DesktopSession};
use super::ops::{
    add_memory_lines, ensure_dir, files_under, in_journal, is_memory_index, project_label,
    require_idle, write_keeping_mode,
};
use super::slug::project_slug;
use super::store::{self, Project};
use super::{Runtime, config};
use crate::cursor::rewrite::{Boundary, Replacement, Rewriter, path_replacements};
use crate::cursor::uri::Platform;
use crate::engine::Report;
use crate::engine::archive::{
    CLAUDE, file_entry, is_uuid, safe_relative, unpack, valid_slug, verify_checksums,
};
use crate::engine::fsops::{SpaceNeeds, copy_tree, dir_size, exists, write_atomic};
use crate::engine::journal::Journal;
use crate::ui;

const VERSION: u64 = 1;

const PROJECT: &str = "project";
const CHECKPOINTS: &str = "file-history";
const TODOS: &str = "todos";
const SETTINGS: &str = "settings.json";
const PROMPTS: &str = "prompts.jsonl";
const DESKTOP: &str = "desktop";

/// One file of the archive: where it is on disk and its name inside the archive.
struct Packed {
    source: PathBuf,
    name: String,
}

fn pack_tree(root: &Path, prefix: &str, out: &mut Vec<Packed>) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("refusing to export symlink {}", entry.path().display());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative: Vec<String> = entry
            .path()
            .strip_prefix(root)?
            .components()
            .map(|part| part.as_os_str().to_string_lossy().to_string())
            .collect();
        out.push(Packed {
            source: entry.path().to_path_buf(),
            name: format!("{prefix}/{}", relative.join("/")),
        });
    }
    Ok(())
}

/// Desktop app entries that belong to `project`: they run in its folder or point at one of
/// its sessions.
fn desktop_entries(rt: &Runtime, project: &Project) -> Result<Vec<DesktopSession>> {
    let path = project
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned());
    Ok(desktop::sessions(&rt.layout)?
        .into_iter()
        .filter(|session| {
            (path.is_some() && session.cwd == path)
                || session
                    .cli_session
                    .as_deref()
                    .is_some_and(|id| project.session(id).is_some())
        })
        .collect())
}

pub fn export(rt: &Runtime, project: &Project, file: &Path) -> Result<Report> {
    let file = rt.resolve_path(file);
    if exists(&file) {
        bail!("refusing to overwrite {}", file.display());
    }
    let path = project
        .path
        .as_ref()
        .with_context(|| {
            format!(
                "chatkeep cannot tell which folder {} belongs to",
                project.slug
            )
        })?
        .to_string_lossy()
        .into_owned();
    let mut report = Report::default();
    report.applied.push(format!(
        "export project {} ({}) -> {}",
        project_label(project),
        ui::plural(project.sessions.len(), "session", "sessions"),
        file.display()
    ));
    if rt.dry_run {
        return Ok(report);
    }
    let ids: HashSet<String> = project
        .sessions
        .iter()
        .map(|session| session.id.clone())
        .collect();
    require_idle(rt, &BTreeSet::from([project.slug.clone()]), &ids, false)?;

    let parent = file.parent().context("archive path has no parent")?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    let mut packed = Vec::new();
    pack_tree(&project.dir, PROJECT, &mut packed)?;
    for session in &project.sessions {
        pack_tree(
            &rt.layout.config_dir.join(CHECKPOINTS).join(&session.id),
            &format!("{CHECKPOINTS}/{}", session.id),
            &mut packed,
        )?;
    }
    let todos = rt.layout.config_dir.join(TODOS);
    if todos.is_dir() {
        let mut names: Vec<String> = fs::read_dir(&todos)?
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| ids.iter().any(|id| name.starts_with(id.as_str())))
            .collect();
        names.sort();
        for name in names {
            packed.push(Packed {
                source: todos.join(&name),
                name: format!("{TODOS}/{name}"),
            });
        }
    }
    if let Some(settings) = config::read(&rt.layout.config_file)?
        .as_ref()
        .and_then(|json| json.get("projects"))
        .and_then(|projects| projects.get(&path))
    {
        let staged = staging.path().join(SETTINGS);
        fs::write(&staged, serde_json::to_vec_pretty(settings)?)?;
        packed.push(Packed {
            source: staged,
            name: SETTINGS.to_string(),
        });
    }
    let prompts = project_prompts(&rt.layout.prompt_history(), &path, &ids)?;
    if !prompts.is_empty() {
        let staged = staging.path().join(PROMPTS);
        fs::write(&staged, prompts)?;
        packed.push(Packed {
            source: staged,
            name: PROMPTS.to_string(),
        });
    }
    for entry in desktop_entries(rt, project)? {
        let name = entry
            .file
            .file_name()
            .context("desktop session file has no name")?
            .to_string_lossy()
            .to_string();
        packed.push(Packed {
            name: format!("{DESKTOP}/{}/{}/{name}", entry.account, entry.organization),
            source: entry.file,
        });
    }

    let files: Vec<Value> = packed
        .iter()
        .map(|item| file_entry(&item.name, &item.source))
        .collect::<Result<_>>()?;
    let mut sessions: Vec<&String> = ids.iter().collect();
    sessions.sort();
    let manifest = json!({
        "version": VERSION,
        "tool": CLAUDE,
        "project": {"path": path, "slug": project.slug},
        "sessions": sessions,
        "files": files,
    });
    let manifest_path = staging.path().join("manifest.json");
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    let partial = tempfile::NamedTempFile::new_in(parent)?;
    {
        let gz = GzEncoder::new(BufWriter::new(partial.as_file()), Compression::default());
        let mut builder = Builder::new(gz);
        builder.follow_symlinks(false);
        for item in &packed {
            builder.append_path_with_name(&item.source, &item.name)?;
        }
        builder.append_path_with_name(&manifest_path, "manifest.json")?;
        builder.into_inner()?.finish()?.flush()?;
    }
    partial.as_file().sync_all()?;
    partial
        .persist_noclobber(&file)
        .map_err(|err| err.error)
        .with_context(|| format!("failed to write {}", file.display()))?;
    Ok(report)
}

/// The prompt history lines typed in `path` or in one of `ids`, each with its newline.
fn project_prompts(history: &Path, path: &str, ids: &HashSet<String>) -> Result<String> {
    if !history.is_file() {
        return Ok(String::new());
    }
    let raw = fs::read_to_string(history)
        .with_context(|| format!("failed to read {}", history.display()))?;
    let mut out = String::new();
    for line in raw.lines() {
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let in_project = json.get("project").and_then(Value::as_str) == Some(path);
        let in_session = json
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(id));
        if in_project || in_session {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

struct Archived {
    path: String,
    slug: String,
    sessions: Vec<String>,
}

fn read_manifest(staging: &Path) -> Result<Archived> {
    let manifest: Value = serde_json::from_str(
        &fs::read_to_string(staging.join("manifest.json")).context("archive has no manifest")?,
    )
    .context("archive manifest is corrupt")?;
    verify_checksums(staging, &manifest)?;
    match manifest.get("tool").and_then(Value::as_str) {
        Some(CLAUDE) => {}
        Some(other) => bail!("this archive holds {other} chats, not Claude Code chats"),
        None => bail!("this archive holds Cursor chats, not Claude Code chats"),
    }
    let version = manifest.get("version").and_then(Value::as_u64).unwrap_or(0);
    if version != VERSION {
        bail!("unsupported archive version {version}");
    }
    let text = |pointer: &str| {
        manifest
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("manifest is missing {pointer}"))
    };
    let slug = text("/project/slug")?;
    if !valid_slug(&slug) {
        bail!("archive has an invalid project folder name {slug:?}");
    }
    let sessions: Vec<String> = manifest
        .get("sessions")
        .and_then(Value::as_array)
        .context("manifest is missing sessions")?
        .iter()
        .filter_map(|id| id.as_str().map(str::to_string))
        .collect();
    for id in &sessions {
        if !is_uuid(id) {
            bail!("archive has an invalid session id {id:?}");
        }
    }
    Ok(Archived {
        path: text("/project/path")?,
        slug,
        sessions,
    })
}

/// Copy `source` to `dest`, rewriting it when it is text.
fn restore_file(source: &Path, dest: &Path, rewriter: &Rewriter) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
    let out = match String::from_utf8(bytes) {
        Ok(text) => rewriter.rewrite(&text).into_owned().into_bytes(),
        Err(raw) => raw.into_bytes(),
    };
    write_atomic(dest, &out)
}

fn restore_tree(
    journal: &mut Journal,
    source: &Path,
    dest: &Path,
    rewriter: &Rewriter,
) -> Result<()> {
    journal.created(dest)?;
    for file in files_under(source)? {
        restore_file(&file, &dest.join(file.strip_prefix(source)?), rewriter)?;
    }
    Ok(())
}

pub fn import(rt: &Runtime, file: &Path, dest: Option<&Path>, overwrite: bool) -> Result<Report> {
    let staging = tempfile::tempdir()?;
    let staged = staging.path();
    unpack(&rt.resolve_path(file), staged)?;
    let archived = read_manifest(staged)?;
    let to = match dest {
        Some(dest) => {
            let dest = rt.resolve_path(dest);
            if !dest.is_dir() {
                bail!("import destination does not exist: {}", dest.display());
            }
            dest
        }
        None => PathBuf::from(&archived.path),
    };
    let to_str = to.to_string_lossy().into_owned();
    let dest_slug = project_slug(&to_str);
    let dest_dir = rt.layout.projects_dir().join(&dest_slug);
    let projects = store::discover(rt)?;
    let mut report = Report::default();
    let mut imported = Vec::new();
    let mut replaced: Vec<(Project, String)> = Vec::new();
    for id in &archived.sessions {
        if !staged.join(PROJECT).join(format!("{id}.jsonl")).is_file() {
            bail!("archive lists session {id} but holds no transcript for it");
        }
        match store::find_session(&projects, id) {
            Some((owner, _)) if overwrite => {
                report
                    .warnings
                    .push(format!("replacing existing session {id}"));
                replaced.push((owner.clone(), id.clone()));
                imported.push(id.clone());
            }
            Some(_) => {
                report.warnings.push(format!(
                    "skipped {id}: session already exists (pass --overwrite to replace)"
                ));
                report.skipped.push(id.clone());
            }
            None => imported.push(id.clone()),
        }
    }
    report.applied.push(format!(
        "import {} -> {}",
        ui::plural(imported.len(), "session", "sessions"),
        ui::home_relative(&to_str)
    ));
    if rt.dry_run {
        return Ok(report);
    }
    let mut needs = SpaceNeeds::default();
    needs.add(
        &rt.layout.projects_dir(),
        dir_size(staged)?,
        "imported project",
    );
    needs.check()?;

    let mut entries: Vec<(Replacement, Boundary)> = Vec::new();
    if archived.path != to_str {
        entries.extend(
            path_replacements(Platform::current(), &archived.path, &to_str)
                .into_iter()
                .map(|item| (item, Boundary::Path)),
        );
        entries.push((
            Replacement::new(&archived.slug, &dest_slug),
            Boundary::Token,
        ));
    }
    let rewriter = Rewriter::build(entries);
    let wanted: HashSet<String> = imported.iter().cloned().collect();
    let mut slugs = BTreeSet::from([dest_slug.clone()]);
    slugs.extend(replaced.iter().map(|(owner, _)| owner.slug.clone()));
    let desktop_now = desktop::sessions(&rt.layout)?;
    let stale_desktop: Vec<PathBuf> = desktop_now
        .iter()
        .filter(|entry| {
            entry
                .cli_session
                .as_deref()
                .is_some_and(|id| replaced.iter().any(|(_, replaced)| replaced == id))
        })
        .map(|entry| entry.file.clone())
        .collect();
    require_idle(rt, &slugs, &wanted, !stale_desktop.is_empty())?;

    in_journal(rt, "claude-import", |journal| {
        for (owner, id) in &replaced {
            journal.stash(&owner.dir.join(format!("{id}.jsonl")))?;
            journal.stash(&owner.dir.join(id))?;
            journal.stash(&rt.layout.config_dir.join(CHECKPOINTS).join(id))?;
        }
        ensure_dir(journal, &dest_dir)?;
        let packed = staged.join(PROJECT);
        for entry in
            fs::read_dir(&packed).with_context(|| format!("failed to read {}", packed.display()))?
        {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let source = entry.path();
            let target = dest_dir.join(&name);
            let session = name.strip_suffix(".jsonl").unwrap_or(&name);
            if archived.sessions.iter().any(|id| id == session) {
                if !wanted.contains(session) {
                    continue;
                }
                if source.is_dir() {
                    restore_tree(journal, &source, &target, &rewriter)?;
                } else {
                    journal.created(&target)?;
                    restore_file(&source, &target, &rewriter)?;
                }
            } else if name == "memory" && source.is_dir() {
                restore_memory(journal, &source, &target, &rewriter, overwrite, &mut report)?;
            } else if !exists(&target) {
                if source.is_dir() {
                    restore_tree(journal, &source, &target, &rewriter)?;
                } else {
                    journal.created(&target)?;
                    restore_file(&source, &target, &rewriter)?;
                }
            }
        }
        for id in &imported {
            let source = staged.join(CHECKPOINTS).join(id);
            if source.is_dir() {
                let checkpoints = rt.layout.config_dir.join(CHECKPOINTS);
                ensure_dir(journal, &checkpoints)?;
                let target = checkpoints.join(id);
                journal.created(&target)?;
                copy_tree(&source, &target)?;
            }
        }
        restore_todos(rt, journal, staged, &wanted, overwrite)?;
        restore_settings(rt, journal, staged, &to_str, &rewriter)?;
        restore_prompts(rt, journal, staged, &archived, &to_str, &wanted)?;
        for file in &stale_desktop {
            journal.stash(file)?;
        }
        restore_desktop(rt, journal, staged, &wanted, &rewriter, &mut report)?;
        for id in &imported {
            let landed = dest_dir.join(format!("{id}.jsonl"));
            if !landed.is_file() {
                bail!("verify failed: {} is missing", landed.display());
            }
        }
        Ok(())
    })?;
    Ok(report)
}

/// Memory files the destination lacks are added, the index gains the lines it lacks, and a
/// file both hold with different text is kept unless `--overwrite` was passed.
fn restore_memory(
    journal: &mut Journal,
    source: &Path,
    dest: &Path,
    rewriter: &Rewriter,
    overwrite: bool,
    report: &mut Report,
) -> Result<()> {
    for file in files_under(source)? {
        let target = dest.join(file.strip_prefix(source)?);
        if !exists(&target) {
            journal.created(&target)?;
            restore_file(&file, &target, rewriter)?;
            continue;
        }
        let Ok(theirs) = fs::read_to_string(&file) else {
            continue;
        };
        let theirs = rewriter.rewrite(&theirs).into_owned();
        if is_memory_index(&file) {
            add_memory_lines(journal, &theirs, &target)?;
        } else if fs::read(&target)? != theirs.as_bytes() {
            if overwrite {
                journal.save(&target)?;
                write_keeping_mode(&target, theirs.as_bytes())?;
            } else {
                report.warnings.push(format!(
                    "kept existing memory file {} (pass --overwrite to replace)",
                    ui::home_relative(&target.display().to_string())
                ));
            }
        }
    }
    Ok(())
}

fn restore_todos(
    rt: &Runtime,
    journal: &mut Journal,
    staged: &Path,
    wanted: &HashSet<String>,
    overwrite: bool,
) -> Result<()> {
    let packed = staged.join(TODOS);
    if !packed.is_dir() {
        return Ok(());
    }
    let dest = rt.layout.config_dir.join(TODOS);
    for file in files_under(&packed)? {
        let name = file
            .file_name()
            .context("todo file has no name")?
            .to_string_lossy()
            .to_string();
        if !wanted.iter().any(|id| name.starts_with(id.as_str())) {
            continue;
        }
        let target = dest.join(&name);
        if exists(&target) {
            if !overwrite {
                continue;
            }
            journal.stash(&target)?;
        }
        ensure_dir(journal, &dest)?;
        journal.created(&target)?;
        fs::copy(&file, &target)?;
    }
    Ok(())
}

/// The project's settings, unless the folder already has its own.
fn restore_settings(
    rt: &Runtime,
    journal: &mut Journal,
    staged: &Path,
    to: &str,
    rewriter: &Rewriter,
) -> Result<()> {
    let packed = staged.join(SETTINGS);
    if !packed.is_file() {
        return Ok(());
    }
    let Some(mut json) = config::read(&rt.layout.config_file)? else {
        return Ok(());
    };
    if config::has_project(&json, to) {
        return Ok(());
    }
    let text = rewriter.rewrite(&fs::read_to_string(&packed)?).into_owned();
    let settings: Value = serde_json::from_str(&text).context("archived settings are corrupt")?;
    let Some(root) = json.as_object_mut() else {
        return Ok(());
    };
    let projects = root
        .entry("projects")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(projects) = projects.as_object_mut() else {
        return Ok(());
    };
    projects.insert(to.to_string(), settings);
    journal.save(&rt.layout.config_file)?;
    write_keeping_mode(
        &rt.layout.config_file,
        serde_json::to_string_pretty(&json)?.as_bytes(),
    )
}

/// The archived prompts the history does not hold yet, pointed at the import folder.
fn restore_prompts(
    rt: &Runtime,
    journal: &mut Journal,
    staged: &Path,
    archived: &Archived,
    to: &str,
    wanted: &HashSet<String>,
) -> Result<()> {
    let packed = staged.join(PROMPTS);
    if !packed.is_file() {
        return Ok(());
    }
    let history = rt.layout.prompt_history();
    let current = if history.is_file() {
        fs::read_to_string(&history)?
    } else {
        String::new()
    };
    let present: HashSet<&str> = current.lines().collect();
    let mut added = String::new();
    for line in fs::read_to_string(&packed)?.lines() {
        let Ok(mut json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let session = json.get("sessionId").and_then(Value::as_str);
        if session.is_some_and(|id| archived.sessions.iter().any(|known| known == id))
            && !session.is_some_and(|id| wanted.contains(id))
        {
            continue;
        }
        let line = if json.get("project").and_then(Value::as_str) == Some(archived.path.as_str())
            && archived.path != to
        {
            json["project"] = Value::String(to.to_string());
            serde_json::to_string(&json)?
        } else {
            line.to_string()
        };
        if !present.contains(line.as_str()) {
            added.push_str(&line);
            added.push('\n');
        }
    }
    if added.is_empty() {
        return Ok(());
    }
    let mut merged = current.clone();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(&added);
    journal.save(&history)?;
    if history.is_file() {
        write_keeping_mode(&history, merged.as_bytes())
    } else {
        write_atomic(&history, merged.as_bytes())
    }
}

/// The desktop app's entries of the imported sessions, under the account they were exported
/// from. An entry the app already has is left alone.
fn restore_desktop(
    rt: &Runtime,
    journal: &mut Journal,
    staged: &Path,
    wanted: &HashSet<String>,
    rewriter: &Rewriter,
    report: &mut Report,
) -> Result<()> {
    let packed = staged.join(DESKTOP);
    let Some(root) = rt.layout.desktop_sessions_dir() else {
        return Ok(());
    };
    if !packed.is_dir() {
        return Ok(());
    }
    let current = super::accounts::current(&rt.layout);
    let mut elsewhere = BTreeSet::new();
    let mut restored = 0usize;
    for file in files_under(&packed)? {
        let relative = file.strip_prefix(&packed)?;
        if !safe_relative(relative) || relative.components().count() != 3 {
            bail!(
                "archive has an unexpected desktop entry {}",
                relative.display()
            );
        }
        let text = rewriter.rewrite(&fs::read_to_string(&file)?).into_owned();
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let session = json.get("cliSessionId").and_then(Value::as_str);
        if !session.is_some_and(|id| wanted.contains(id)) {
            continue;
        }
        let target = root.join(relative);
        if exists(&target) {
            continue;
        }
        if let Some(parent) = target.parent() {
            ensure_dir(journal, parent)?;
        }
        journal.created(&target)?;
        write_atomic(&target, text.as_bytes())?;
        restored += 1;
        let account = relative
            .components()
            .next()
            .map(|part| part.as_os_str().to_string_lossy().to_string())
            .unwrap_or_default();
        if current.as_deref().is_some_and(|current| current != account) {
            elsewhere.insert(account);
        }
    }
    if restored > 0 {
        report.applied.push(format!(
            "add {}",
            ui::plural(restored, "desktop app session", "desktop app sessions")
        ));
    }
    for account in elsewhere {
        report.warnings.push(format!(
            "the desktop app lists these chats under account {account}, not the one signed in now; run: chatkeep claude accounts cp {account}"
        ));
    }
    Ok(())
}
