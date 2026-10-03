//! Claude Code projects on disk: one folder per path in `~/.claude/projects`, holding one
//! `<session>.jsonl` transcript per session, an optional `<session>/` folder with subagent
//! transcripts and tool output, and an optional `memory/` folder.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::slug::project_slug;
use super::{Runtime, config, desktop, index};
use crate::engine::fsops::dir_size;

/// How many lines of a transcript are read to find the folder a session started in.
const CWD_SCAN_LINES: usize = 64;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub transcript: PathBuf,
    /// The folder the session started in, from its transcript.
    pub cwd: Option<String>,
    pub subagents: usize,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Title and usage, when the index already read them.
    pub facts: Option<index::Facts>,
}

#[derive(Debug, Clone)]
pub struct Project {
    /// Folder name in `~/.claude/projects`.
    pub slug: String,
    pub dir: PathBuf,
    /// The real folder this project belongs to, when chatkeep could tell.
    pub path: Option<PathBuf>,
    pub sessions: Vec<Session>,
    pub memory_files: usize,
    pub size: u64,
}

impl Project {
    pub fn destination_missing(&self) -> bool {
        self.path.as_ref().is_some_and(|path| !path.exists())
    }

    pub fn subagents(&self) -> usize {
        self.sessions.iter().map(|session| session.subagents).sum()
    }

    pub fn session(&self, id: &str) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }
}

/// Every project folder, with its real path worked out from `~/.claude.json`, the
/// transcripts, and the desktop app's session list, in that order.
pub fn discover(rt: &Runtime) -> Result<Vec<Project>> {
    let root = rt.layout.projects_dir();
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let known = known_paths(rt)?;
    let cache = rt.open_cache();
    let mut projects = Vec::new();
    for entry in
        fs::read_dir(&root).with_context(|| format!("failed to read {}", root.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(slug) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        projects.push(read_project(&entry.path(), slug, &known, cache.as_ref())?);
    }
    if let Some(cache) = &cache {
        cache.prune()?;
    }
    projects.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(projects)
}

/// Paths Claude Code or its desktop app recorded, by the folder name they map to.
fn known_paths(rt: &Runtime) -> Result<HashMap<String, Vec<String>>> {
    let mut known: HashMap<String, Vec<String>> = HashMap::new();
    let mut add = |path: String| {
        let entry = known.entry(project_slug(&path)).or_default();
        if !entry.contains(&path) {
            entry.push(path);
        }
    };
    if let Some(json) = config::read(&rt.layout.config_file)? {
        for path in config::project_paths(&json) {
            add(path);
        }
    }
    for session in desktop::sessions(&rt.layout)? {
        if let Some(cwd) = session.cwd {
            add(cwd);
        }
    }
    Ok(known)
}

fn read_project(
    dir: &Path,
    slug: String,
    known: &HashMap<String, Vec<String>>,
    cache: Option<&index::Cache>,
) -> Result<Project> {
    let mut sessions = Vec::new();
    let mut memory_files = 0;
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "memory" && entry.file_type()?.is_dir() {
            memory_files = count_files(&path)?;
            continue;
        }
        let Some(id) = name.strip_suffix(".jsonl") else {
            continue;
        };
        if !entry.file_type()?.is_file() {
            continue;
        }
        let extra = dir.join(id);
        let meta = entry.metadata()?;
        let extra_size = if extra.is_dir() { dir_size(&extra)? } else { 0 };
        let facts = match cache {
            Some(cache) => Some(cache.facts(&path)?),
            None => None,
        };
        sessions.push(Session {
            id: id.to_string(),
            cwd: match &facts {
                Some(facts) => facts.cwd.clone(),
                None => first_cwd(&path)?,
            },
            subagents: count_subagents(&extra.join("subagents"))?,
            size: meta.len() + extra_size,
            modified: meta.modified().ok(),
            transcript: path,
            facts,
        });
    }
    sessions.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    let path = resolve_path(&slug, &sessions, known);
    Ok(Project {
        size: dir_size(dir)?,
        dir: dir.to_path_buf(),
        slug,
        path,
        sessions,
        memory_files,
    })
}

/// The recorded path that maps to `slug`, preferring one a transcript confirms. A session
/// that changed folders mid-way still started in the project folder, so the first `cwd`
/// decides.
fn resolve_path(
    slug: &str,
    sessions: &[Session],
    known: &HashMap<String, Vec<String>>,
) -> Option<PathBuf> {
    let from_transcripts: Vec<&str> = sessions
        .iter()
        .filter_map(|session| session.cwd.as_deref())
        .filter(|cwd| project_slug(cwd) == slug)
        .collect();
    if let Some(candidates) = known.get(slug) {
        if let Some(confirmed) = candidates
            .iter()
            .find(|path| from_transcripts.contains(&path.as_str()))
        {
            return Some(PathBuf::from(confirmed));
        }
        if candidates.len() == 1 {
            return Some(PathBuf::from(&candidates[0]));
        }
    }
    from_transcripts.first().map(PathBuf::from)
}

/// The `cwd` of the first transcript line that has one.
pub fn first_cwd(transcript: &Path) -> Result<Option<String>> {
    let file = fs::File::open(transcript)
        .with_context(|| format!("failed to open {}", transcript.display()))?;
    for line in BufReader::new(file).lines().take(CWD_SCAN_LINES) {
        let Ok(line) = line else {
            break;
        };
        if !line.contains("\"cwd\"") {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<Value>(&line)
            && let Some(cwd) = json.get("cwd").and_then(Value::as_str)
        {
            return Ok(Some(cwd.to_string()));
        }
    }
    Ok(None)
}

/// The last custom title a session got: from `<session>/custom-title.json`, else the last
/// `custom-title` line of its transcript.
pub fn title(project: &Project, session: &Session) -> Option<String> {
    let side = project.dir.join(&session.id).join("custom-title.json");
    if let Some(facts) = &session.facts {
        return match fs::read_to_string(&side) {
            Ok(raw) => serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|json| {
                    json.get("customTitle")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .or_else(|| facts.title.clone()),
            Err(_) => facts.title.clone(),
        };
    }
    if let Ok(raw) = fs::read_to_string(&side)
        && let Ok(json) = serde_json::from_str::<Value>(&raw)
        && let Some(title) = json.get("customTitle").and_then(Value::as_str)
    {
        return Some(title.to_string());
    }
    let file = fs::File::open(&session.transcript).ok()?;
    let mut found = None;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if !line.contains("\"custom-title\"") {
            continue;
        }
        if let Ok(json) = serde_json::from_str::<Value>(&line)
            && let Some(title) = json.get("customTitle").and_then(Value::as_str)
        {
            found = Some(title.to_string());
        }
    }
    found
}

fn count_subagents(dir: &Path) -> Result<usize> {
    if !dir.is_dir() {
        return Ok(0);
    }
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        if name.to_string_lossy().ends_with(".jsonl") {
            count += 1;
        }
    }
    Ok(count)
}

fn count_files(dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            count += count_files(&entry.path())?;
        } else {
            count += 1;
        }
    }
    Ok(count)
}

/// The project a path or folder name points at.
pub fn find<'a>(projects: &'a [Project], rt: &Runtime, spec: &str) -> Option<&'a Project> {
    if let Some(project) = projects.iter().find(|project| project.slug == spec) {
        return Some(project);
    }
    let wanted = rt.resolve_path(spec);
    if let Some(project) = projects
        .iter()
        .find(|project| project.path.as_deref() == Some(wanted.as_path()))
    {
        return Some(project);
    }
    // A project chatkeep found no path for still answers to the folder its name encodes.
    let slug = project_slug(&wanted.to_string_lossy());
    projects
        .iter()
        .find(|project| project.path.is_none() && project.slug == slug)
}

/// The project holding session `id`, with the session.
pub fn find_session<'a>(projects: &'a [Project], id: &str) -> Option<(&'a Project, &'a Session)> {
    projects
        .iter()
        .find_map(|project| project.session(id).map(|session| (project, session)))
}
