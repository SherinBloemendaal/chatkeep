//! `cp`, `split`, and `combine` for Claude Code: sessions copied or moved to the project of
//! another folder.
//!
//! A copy gets a new session id, so both chats can go on separately. `cp` treats the new
//! folder as a second home of the whole project and rewrites every path in the copies;
//! `split` and `combine` only hand chats to another project and leave the paths they mention
//! alone, because the files those chats worked on did not move.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::desktop::{self, DesktopSession};
use super::live;
use super::ops::{
    self, LineEdit, add_memory_lines, check_new_folder, edit_lines, ensure_dir, files_under,
    in_journal, is_memory_index, project_label, require_idle, write_keeping_mode,
};
use super::slug::project_slug;
use super::store::{self, Project, Session};
use super::{Runtime, config};
use crate::cursor::rewrite::{Boundary, Replacement, Rewriter, path_replacements};
use crate::cursor::uri::Platform;
use crate::engine::fsops::{SpaceNeeds, copy_tree, dir_size, exists, write_atomic};
use crate::engine::journal::Journal;
use crate::engine::{Report, SplitSuggestion};
use crate::ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Copy,
    Move,
}

impl Mode {
    fn verb(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
        }
    }
}

/// Sessions of one project going to the project of one other folder.
#[derive(Debug, Clone)]
pub struct Batch {
    pub source: Project,
    pub from: String,
    pub to: PathBuf,
    pub dest_dir: PathBuf,
    pub mode: Mode,
    /// `cp`: the destination is a second home of the whole project, so paths are rewritten.
    relocate: bool,
    /// Old session id to the id it has at the destination; the same id for a move.
    ids: Vec<(Session, String)>,
    desktop: Vec<DesktopSession>,
}

impl Batch {
    fn new(
        rt: &Runtime,
        source: &Project,
        sessions: Vec<Session>,
        to: PathBuf,
        mode: Mode,
        relocate: bool,
        desktop: &[DesktopSession],
    ) -> Result<Self> {
        let from = source
            .path
            .as_ref()
            .with_context(|| {
                format!(
                    "chatkeep cannot tell which folder {} belongs to",
                    source.slug
                )
            })?
            .to_string_lossy()
            .into_owned();
        let dest_dir = rt
            .layout
            .projects_dir()
            .join(project_slug(&to.to_string_lossy()));
        if dest_dir == source.dir {
            bail!(
                "{} and {} share one Claude Code project folder",
                ui::home_relative(&from),
                ui::home_relative(&to.display().to_string())
            );
        }
        let wanted: HashSet<&str> = sessions.iter().map(|session| session.id.as_str()).collect();
        let desktop = desktop
            .iter()
            .filter(|session| {
                session
                    .cli_session
                    .as_deref()
                    .is_some_and(|id| wanted.contains(id))
            })
            .cloned()
            .collect();
        let ids = sessions
            .into_iter()
            .map(|session| {
                let id = match mode {
                    Mode::Copy => uuid::Uuid::new_v4().to_string(),
                    Mode::Move => session.id.clone(),
                };
                (session, id)
            })
            .collect();
        Ok(Self {
            source: source.clone(),
            from,
            to,
            dest_dir,
            mode,
            relocate,
            ids,
            desktop,
        })
    }

    fn to_str(&self) -> String {
        self.to.to_string_lossy().into_owned()
    }

    fn dest_slug(&self) -> String {
        project_slug(&self.to_str())
    }

    /// What changes in the text of a session that goes to the destination.
    fn rewriter(&self) -> Rewriter {
        let to = self.to_str();
        let dest_slug = self.dest_slug();
        let mut entries: Vec<(Replacement, Boundary)> = Vec::new();
        if self.relocate {
            entries.extend(
                path_replacements(Platform::current(), &self.from, &to)
                    .into_iter()
                    .map(|item| (item, Boundary::Path)),
            );
            entries.push((
                Replacement::new(&self.source.slug, &dest_slug),
                Boundary::Token,
            ));
        } else {
            // The folder a session runs in, as Claude Code writes it on every line.
            entries.push((
                Replacement::new(cwd_member(&self.from), cwd_member(&to)),
                Boundary::Token,
            ));
            // The session's own folder under ~/.claude/projects, e.g. its saved tool output.
            for (session, new) in &self.ids {
                for separator in ["/", "\\", "\\\\"] {
                    entries.push((
                        Replacement::new(
                            format!("{}{separator}{}", self.source.slug, session.id),
                            format!("{dest_slug}{separator}{new}"),
                        ),
                        Boundary::Token,
                    ));
                }
            }
        }
        for (session, new) in &self.ids {
            entries.push((Replacement::new(&session.id, new), Boundary::Token));
        }
        Rewriter::build(entries)
    }

    fn describe(&self) -> String {
        format!(
            "{} {} {} -> {}",
            self.mode.verb(),
            ui::plural(self.ids.len(), "session", "sessions"),
            ui::home_relative(&self.from),
            ui::home_relative(&self.to_str())
        )
    }

    fn bytes(&self) -> u64 {
        self.ids.iter().map(|(session, _)| session.size).sum()
    }
}

/// `"cwd":"<path>"` exactly as a JSON line holds it.
fn cwd_member(path: &str) -> String {
    format!("\"cwd\":{}", Value::String(path.to_string()))
}

/// Whether `line` is the note Claude Code keeps about the remote session a chat is linked
/// to. A copy must not claim the link of its original.
fn is_remote_link(line: &str) -> bool {
    line.contains("\"bridge-session\"")
        && serde_json::from_str::<Value>(line.trim_end())
            .ok()
            .and_then(|json| {
                json.get("type")
                    .and_then(Value::as_str)
                    .map(|kind| kind == "bridge-session")
            })
            .unwrap_or(false)
}

fn copied_transcript(text: &str, rewriter: &Rewriter) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        if !is_remote_link(line) {
            out.push_str(&rewriter.rewrite(line));
        }
    }
    out
}

/// Copy one file, rewriting it when it is text.
fn copy_rewritten(source: &Path, dest: &Path, rewriter: &Rewriter, transcript: bool) -> Result<()> {
    let bytes = fs::read(source).with_context(|| format!("failed to read {}", source.display()))?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let out = match String::from_utf8(bytes) {
        Ok(text) if transcript => copied_transcript(&text, rewriter).into_bytes(),
        Ok(text) => rewriter.rewrite(&text).into_owned().into_bytes(),
        Err(raw) => raw.into_bytes(),
    };
    write_atomic(dest, &out)?;
    if let Ok(meta) = fs::metadata(source) {
        fs::set_permissions(dest, meta.permissions())
            .with_context(|| format!("failed to set permissions of {}", dest.display()))?;
    }
    Ok(())
}

fn copy_tree_rewritten(source: &Path, dest: &Path, rewriter: &Rewriter) -> Result<()> {
    fs::create_dir_all(dest)?;
    for file in files_under(source)? {
        let relative = file.strip_prefix(source)?;
        copy_rewritten(&file, &dest.join(relative), rewriter, false)?;
    }
    Ok(())
}

fn rewrite_in_place(journal: &mut Journal, path: &Path, rewriter: &Rewriter) -> Result<()> {
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(());
    };
    let updated = rewriter.rewrite(&text);
    if updated == text {
        return Ok(());
    }
    journal.save(path)?;
    write_keeping_mode(path, updated.as_bytes())
}

/// The desktop app's entry for a copied session: a new entry for the new session id.
fn clone_desktop(
    journal: &mut Journal,
    batch: &Batch,
    session: &DesktopSession,
    new_id: &str,
    rewriter: &Rewriter,
) -> Result<()> {
    let raw = fs::read_to_string(&session.file)
        .with_context(|| format!("failed to read {}", session.file.display()))?;
    let Ok(mut json) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    let name = format!("local_{}", uuid::Uuid::new_v4());
    let Some(object) = json.as_object_mut() else {
        return Ok(());
    };
    object.insert("sessionId".to_string(), Value::String(name.clone()));
    object.insert(
        "cliSessionId".to_string(),
        Value::String(new_id.to_string()),
    );
    // The link to the original's remote session stays with the original.
    object.shift_remove("bridgeSessionIds");
    if !batch.relocate {
        repoint_desktop_cwd(object, &batch.from, &batch.to_str());
    }
    let mut text = serde_json::to_string(&json)?;
    if batch.relocate {
        text = rewriter.rewrite(&text).into_owned();
    }
    let dir = session
        .file
        .parent()
        .context("desktop session file has no folder")?;
    let file = dir.join(format!("{name}.json"));
    journal.created(&file)?;
    write_atomic(&file, text.as_bytes())?;
    if let Ok(meta) = fs::metadata(&session.file) {
        fs::set_permissions(&file, meta.permissions())?;
    }
    Ok(())
}

/// Point a desktop app entry at `to` when it runs in `from`. Returns whether it changed.
fn repoint_desktop_cwd(object: &mut serde_json::Map<String, Value>, from: &str, to: &str) -> bool {
    let mut changed = false;
    for key in ["cwd", "originCwd"] {
        if object.get(key).and_then(Value::as_str) == Some(from) {
            object.insert(key.to_string(), Value::String(to.to_string()));
            changed = true;
        }
    }
    changed
}

fn move_desktop(journal: &mut Journal, batch: &Batch, session: &DesktopSession) -> Result<()> {
    let raw = fs::read_to_string(&session.file)
        .with_context(|| format!("failed to read {}", session.file.display()))?;
    let Ok(mut json) = serde_json::from_str::<Value>(&raw) else {
        return Ok(());
    };
    let Some(object) = json.as_object_mut() else {
        return Ok(());
    };
    if repoint_desktop_cwd(object, &batch.from, &batch.to_str()) {
        journal.save(&session.file)?;
        write_keeping_mode(&session.file, serde_json::to_string(&json)?.as_bytes())?;
    }
    Ok(())
}

fn apply_batch(rt: &Runtime, journal: &mut Journal, batch: &Batch) -> Result<()> {
    ensure_dir(journal, &batch.dest_dir)?;
    let rewriter = batch.rewriter();
    for (session, new) in &batch.ids {
        let extra = batch.source.dir.join(&session.id);
        let dest_transcript = batch.dest_dir.join(format!("{new}.jsonl"));
        let dest_extra = batch.dest_dir.join(new);
        if exists(&dest_transcript) || exists(&dest_extra) {
            bail!(
                "collision: {} already has a session {new}",
                ui::home_relative(&batch.to_str())
            );
        }
        match batch.mode {
            Mode::Copy => {
                journal.created(&dest_transcript)?;
                copy_rewritten(&session.transcript, &dest_transcript, &rewriter, true)?;
                if extra.is_dir() {
                    journal.created(&dest_extra)?;
                    copy_tree_rewritten(&extra, &dest_extra, &rewriter)?;
                }
                copy_keyed_by_id(rt, journal, &session.id, new)?;
            }
            Mode::Move => {
                rewrite_in_place(journal, &session.transcript, &rewriter)?;
                if extra.is_dir() {
                    for file in files_under(&extra)? {
                        rewrite_in_place(journal, &file, &rewriter)?;
                    }
                }
                journal.move_path(&session.transcript, &dest_transcript)?;
                if extra.is_dir() {
                    journal.move_path(&extra, &dest_extra)?;
                }
            }
        }
        for entry in batch
            .desktop
            .iter()
            .filter(|entry| entry.cli_session.as_deref() == Some(session.id.as_str()))
        {
            match batch.mode {
                Mode::Copy => clone_desktop(journal, batch, entry, new, &rewriter)?,
                Mode::Move => move_desktop(journal, batch, entry)?,
            }
        }
    }
    Ok(())
}

/// Copy what Claude Code keeps per session id outside the project folder: the file
/// checkpoints behind rewind, and the todo lists of older versions.
fn copy_keyed_by_id(rt: &Runtime, journal: &mut Journal, old: &str, new: &str) -> Result<()> {
    let checkpoints = rt.layout.config_dir.join("file-history");
    let source = checkpoints.join(old);
    if source.is_dir() {
        let dest = checkpoints.join(new);
        journal.created(&dest)?;
        copy_tree(&source, &dest)?;
    }
    let todos = rt.layout.config_dir.join("todos");
    if todos.is_dir() {
        let mut names: Vec<String> = fs::read_dir(&todos)?
            .flatten()
            .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
            .filter(|name| name.starts_with(old))
            .collect();
        names.sort();
        for name in names {
            let dest = todos.join(name.replace(old, new));
            journal.created(&dest)?;
            copy_tree(&todos.join(&name), &dest)?;
        }
    }
    Ok(())
}

/// The prompts of moved sessions now belong to the project they moved to.
fn repoint_moved_prompts(rt: &Runtime, journal: &mut Journal, batches: &[Batch]) -> Result<()> {
    let moved: HashMap<&str, (&str, String)> = batches
        .iter()
        .filter(|batch| batch.mode == Mode::Move)
        .flat_map(|batch| {
            batch.ids.iter().map(move |(session, _)| {
                (session.id.as_str(), (batch.from.as_str(), batch.to_str()))
            })
        })
        .collect();
    let history = rt.layout.prompt_history();
    if moved.is_empty() || !history.is_file() {
        return Ok(());
    }
    edit_lines(journal, &history, |json| {
        let session = json.get("sessionId").and_then(Value::as_str);
        let project = json.get("project").and_then(Value::as_str);
        match session.and_then(|id| moved.get(id)) {
            Some((from, to)) if project == Some(*from) => {
                json["project"] = Value::String(to.clone());
                LineEdit::Changed
            }
            _ => LineEdit::Keep,
        }
    })
}

fn verify_batch(batch: &Batch) -> Result<()> {
    for (session, new) in &batch.ids {
        let landed = batch.dest_dir.join(format!("{new}.jsonl"));
        if !landed.is_file() {
            bail!("verify failed: {} is missing", landed.display());
        }
        match batch.mode {
            Mode::Copy if !session.transcript.is_file() => bail!(
                "verify failed: {} is gone after a copy",
                session.transcript.display()
            ),
            Mode::Move if exists(&session.transcript) => bail!(
                "verify failed: {} is still there",
                session.transcript.display()
            ),
            _ => {}
        }
    }
    Ok(())
}

/// What a batch needs closed and how much room it takes.
struct Footprint {
    slugs: BTreeSet<String>,
    ids: HashSet<String>,
    /// Entries of the desktop app are rewritten, which the running app would undo.
    desktop_rewrites: bool,
    desktop_additions: bool,
}

fn footprint(batches: &[Batch]) -> Footprint {
    let mut found = Footprint {
        slugs: BTreeSet::new(),
        ids: HashSet::new(),
        desktop_rewrites: false,
        desktop_additions: false,
    };
    for batch in batches {
        found.slugs.insert(batch.source.slug.clone());
        found.slugs.insert(batch.dest_slug());
        found
            .ids
            .extend(batch.ids.iter().map(|(session, _)| session.id.clone()));
        match batch.mode {
            Mode::Move => found.desktop_rewrites |= !batch.desktop.is_empty(),
            Mode::Copy => found.desktop_additions |= !batch.desktop.is_empty(),
        }
    }
    found
}

/// The desktop app reads its chat list when it starts, so chats added while it runs only
/// show up after a restart.
fn note_desktop_restart(rt: &Runtime, report: &mut Report) -> Result<()> {
    let processes = rt.processes.processes()?;
    if !live::desktop_apps(&processes).is_empty() {
        report
            .warnings
            .push("restart the Claude desktop app to see the copied chats".to_string());
    }
    Ok(())
}

fn run_batches(rt: &Runtime, label: &str, batches: Vec<Batch>, report: &mut Report) -> Result<()> {
    for batch in &batches {
        report.applied.push(batch.describe());
    }
    if rt.dry_run || batches.is_empty() {
        return Ok(());
    }
    let mut needs = SpaceNeeds::default();
    for batch in &batches {
        match batch.mode {
            Mode::Copy => needs.add(&rt.layout.projects_dir(), batch.bytes(), "copied sessions"),
            Mode::Move => needs.add(
                &rt.layout.backup_root(),
                batch.bytes(),
                "backups of rewritten transcripts",
            ),
        }
    }
    needs.check()?;
    let found = footprint(&batches);
    require_idle(rt, &found.slugs, &found.ids, found.desktop_rewrites)?;
    in_journal(rt, label, |journal| {
        // Copies first: a session that is both moved and copied must still be at its source.
        for mode in [Mode::Copy, Mode::Move] {
            for batch in batches.iter().filter(|batch| batch.mode == mode) {
                apply_batch(rt, journal, batch)?;
                verify_batch(batch)?;
            }
        }
        repoint_moved_prompts(rt, journal, &batches)
    })?;
    if found.desktop_additions {
        note_desktop_restart(rt, report)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------
// cp

#[derive(Debug, Clone)]
pub struct CopyPlan {
    pub batch: Batch,
    /// Also copy the real folder (`--project`).
    pub folder: bool,
    /// Memory files to copy, with where each lands.
    memory: Vec<(PathBuf, PathBuf)>,
    /// The memory index of the source, when the destination has one to merge it into.
    memory_index: Option<(PathBuf, PathBuf)>,
    settings: bool,
}

/// Whether `cp` has anything to do for Claude Code with these arguments.
pub fn copy_matches(
    rt: &Runtime,
    pairs: &[(String, String)],
    replace: Option<(&str, &str)>,
    regex: bool,
) -> Result<bool> {
    ops::move_matches(rt, pairs, replace, regex)
}

pub fn plan_copy(
    rt: &Runtime,
    pairs: &[(String, String)],
    replace: Option<(&str, &str)>,
    regex: bool,
    folder: bool,
) -> Result<(Vec<CopyPlan>, Report)> {
    let projects = store::discover(rt)?;
    let desktop = desktop::sessions(&rt.layout)?;
    let settings = config::read(&rt.layout.config_file)?;
    let mut report = Report::default();
    let specs: Vec<(Project, PathBuf)> = match replace {
        Some((from, to)) => ops::replaced(rt, &projects, from, to, regex)?,
        None => pairs
            .iter()
            .map(|(from, to)| {
                let project = store::find(&projects, rt, from)
                    .with_context(|| format!("no Claude Code project matches {from}"))?;
                Ok((project.clone(), rt.resolve_path(to)))
            })
            .collect::<Result<_>>()?,
    };
    let mut plans = Vec::new();
    let mut dest_dirs = HashSet::new();
    for (source, to) in specs {
        let Some(from_path) = source.path.clone() else {
            report.warnings.push(format!(
                "skipped {}: chatkeep cannot tell which folder it belongs to",
                source.slug
            ));
            report.skipped.push(source.slug.clone());
            continue;
        };
        if to == from_path {
            bail!("collision: {} is the source project itself", to.display());
        }
        if folder {
            check_new_folder(&from_path, &to)?;
        } else if !to.exists() {
            report.warnings.push(format!(
                "skipped {}: destination missing {}",
                ui::home_relative(&from_path.display().to_string()),
                to.display()
            ));
            report.skipped.push(source.slug.clone());
            continue;
        }
        let batch = Batch::new(
            rt,
            &source,
            source.sessions.clone(),
            to.clone(),
            Mode::Copy,
            true,
            &desktop,
        )?;
        if !dest_dirs.insert(batch.dest_dir.clone()) {
            bail!(
                "two projects would be copied to the same folder: {}",
                to.display()
            );
        }
        let (memory, memory_index) = plan_memory(&source.dir, &batch.dest_dir)?;
        let from = batch.from.clone();
        let to_str = batch.to_str();
        plans.push(CopyPlan {
            settings: settings.as_ref().is_some_and(|json| {
                config::has_project(json, &from) && !config::has_project(json, &to_str)
            }),
            batch,
            folder,
            memory,
            memory_index,
        });
    }
    Ok((plans, report))
}

type MemoryPlan = (Vec<(PathBuf, PathBuf)>, Option<(PathBuf, PathBuf)>);

/// Which memory files of `from` go to `to`. A file both projects hold with the same bytes is
/// left alone, the index is merged line by line, and any other clash stops the copy.
fn plan_memory(from: &Path, to: &Path) -> Result<MemoryPlan> {
    let source = from.join("memory");
    let dest = to.join("memory");
    let mut copies = Vec::new();
    let mut index = None;
    let mut conflicts = Vec::new();
    if !source.is_dir() {
        return Ok((copies, index));
    }
    for file in files_under(&source)? {
        let target = dest.join(file.strip_prefix(&source)?);
        if !exists(&target) {
            copies.push((file, target));
        } else if is_memory_index(&file) {
            index = Some((file, target));
        } else if target.is_dir() || fs::read(&file)? != fs::read(&target)? {
            conflicts.push(target);
        }
    }
    if !conflicts.is_empty() {
        bail!(
            "cannot copy: the destination project already has different memory files with these names:\n- {}",
            conflicts
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n- ")
        );
    }
    Ok((copies, index))
}

fn describe_copy(rt: &Runtime, plan: &CopyPlan) -> Vec<String> {
    let from = ui::home_relative(&plan.batch.from);
    let to = ui::home_relative(&plan.batch.to_str());
    let mut lines = vec![format!(
        "copy project {from} ({}) -> {to}",
        ui::plural(plan.batch.ids.len(), "session", "sessions")
    )];
    if plan.folder {
        lines.push(format!("copy folder {from} -> {to}"));
    }
    let memory = plan.memory.len() + usize::from(plan.memory_index.is_some());
    if memory > 0 {
        lines.push(format!(
            "copy {}",
            ui::plural(memory, "memory file", "memory files")
        ));
    }
    if plan.settings {
        lines.push(format!(
            "copy settings {}",
            ui::home_relative(&rt.layout.config_file.display().to_string())
        ));
    }
    if !plan.batch.desktop.is_empty() {
        lines.push(format!(
            "add {}",
            ui::plural(
                plan.batch.desktop.len(),
                "desktop app session",
                "desktop app sessions"
            )
        ));
    }
    lines
}

pub fn execute_copy(rt: &Runtime, plans: Vec<CopyPlan>, mut report: Report) -> Result<Report> {
    for plan in &plans {
        report.applied.extend(describe_copy(rt, plan));
    }
    if rt.dry_run || plans.is_empty() {
        return Ok(report);
    }
    let mut needs = SpaceNeeds::default();
    for plan in &plans {
        needs.add(
            &rt.layout.projects_dir(),
            plan.batch.source.size,
            "project copy",
        );
        if plan.folder {
            needs.add(
                &plan.batch.to,
                dir_size(Path::new(&plan.batch.from))?,
                "project folder copy",
            );
        }
    }
    needs.check()?;
    let batches: Vec<Batch> = plans.iter().map(|plan| plan.batch.clone()).collect();
    let found = footprint(&batches);
    require_idle(rt, &found.slugs, &found.ids, false)?;
    in_journal(rt, "claude-cp", |journal| {
        for plan in &plans {
            if plan.folder {
                journal.created(&plan.batch.to)?;
                copy_tree(Path::new(&plan.batch.from), &plan.batch.to)?;
            }
            apply_batch(rt, journal, &plan.batch)?;
            let rewriter = plan.batch.rewriter();
            for (source, dest) in &plan.memory {
                journal.created(dest)?;
                copy_rewritten(source, dest, &rewriter, false)?;
            }
            if let Some((source, dest)) = &plan.memory_index {
                let theirs = fs::read_to_string(source)?;
                add_memory_lines(journal, &rewriter.rewrite(&theirs), dest)?;
            }
            if plan.settings
                && let Some(mut json) = config::read(&rt.layout.config_file)?
                && config::copy_project(&mut json, &plan.batch.from, &plan.batch.to_str())
            {
                journal.save(&rt.layout.config_file)?;
                write_keeping_mode(
                    &rt.layout.config_file,
                    serde_json::to_string_pretty(&json)?.as_bytes(),
                )?;
            }
        }
        for plan in &plans {
            verify_batch(&plan.batch)?;
            if plan.folder && !plan.batch.to.is_dir() {
                bail!("verify failed: {} is missing", plan.batch.to.display());
            }
        }
        Ok(())
    })?;
    if found.desktop_additions {
        note_desktop_restart(rt, &mut report)?;
    }
    Ok(report)
}

// ---------------------------------------------------------------------------------------
// split

/// Every local path a session worked in or on: the folder of each line, and the files its
/// tools read or wrote.
pub fn touched_paths(transcript: &Path) -> Result<Vec<PathBuf>> {
    let raw = fs::read_to_string(transcript)
        .with_context(|| format!("failed to read {}", transcript.display()))?;
    let mut seen = HashSet::new();
    let mut paths = Vec::new();
    let mut add = |path: &str| {
        if !path.is_empty() && seen.insert(path.to_string()) {
            paths.push(PathBuf::from(path));
        }
    };
    for line in raw.lines() {
        let Ok(json) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some(cwd) = json.get("cwd").and_then(Value::as_str) {
            add(cwd);
        }
        if let Some(path) = json
            .pointer("/toolUseResult/filePath")
            .and_then(Value::as_str)
        {
            add(path);
        }
        let blocks = json
            .pointer("/message/content")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            for key in ["file_path", "path", "notebook_path"] {
                if let Some(path) = block
                    .get("input")
                    .and_then(|input| input.get(key))
                    .and_then(Value::as_str)
                {
                    add(path);
                }
            }
        }
    }
    Ok(paths)
}

fn longest_target(path: &Path, targets: &[PathBuf]) -> Option<PathBuf> {
    targets
        .iter()
        .filter(|target| path.starts_with(target))
        .max_by_key(|target| target.as_os_str().len())
        .cloned()
}

/// Which target each session of `source` belongs to, judged by the paths it touched.
pub fn suggest_split(
    rt: &Runtime,
    source: &Project,
    targets: &[PathBuf],
) -> Result<SplitSuggestion> {
    let roots: Vec<PathBuf> = targets.iter().map(|path| rt.resolve_path(path)).collect();
    let mut assigned: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut unassigned = Vec::new();
    let mut titles = HashMap::new();
    let spinner = ui::spinner("Matching chats to targets", rt.quiet);
    for (done, session) in source.sessions.iter().enumerate() {
        spinner.set_message(&format!(
            "Matching chats to targets ({}/{})",
            done + 1,
            source.sessions.len()
        ));
        let mut hits = Vec::new();
        for path in touched_paths(&session.transcript)? {
            if let Some(target) = longest_target(&path, &roots)
                && !hits.contains(&target)
            {
                hits.push(target);
            }
        }
        if hits.is_empty() {
            unassigned.push(session.id.clone());
        } else {
            assigned.insert(session.id.clone(), hits);
        }
        titles.insert(session.id.clone(), store::title(source, session));
    }
    drop(spinner);
    Ok(SplitSuggestion {
        assigned,
        unassigned,
        titles,
    })
}

pub fn split(
    rt: &Runtime,
    source: &Project,
    targets: &[PathBuf],
    assignments: &BTreeMap<String, Vec<PathBuf>>,
    move_chats: bool,
) -> Result<Report> {
    let desktop = desktop::sessions(&rt.layout)?;
    let mut resolved: Vec<PathBuf> = Vec::new();
    for target in targets {
        let target = rt.resolve_path(target);
        if !target.is_dir() {
            bail!("target does not exist: {}", target.display());
        }
        if source.path.as_deref() == Some(target.as_path()) {
            bail!("split target {} is the source project", target.display());
        }
        if resolved.contains(&target) {
            bail!("split target {} is listed twice", target.display());
        }
        resolved.push(target);
    }
    let mut copies: Vec<Vec<Session>> = vec![Vec::new(); resolved.len()];
    let mut moves: Vec<Vec<Session>> = vec![Vec::new(); resolved.len()];
    for (id, dests) in assignments {
        let session = source
            .session(id)
            .with_context(|| format!("chat {id} does not belong to {}", project_label(source)))?;
        let mut indexes: Vec<usize> = Vec::new();
        for dest in dests {
            let dest = rt.resolve_path(dest);
            let index = resolved
                .iter()
                .position(|path| path == &dest)
                .with_context(|| {
                    format!("chat {id} is assigned to unknown target {}", dest.display())
                })?;
            if !indexes.contains(&index) {
                indexes.push(index);
            }
        }
        let Some((first, rest)) = indexes.split_first() else {
            continue;
        };
        if move_chats {
            moves[*first].push(session.clone());
            for index in rest {
                copies[*index].push(session.clone());
            }
        } else {
            for index in indexes {
                copies[index].push(session.clone());
            }
        }
    }
    let mut batches = Vec::new();
    for (mode, groups) in [(Mode::Copy, copies), (Mode::Move, moves)] {
        for (target, sessions) in resolved.iter().zip(groups) {
            if !sessions.is_empty() {
                batches.push(Batch::new(
                    rt,
                    source,
                    sessions,
                    target.clone(),
                    mode,
                    false,
                    &desktop,
                )?);
            }
        }
    }
    let mut report = Report::default();
    run_batches(rt, "claude-split", batches, &mut report)?;
    Ok(report)
}

// ---------------------------------------------------------------------------------------
// combine

/// Whether `spec` names something `combine` can take chats from.
pub fn is_source(rt: &Runtime, spec: &str) -> Result<bool> {
    let projects = store::discover(rt)?;
    Ok(
        store::find(&projects, rt, spec).is_some()
            || store::find_session(&projects, spec).is_some(),
    )
}

/// The folder `combine` sends chats to: a project chatkeep knows, or any existing folder.
fn combine_target(rt: &Runtime, projects: &[Project], spec: &str) -> Result<PathBuf> {
    if let Some(project) = store::find(projects, rt, spec) {
        return project.path.clone().with_context(|| {
            format!(
                "chatkeep cannot tell which folder {} belongs to",
                project.slug
            )
        });
    }
    let path = rt.resolve_path(spec);
    if !path.is_dir() {
        bail!("target does not exist: {}", path.display());
    }
    Ok(path)
}

pub fn combine(rt: &Runtime, target: &str, sources: &[String], move_chats: bool) -> Result<Report> {
    let projects = store::discover(rt)?;
    let desktop = desktop::sessions(&rt.layout)?;
    let to = combine_target(rt, &projects, target)?;
    let dest_dir = rt
        .layout
        .projects_dir()
        .join(project_slug(&to.to_string_lossy()));
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut groups: Vec<(Project, Vec<Session>)> = Vec::new();
    for spec in sources {
        let (project, sessions) = if let Some(project) = store::find(&projects, rt, spec) {
            (project, project.sessions.clone())
        } else if let Some((project, session)) = store::find_session(&projects, spec) {
            (project, vec![session.clone()])
        } else {
            bail!("no Claude Code project or session matches {spec}");
        };
        if project.dir == dest_dir {
            report.warnings.push(if project.session(spec).is_some() {
                format!("skipped {spec}: already in the combine target")
            } else {
                format!("skipped {spec}: it is the combine target")
            });
            report.skipped.push(spec.clone());
            continue;
        }
        let kept: Vec<Session> = sessions
            .into_iter()
            .filter(|session| seen.insert(session.id.clone()))
            .collect();
        if kept.is_empty() {
            continue;
        }
        match groups
            .iter_mut()
            .find(|(existing, _)| existing.dir == project.dir)
        {
            Some((_, sessions)) => sessions.extend(kept),
            None => groups.push((project.clone(), kept)),
        }
    }
    let mode = if move_chats { Mode::Move } else { Mode::Copy };
    let mut batches = Vec::new();
    for (project, sessions) in groups {
        batches.push(Batch::new(
            rt,
            &project,
            sessions,
            to.clone(),
            mode,
            false,
            &desktop,
        )?);
    }
    run_batches(rt, "claude-combine", batches, &mut report)?;
    Ok(report)
}
