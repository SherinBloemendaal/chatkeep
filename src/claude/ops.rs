//! `mv` and `rm` for Claude Code projects and sessions.
//!
//! Every write goes through a [`Journal`]: files are copied aside before they change, moves
//! are recorded, and a failure anywhere puts everything back.

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::desktop::{self, DesktopSession};
use super::live;
use super::slug::project_slug;
use super::store::{self, Project, Session};
use super::{Runtime, config};
use crate::cursor::rewrite::{Boundary, Replacement, Rewriter, path_replacements};
use crate::cursor::uri::Platform;
use crate::engine::Report;
use crate::engine::fsops::{SpaceNeeds, exists, write_atomic};
use crate::engine::journal::Journal;
use crate::ui;

/// The index of a memory folder: one line per memory, merged line by line.
const MEMORY_INDEX: &str = "MEMORY.md";

#[derive(Debug, Clone)]
pub struct MovePlan {
    pub source: Project,
    pub from: String,
    pub to: PathBuf,
    pub dest_dir: PathBuf,
    /// The destination already has a project folder; the sessions join it.
    pub merge: bool,
    /// Also move the real folder (`--project`).
    pub folder: bool,
    /// Files in the source project that mention the old path.
    rewrites: Vec<PathBuf>,
    /// Source files identical to the destination's copy, dropped instead of moved.
    duplicates: Vec<PathBuf>,
    desktop: Vec<DesktopSession>,
    prompts: usize,
    settings: bool,
}

impl MovePlan {
    fn to_str(&self) -> String {
        self.to.to_string_lossy().into_owned()
    }

    fn rewriter(&self) -> Rewriter {
        let to = self.to_str();
        let mut entries: Vec<(Replacement, Boundary)> =
            path_replacements(Platform::current(), &self.from, &to)
                .into_iter()
                .map(|item| (item, Boundary::Path))
                .collect();
        // References to the project folder itself, e.g. memory files under ~/.claude/projects.
        entries.push((
            Replacement::new(&self.source.slug, project_slug(&to)),
            Boundary::Token,
        ));
        Rewriter::build(entries)
    }
}

/// Whether `mv` has anything to do for Claude Code with these arguments.
pub fn move_matches(
    rt: &Runtime,
    pairs: &[(String, String)],
    replace: Option<(&str, &str)>,
    regex: bool,
) -> Result<bool> {
    let projects = store::discover(rt)?;
    if let Some((from, to)) = replace {
        return Ok(!replaced(rt, &projects, from, to, regex)?.is_empty());
    }
    Ok(pairs
        .iter()
        .all(|(from, _)| store::find(&projects, rt, from).is_some()))
}

pub fn plan_move(
    rt: &Runtime,
    pairs: &[(String, String)],
    replace: Option<(&str, &str)>,
    regex: bool,
    folder: bool,
) -> Result<(Vec<MovePlan>, Report)> {
    let projects = store::discover(rt)?;
    let desktop = desktop::sessions(&rt.layout)?;
    let config = config::read(&rt.layout.config_file)?;
    let prompts = prompt_projects(&rt.layout.prompt_history())?;
    let mut report = Report::default();
    let specs: Vec<(Project, PathBuf)> = match replace {
        Some((from, to)) => replaced(rt, &projects, from, to, regex)?,
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
            continue;
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
        let from = from_path.to_string_lossy().into_owned();
        let to_str = to.to_string_lossy().into_owned();
        let dest_dir = rt.layout.projects_dir().join(project_slug(&to_str));
        if !dest_dirs.insert(dest_dir.clone()) {
            bail!(
                "two projects would move to the same folder: {}",
                to.display()
            );
        }
        let merge = dest_dir != source.dir && dest_dir.exists();
        let mut plan = MovePlan {
            from: from.clone(),
            to,
            merge,
            folder,
            rewrites: Vec::new(),
            duplicates: Vec::new(),
            desktop: desktop
                .iter()
                .filter(|session| session.cwd.as_deref() == Some(from.as_str()))
                .cloned()
                .collect(),
            prompts: prompts.iter().filter(|project| **project == from).count(),
            settings: config
                .as_ref()
                .is_some_and(|json| config::has_project(json, &from)),
            dest_dir,
            source,
        };
        if plan.merge {
            plan.duplicates = merge_conflicts(&plan.source.dir, &plan.dest_dir)?;
        }
        let rewriter = plan.rewriter();
        plan.rewrites = files_mentioning(&plan.source.dir, &rewriter)?
            .into_iter()
            .filter(|path| !plan.duplicates.contains(path))
            .collect();
        if let Some(json) = &config
            && plan.settings
            && config::has_project(json, &to_str)
        {
            report.warnings.push(format!(
                "{} already has Claude Code settings; they are kept and those of {} are dropped",
                ui::home_relative(&to_str),
                ui::home_relative(&from)
            ));
        }
        plans.push(plan);
    }
    Ok((plans, report))
}

/// Projects whose path `--replace FROM TO` changes, with the new path.
pub(super) fn replaced(
    rt: &Runtime,
    projects: &[Project],
    from: &str,
    to: &str,
    regex: bool,
) -> Result<Vec<(Project, PathBuf)>> {
    let pattern = if regex {
        Some(Regex::new(from).with_context(|| format!("invalid regex: {from}"))?)
    } else {
        None
    };
    let mut found = Vec::new();
    for project in projects {
        let Some(path) = &project.path else {
            continue;
        };
        let path_str = path.to_string_lossy();
        let updated = match &pattern {
            Some(pattern) if pattern.is_match(&path_str) => {
                pattern.replace(&path_str, to).into_owned()
            }
            Some(_) => continue,
            None => match path_str.find(from) {
                Some(at) => format!("{}{to}{}", &path_str[..at], &path_str[at + from.len()..]),
                None => continue,
            },
        };
        let dest = rt.resolve_path(&updated);
        if &dest != path {
            found.push((project.clone(), dest));
        }
    }
    Ok(found)
}

pub(super) fn check_new_folder(from: &Path, to: &Path) -> Result<()> {
    if !from.is_dir() {
        bail!(
            "--project needs the real folder to exist: {}",
            from.display()
        );
    }
    if exists(to) {
        bail!("collision: destination already exists: {}", to.display());
    }
    if let Some(parent) = to.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        bail!(
            "--project refused: destination parent is missing: {}",
            parent.display()
        );
    }
    Ok(())
}

/// Source files that already exist in the destination with the same bytes. Any other file
/// both folders hold stops the merge before it starts, except the memory index, which is
/// merged line by line.
fn merge_conflicts(from: &Path, to: &Path) -> Result<Vec<PathBuf>> {
    let mut duplicates = Vec::new();
    let mut conflicts = Vec::new();
    walk_pairs(from, to, &mut |source, dest| {
        if is_memory_index(source) {
            return Ok(());
        }
        if fs::read(source)? == fs::read(dest)? {
            duplicates.push(source.to_path_buf());
        } else {
            conflicts.push(dest.to_path_buf());
        }
        Ok(())
    })?;
    if !conflicts.is_empty() {
        bail!(
            "cannot merge: the destination project already has different files with these names:\n- {}",
            conflicts
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n- ")
        );
    }
    Ok(duplicates)
}

/// Calls `found` for every file that exists under both folders at the same relative path.
fn walk_pairs(
    from: &Path,
    to: &Path,
    found: &mut dyn FnMut(&Path, &Path) -> Result<()>,
) -> Result<()> {
    for entry in fs::read_dir(from).with_context(|| format!("failed to read {}", from.display()))? {
        let entry = entry?;
        let source = entry.path();
        let dest = to.join(entry.file_name());
        if !exists(&dest) {
            continue;
        }
        match (entry.file_type()?.is_dir(), dest.is_dir()) {
            (true, true) => walk_pairs(&source, &dest, found)?,
            (false, false) => found(&source, &dest)?,
            _ => bail!(
                "cannot merge: {} is a folder in one project and a file in the other",
                dest.display()
            ),
        }
    }
    Ok(())
}

pub(super) fn is_memory_index(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == MEMORY_INDEX)
        && path
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "memory")
}

/// Text files under `dir` the rewriter would change. Binary files are left alone.
fn files_mentioning(dir: &Path, rewriter: &Rewriter) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    if rewriter.is_empty() {
        return Ok(found);
    }
    for path in files_under(dir)? {
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        if let Ok(text) = std::str::from_utf8(&bytes)
            && rewriter.contains(text)
        {
            found.push(path);
        }
    }
    Ok(found)
}

pub(super) fn files_under(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            found.extend(files_under(&entry.path())?);
        } else if kind.is_file() {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// The `project` of every prompt in the prompt history, one per line.
pub(super) fn prompt_projects(path: &Path) -> Result<Vec<String>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|json| {
            json.get("project")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect())
}

/// Lines a dry run reports, and a real run reports once it is done.
/// Result lines in the shared `verb subject -> target` form the result table splits up.
fn describe_move(rt: &Runtime, plan: &MovePlan) -> Vec<String> {
    let from = ui::home_relative(&plan.from);
    let to = ui::home_relative(&plan.to_str());
    let verb = if plan.merge {
        "merge project"
    } else {
        "move project"
    };
    let mut lines = vec![format!(
        "{verb} {from} ({}) -> {to}",
        ui::plural(plan.source.sessions.len(), "session", "sessions")
    )];
    if plan.folder {
        lines.push(format!("move folder {from} -> {to}"));
    }
    if !plan.rewrites.is_empty() {
        lines.push(format!(
            "rewrite the path in {}",
            ui::plural(plan.rewrites.len(), "file", "files")
        ));
    }
    if plan.settings {
        lines.push(format!(
            "move settings {}",
            ui::home_relative(&rt.layout.config_file.display().to_string())
        ));
    }
    if plan.prompts > 0 {
        lines.push(format!(
            "repoint {}",
            ui::plural(plan.prompts, "prompt", "prompts")
        ));
    }
    if !plan.desktop.is_empty() {
        lines.push(format!(
            "repoint {}",
            ui::plural(
                plan.desktop.len(),
                "desktop app session",
                "desktop app sessions"
            )
        ));
    }
    lines
}

pub fn execute_move(rt: &Runtime, plans: Vec<MovePlan>, mut report: Report) -> Result<Report> {
    if rt.dry_run {
        for plan in &plans {
            report.applied.extend(describe_move(rt, plan));
        }
        return Ok(report);
    }
    if plans.is_empty() {
        return Ok(report);
    }
    let mut slugs = BTreeSet::new();
    let mut session_ids = HashSet::new();
    let mut desktop = false;
    let mut needs = SpaceNeeds::default();
    for plan in &plans {
        slugs.insert(plan.source.slug.clone());
        slugs.insert(project_slug(&plan.to_str()));
        session_ids.extend(
            plan.source
                .sessions
                .iter()
                .map(|session| session.id.clone()),
        );
        desktop |= !plan.desktop.is_empty();
        let bytes: u64 = plan
            .rewrites
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|meta| meta.len())
            .sum();
        needs.add(
            &rt.layout.backup_root(),
            bytes,
            "backups of rewritten transcripts",
        );
    }
    needs.check()?;
    require_idle(rt, &slugs, &session_ids, desktop)?;
    in_journal(rt, "claude-mv", |journal| {
        let history_from: Vec<(String, String)> = plans
            .iter()
            .filter(|plan| plan.prompts > 0)
            .map(|plan| (plan.from.clone(), plan.to_str()))
            .collect();
        for plan in &plans {
            apply_move(rt, journal, plan)?;
            report.applied.extend(describe_move(rt, plan));
        }
        repoint_prompts(journal, &rt.layout.prompt_history(), &history_from)?;
        for plan in &plans {
            verify_move(plan)?;
        }
        Ok(())
    })?;
    Ok(report)
}

fn apply_move(rt: &Runtime, journal: &mut Journal, plan: &MovePlan) -> Result<()> {
    if plan.folder {
        journal.move_path(Path::new(&plan.from), &plan.to)?;
    }
    let rewriter = plan.rewriter();
    for path in plan
        .rewrites
        .iter()
        .filter(|path| !plan.duplicates.contains(path))
    {
        rewrite_file(journal, path, &rewriter)?;
    }
    if plan.dest_dir != plan.source.dir {
        for duplicate in &plan.duplicates {
            journal.stash(duplicate)?;
        }
        merge_into(journal, &plan.source.dir, &plan.dest_dir)?;
    }
    if plan.settings
        && let Some(mut json) = config::read(&rt.layout.config_file)?
    {
        config::move_project(&mut json, &plan.from, &plan.to_str());
        journal.save(&rt.layout.config_file)?;
        write_keeping_mode(
            &rt.layout.config_file,
            serde_json::to_string_pretty(&json)?.as_bytes(),
        )?;
    }
    for session in &plan.desktop {
        rewrite_file(journal, &session.file, &rewriter)?;
    }
    Ok(())
}

/// Move everything in `from` into `to`. Folders both hold are merged entry by entry and the
/// memory index gains the lines it lacks; [`merge_conflicts`] already ruled out other clashes.
fn merge_into(journal: &mut Journal, from: &Path, to: &Path) -> Result<()> {
    if !exists(to) {
        return journal.move_path(from, to);
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(from)
        .with_context(|| format!("failed to read {}", from.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    for source in entries {
        let name = source.file_name().context("entry has no name")?;
        let dest = to.join(name);
        if !exists(&dest) {
            journal.move_path(&source, &dest)?;
        } else if source.is_dir() {
            merge_into(journal, &source, &dest)?;
        } else if is_memory_index(&source) {
            merge_memory_index(journal, &source, &dest)?;
        } else {
            bail!("cannot merge: {} already exists", dest.display());
        }
    }
    journal.stash(from)
}

fn merge_memory_index(journal: &mut Journal, source: &Path, dest: &Path) -> Result<()> {
    add_memory_lines(journal, &fs::read_to_string(source)?, dest)?;
    journal.stash(source)
}

/// Append to the memory index at `dest` every line of `theirs` it lacks.
pub(super) fn add_memory_lines(journal: &mut Journal, theirs: &str, dest: &Path) -> Result<()> {
    let ours = fs::read_to_string(dest)?;
    let present: HashSet<&str> = ours.lines().collect();
    let missing: Vec<&str> = theirs
        .lines()
        .filter(|line| !line.trim().is_empty() && !present.contains(line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut merged = ours.clone();
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    for line in missing {
        merged.push_str(line);
        merged.push('\n');
    }
    journal.save(dest)?;
    write_keeping_mode(dest, merged.as_bytes())
}

fn rewrite_file(journal: &mut Journal, path: &Path, rewriter: &Rewriter) -> Result<()> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let updated = rewriter.rewrite(&text);
    if updated == text {
        return Ok(());
    }
    journal.save(path)?;
    write_keeping_mode(path, updated.as_bytes())
}

/// The prompt history with every `project` in `moves` repointed. Other lines keep their bytes.
fn repoint_prompts(journal: &mut Journal, path: &Path, moves: &[(String, String)]) -> Result<()> {
    if moves.is_empty() || !path.is_file() {
        return Ok(());
    }
    edit_lines(journal, path, |json| {
        let Some(project) = json.get("project").and_then(Value::as_str) else {
            return LineEdit::Keep;
        };
        match moves.iter().find(|(from, _)| from == project) {
            Some((_, to)) => {
                json["project"] = Value::String(to.clone());
                LineEdit::Changed
            }
            None => LineEdit::Keep,
        }
    })
}

pub(super) enum LineEdit {
    Keep,
    Changed,
    Drop,
}

pub(super) fn edit_lines(
    journal: &mut Journal,
    path: &Path,
    mut edit: impl FnMut(&mut Value) -> LineEdit,
) -> Result<()> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut out = String::with_capacity(raw.len());
    let mut changed = false;
    for line in raw.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        let Ok(mut json) = serde_json::from_str::<Value>(body) else {
            out.push_str(line);
            continue;
        };
        match edit(&mut json) {
            LineEdit::Keep => out.push_str(line),
            LineEdit::Changed => {
                changed = true;
                out.push_str(&serde_json::to_string(&json)?);
                out.push_str(&line[body.len()..]);
            }
            LineEdit::Drop => changed = true,
        }
    }
    if changed {
        journal.save(path)?;
        write_keeping_mode(path, out.as_bytes())?;
    }
    Ok(())
}

fn verify_move(plan: &MovePlan) -> Result<()> {
    for session in &plan.source.sessions {
        let moved = plan.dest_dir.join(format!("{}.jsonl", session.id));
        if !moved.is_file() {
            bail!("verify failed: {} is missing", moved.display());
        }
    }
    if plan.dest_dir != plan.source.dir && exists(&plan.source.dir) {
        bail!(
            "verify failed: {} is still there",
            plan.source.dir.display()
        );
    }
    if plan.folder && !plan.to.is_dir() {
        bail!("verify failed: {} is missing", plan.to.display());
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub enum RemoveTarget {
    Project(Project),
    Session {
        project: Project,
        session: Box<Session>,
    },
}

impl RemoveTarget {
    pub fn label(&self) -> String {
        match self {
            Self::Project(project) => project_label(project),
            Self::Session { session, .. } => format!("session {}", session.id),
        }
    }

    pub fn location(&self) -> String {
        match self {
            Self::Project(project) => ui::plural(project.sessions.len(), "session", "sessions"),
            Self::Session { project, .. } => project_label(project),
        }
    }
}

pub(super) fn project_label(project: &Project) -> String {
    project
        .path
        .as_ref()
        .map(|path| ui::home_relative(&path.display().to_string()))
        .unwrap_or_else(|| project.slug.clone())
}

/// A project path or folder name, or a session id.
pub fn find_target(rt: &Runtime, spec: &str) -> Result<Option<RemoveTarget>> {
    let projects = store::discover(rt)?;
    if let Some(project) = store::find(&projects, rt, spec) {
        return Ok(Some(RemoveTarget::Project(project.clone())));
    }
    Ok(
        store::find_session(&projects, spec).map(|(project, session)| RemoveTarget::Session {
            project: project.clone(),
            session: Box::new(session.clone()),
        }),
    )
}

pub fn remove(rt: &Runtime, targets: &[RemoveTarget]) -> Result<Report> {
    let mut report = Report::default();
    let desktop_sessions = desktop::sessions(&rt.layout)?;
    let mut slugs = BTreeSet::new();
    let mut ids = HashSet::new();
    let mut desktop_files = Vec::new();
    let mut paths = Vec::new();
    for target in targets {
        let sessions: Vec<&Session> = match target {
            RemoveTarget::Project(project) => {
                slugs.insert(project.slug.clone());
                if let Some(path) = &project.path {
                    let path = path.to_string_lossy().into_owned();
                    desktop_files.extend(
                        desktop_sessions
                            .iter()
                            .filter(|session| session.cwd.as_deref() == Some(path.as_str()))
                            .map(|session| session.file.clone()),
                    );
                    paths.push(path);
                }
                project.sessions.iter().collect()
            }
            RemoveTarget::Session { session, .. } => vec![session],
        };
        for session in sessions {
            ids.insert(session.id.clone());
        }
    }
    desktop_files.extend(
        desktop_sessions
            .iter()
            .filter(|session| {
                session
                    .cli_session
                    .as_ref()
                    .is_some_and(|id| ids.contains(id))
            })
            .map(|session| session.file.clone()),
    );
    desktop_files.sort();
    desktop_files.dedup();
    for target in targets {
        report.applied.push(match target {
            RemoveTarget::Project(project) => format!(
                "remove project {} ({}, {})",
                project_label(project),
                ui::plural(project.sessions.len(), "session", "sessions"),
                ui::plural(project.memory_files, "memory file", "memory files")
            ),
            RemoveTarget::Session { project, session } => {
                format!(
                    "remove session {} -> {}",
                    session.id,
                    project_label(project)
                )
            }
        });
    }
    if !desktop_files.is_empty() {
        report.applied.push(format!(
            "remove {}",
            ui::plural(
                desktop_files.len(),
                "desktop app session",
                "desktop app sessions"
            )
        ));
    }
    if rt.dry_run {
        return Ok(report);
    }
    require_idle(rt, &slugs, &ids, !desktop_files.is_empty())?;
    in_journal(rt, "claude-rm", |journal| {
        for target in targets {
            match target {
                RemoveTarget::Project(project) => journal.stash(&project.dir)?,
                RemoveTarget::Session { project, session } => {
                    journal.stash(&session.transcript)?;
                    journal.stash(&project.dir.join(&session.id))?;
                }
            }
        }
        let mut sorted: Vec<&String> = ids.iter().collect();
        sorted.sort();
        for id in sorted {
            for extra in rt.layout.session_extras(id) {
                journal.stash(&extra)?;
            }
        }
        for file in &desktop_files {
            journal.stash(file)?;
        }
        if !paths.is_empty()
            && let Some(mut json) = config::read(&rt.layout.config_file)?
        {
            let mut changed = false;
            for path in &paths {
                changed |= config::remove_project(&mut json, path);
            }
            if changed {
                journal.save(&rt.layout.config_file)?;
                write_keeping_mode(
                    &rt.layout.config_file,
                    serde_json::to_string_pretty(&json)?.as_bytes(),
                )?;
            }
        }
        let history = rt.layout.prompt_history();
        if history.is_file() {
            edit_lines(journal, &history, |json| {
                let project = json.get("project").and_then(Value::as_str);
                let session = json.get("sessionId").and_then(Value::as_str);
                if project.is_some_and(|project| paths.iter().any(|path| path == project))
                    || session.is_some_and(|session| ids.contains(session))
                {
                    LineEdit::Drop
                } else {
                    LineEdit::Keep
                }
            })?;
        }
        Ok(())
    })?;
    Ok(report)
}

/// Refuse while a Claude Code session runs in one of `slugs` or is one of `ids`, or while the
/// desktop app runs and would keep its own copy of session files this command changes.
pub(super) fn require_idle(
    rt: &Runtime,
    slugs: &BTreeSet<String>,
    ids: &HashSet<String>,
    desktop: bool,
) -> Result<()> {
    let processes = rt.processes.processes().map_err(|err| {
        ui::hinted(
            format!("Cannot tell whether Claude Code is running: {err:#}"),
            "chatkeep only writes after it has confirmed that no Claude Code session uses this project.",
        )
    })?;
    let busy: Vec<String> = live::sessions(&rt.layout, &processes)?
        .into_iter()
        .filter(|session| {
            session
                .cwd
                .as_deref()
                .is_some_and(|cwd| slugs.contains(&project_slug(cwd)))
                || session
                    .session_id
                    .as_ref()
                    .is_some_and(|id| ids.contains(id))
        })
        .map(|session| session.label())
        .collect();
    if !busy.is_empty() {
        return Err(anyhow::Error::new(super::Busy::sessions(busy)));
    }
    let apps = live::desktop_apps(&processes);
    if desktop && !apps.is_empty() {
        return Err(anyhow::Error::new(super::Busy::desktop_app(&apps)));
    }
    Ok(())
}

/// Run `body` with a journal; undo every recorded step if it fails.
pub(super) fn in_journal(
    rt: &Runtime,
    label: &str,
    body: impl FnOnce(&mut Journal) -> Result<()>,
) -> Result<()> {
    let mut journal = Journal::create(&rt.layout.backup_root(), label)?;
    match body(&mut journal) {
        Ok(()) => {
            let dir = journal.dir().to_path_buf();
            if let Err(err) = journal.discard() {
                ui::warn(&format!(
                    "backup at {} could not be removed: {err:#}",
                    dir.display()
                ));
            }
            Ok(())
        }
        Err(cause) => {
            let failures = journal.undo_files();
            let dir = journal.dir().to_path_buf();
            let note = if failures.is_empty() {
                match journal.discard() {
                    Ok(()) => "All changes were rolled back.".to_string(),
                    Err(err) => format!(
                        "All changes were rolled back, but the backup at {} could not be removed: {err:#}",
                        dir.display()
                    ),
                }
            } else {
                format!(
                    "Rollback incomplete. Backup kept at {}:\n- {}",
                    dir.display(),
                    failures.join("\n- ")
                )
            };
            Err(anyhow::anyhow!("{cause:#}\n{note}"))
        }
    }
}

/// Create `dir` and record the topmost folder that had to be made, so undo removes it all.
pub(super) fn ensure_dir(journal: &mut Journal, dir: &Path) -> Result<()> {
    if exists(dir) {
        return Ok(());
    }
    let mut top = dir;
    while let Some(parent) = top.parent() {
        if parent.as_os_str().is_empty() || exists(parent) {
            break;
        }
        top = parent;
    }
    journal.created(top)?;
    fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))
}

/// Atomic write that keeps the file's permission bits: Claude Code keeps some files private.
pub(super) fn write_keeping_mode(path: &Path, bytes: &[u8]) -> Result<()> {
    let permissions = fs::metadata(path).ok().map(|meta| meta.permissions());
    write_atomic(path, bytes)?;
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to restore permissions of {}", path.display()))?;
    }
    Ok(())
}
