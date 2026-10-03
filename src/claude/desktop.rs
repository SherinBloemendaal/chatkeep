//! The session list of the Claude desktop app: one `local_<id>.json` per session in
//! `claude-code-sessions/<account>/<organization>/`, pointing at a Claude Code session id and
//! the folder it runs in.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

use super::Layout;

#[derive(Debug, Clone)]
pub struct DesktopSession {
    pub file: PathBuf,
    pub account: String,
    pub organization: String,
    /// The Claude Code session id, the name of its transcript.
    pub cli_session: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

pub fn sessions(layout: &Layout) -> Result<Vec<DesktopSession>> {
    let Some(root) = layout.desktop_sessions_dir() else {
        return Ok(Vec::new());
    };
    let mut found = Vec::new();
    for account in subdirs(&root)? {
        for organization in subdirs(&account)? {
            for entry in fs::read_dir(&organization)
                .with_context(|| format!("failed to read {}", organization.display()))?
            {
                let file = entry?.path();
                let is_session = file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"));
                if !is_session {
                    continue;
                }
                let Some(json) = read_json(&file)? else {
                    continue;
                };
                let text = |key: &str| json.get(key).and_then(Value::as_str).map(str::to_string);
                found.push(DesktopSession {
                    account: name_of(&account),
                    organization: name_of(&organization),
                    cli_session: text("cliSessionId"),
                    cwd: text("cwd"),
                    title: text("title"),
                    file,
                });
            }
        }
    }
    found.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(found)
}

/// A session file that is not valid JSON is skipped: the app may be writing it, and the list
/// only informs what chatkeep rewrites, never what it deletes.
fn read_json(file: &Path) -> Result<Option<Value>> {
    let raw =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    Ok(serde_json::from_str(&raw).ok())
}

pub(super) fn subdirs(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

pub(super) fn name_of(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}
