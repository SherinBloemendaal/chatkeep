//! `~/.claude.json`: Claude Code's global settings, with one `projects` entry per folder path
//! (trust, allowed tools, MCP servers) and `githubRepoPaths` listing the checkouts per repo.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::fs;
use std::path::Path;

use crate::engine::fsops::write_atomic;

pub fn read(path: &Path) -> Result<Option<Value>> {
    if !path.is_file() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let json = serde_json::from_str(&raw)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    Ok(Some(json))
}

/// Written the way Claude Code writes it: two-space indent, no trailing newline.
pub fn write(path: &Path, json: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(json)?;
    write_atomic(path, text.as_bytes())
}

pub fn project_paths(json: &Value) -> Vec<String> {
    json.get("projects")
        .and_then(Value::as_object)
        .map(|projects| projects.keys().cloned().collect())
        .unwrap_or_default()
}

/// What [`move_project`] did with the settings of `from`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    Nothing,
    /// The entry now sits under the new path, in the same position.
    Renamed,
    /// The new path already had settings; those were kept and the old entry dropped.
    KeptDestination,
}

/// Move the `projects` entry of `from` to `to` and repoint `githubRepoPaths`.
pub fn move_project(json: &mut Value, from: &str, to: &str) -> Moved {
    let mut moved = Moved::Nothing;
    if let Some(projects) = json.get_mut("projects").and_then(Value::as_object_mut)
        && projects.contains_key(from)
    {
        if projects.contains_key(to) {
            projects.shift_remove(from);
            moved = Moved::KeptDestination;
        } else {
            let renamed: Map<String, Value> = std::mem::take(projects)
                .into_iter()
                .map(|(key, value)| {
                    if key == from {
                        (to.to_string(), value)
                    } else {
                        (key, value)
                    }
                })
                .collect();
            *projects = renamed;
            moved = Moved::Renamed;
        }
    }
    for paths in repo_path_lists(json) {
        let mut seen = Vec::new();
        paths.retain_mut(|path| {
            if path.as_str() == Some(from) {
                *path = Value::String(to.to_string());
            }
            if seen.contains(path) {
                return false;
            }
            seen.push(path.clone());
            true
        });
    }
    moved
}

/// Give `to` a copy of the settings of `from`, unless it has its own. Returns whether
/// anything changed.
pub fn copy_project(json: &mut Value, from: &str, to: &str) -> bool {
    let Some(projects) = json.get_mut("projects").and_then(Value::as_object_mut) else {
        return false;
    };
    if projects.contains_key(to) {
        return false;
    }
    let Some(settings) = projects.get(from).cloned() else {
        return false;
    };
    projects.insert(to.to_string(), settings);
    true
}

/// Drop the settings of `path` and its `githubRepoPaths` mentions. Returns whether anything
/// changed.
pub fn remove_project(json: &mut Value, path: &str) -> bool {
    let mut changed = json
        .get_mut("projects")
        .and_then(Value::as_object_mut)
        .is_some_and(|projects| projects.shift_remove(path).is_some());
    for paths in repo_path_lists(json) {
        let before = paths.len();
        paths.retain(|entry| entry.as_str() != Some(path));
        changed |= paths.len() != before;
    }
    changed
}

pub fn has_project(json: &Value, path: &str) -> bool {
    json.get("projects")
        .and_then(Value::as_object)
        .is_some_and(|projects| projects.contains_key(path))
}

fn repo_path_lists(json: &mut Value) -> Vec<&mut Vec<Value>> {
    json.get_mut("githubRepoPaths")
        .and_then(Value::as_object_mut)
        .map(|repos| repos.values_mut().filter_map(Value::as_array_mut).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "numStartups": 3,
            "projects": {
                "/a": {"allowedTools": ["Bash"]},
                "/old": {"hasTrustDialogAccepted": true},
                "/z": {}
            },
            "githubRepoPaths": {"me/app": ["/old", "/other"]}
        })
    }

    #[test]
    fn moving_keeps_the_entry_in_place_and_repoints_repo_paths() {
        let mut json = sample();
        assert_eq!(move_project(&mut json, "/old", "/new"), Moved::Renamed);
        assert_eq!(project_paths(&json), ["/a", "/new", "/z"]);
        assert_eq!(json["projects"]["/new"]["hasTrustDialogAccepted"], true);
        assert_eq!(json["githubRepoPaths"]["me/app"], json!(["/new", "/other"]));
    }

    #[test]
    fn an_existing_destination_keeps_its_own_settings() {
        let mut json = sample();
        assert_eq!(
            move_project(&mut json, "/old", "/a"),
            Moved::KeptDestination
        );
        assert_eq!(project_paths(&json), ["/a", "/z"]);
        assert_eq!(json["projects"]["/a"]["allowedTools"], json!(["Bash"]));
    }

    #[test]
    fn repo_paths_do_not_end_up_twice() {
        let mut json = sample();
        move_project(&mut json, "/old", "/other");
        assert_eq!(json["githubRepoPaths"]["me/app"], json!(["/other"]));
    }

    #[test]
    fn removing_drops_settings_and_repo_paths() {
        let mut json = sample();
        assert!(remove_project(&mut json, "/old"));
        assert!(!has_project(&json, "/old"));
        assert_eq!(json["githubRepoPaths"]["me/app"], json!(["/other"]));
        assert!(!remove_project(&mut json, "/old"));
    }

    #[test]
    fn copying_settings_never_replaces_the_destination() {
        let mut json = sample();
        assert!(copy_project(&mut json, "/old", "/new"));
        assert_eq!(project_paths(&json), ["/a", "/old", "/z", "/new"]);
        assert_eq!(json["projects"]["/new"], json["projects"]["/old"]);
        assert!(!copy_project(&mut json, "/old", "/a"));
        assert!(!copy_project(&mut json, "/nope", "/x"));
        assert_eq!(json["projects"]["/a"]["allowedTools"], json!(["Bash"]));
    }

    #[test]
    fn unknown_paths_change_nothing() {
        let mut json = sample();
        assert_eq!(move_project(&mut json, "/nope", "/x"), Moved::Nothing);
        assert_eq!(json, sample());
    }
}
