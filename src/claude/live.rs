//! What is running right now: Claude Code sessions, from the `<pid>.json` files each one keeps
//! in `~/.claude/sessions`, and the Claude desktop app, from the process table.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::Layout;
use crate::cursor::process::ProcessInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSession {
    pub pid: u32,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub name: Option<String>,
}

impl LiveSession {
    pub fn label(&self) -> String {
        match &self.name {
            Some(name) => format!("{name} (pid {})", self.pid),
            None => format!("pid {}", self.pid),
        }
    }
}

/// Sessions whose process still runs. A file left behind by a session that crashed names a
/// pid that is gone, so it does not count.
pub fn sessions(layout: &Layout, processes: &[ProcessInfo]) -> Result<Vec<LiveSession>> {
    let dir = layout.live_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let alive: HashSet<u32> = processes.iter().map(|process| process.pid).collect();
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(session) = read(&path)? else {
            continue;
        };
        if alive.contains(&session.pid) {
            found.push(session);
        }
    }
    found.sort_by_key(|session| session.pid);
    Ok(found)
}

fn read(path: &Path) -> Result<Option<LiveSession>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let Ok(json) = serde_json::from_str::<Value>(&raw) else {
        return Ok(None);
    };
    let Some(pid) = json
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
    else {
        return Ok(None);
    };
    let text = |key: &str| json.get(key).and_then(Value::as_str).map(str::to_string);
    Ok(Some(LiveSession {
        pid,
        session_id: text("sessionId"),
        cwd: text("cwd"),
        name: text("name"),
    }))
}

/// Main processes of the Claude desktop app. Its helpers carry other names and the CLI runs
/// from a versioned binary, so neither counts.
pub fn desktop_apps(processes: &[ProcessInfo]) -> Vec<u32> {
    processes
        .iter()
        .filter(|process| is_desktop_app(process))
        .map(|process| process.pid)
        .collect()
}

fn is_desktop_app(process: &ProcessInfo) -> bool {
    let Some(exe) = &process.exe else {
        return false;
    };
    let exe = exe.to_string_lossy().replace('\\', "/");
    let lower = exe.to_lowercase();
    exe.ends_with("Claude.app/Contents/MacOS/Claude")
        || (lower.contains("/anthropicclaude/") && lower.ends_with("/claude.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn process(pid: u32, exe: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: exe.rsplit('/').next().unwrap_or(exe).to_string(),
            exe: Some(PathBuf::from(exe)),
            args: Vec::new(),
        }
    }

    #[test]
    fn only_sessions_with_a_running_pid_count() {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout {
            config_dir: root.path().join(".claude"),
            config_file: root.path().join(".claude.json"),
            desktop_dir: None,
            chatkeep_home: root.path().join("state"),
        };
        fs::create_dir_all(layout.live_dir()).unwrap();
        fs::write(
            layout.live_dir().join("10.json"),
            r#"{"pid":10,"sessionId":"s1","cwd":"/a","name":"Work"}"#,
        )
        .unwrap();
        fs::write(
            layout.live_dir().join("11.json"),
            r#"{"pid":11,"cwd":"/b"}"#,
        )
        .unwrap();
        fs::write(layout.live_dir().join("12.json"), "{not json").unwrap();
        fs::write(layout.live_dir().join("10.abc.key"), "x").unwrap();

        let found = sessions(&layout, &[process(10, "/x/versions/2.1.0")]).unwrap();
        assert_eq!(
            found,
            [LiveSession {
                pid: 10,
                session_id: Some("s1".into()),
                cwd: Some("/a".into()),
                name: Some("Work".into()),
            }]
        );
        assert_eq!(found[0].label(), "Work (pid 10)");
    }

    #[test]
    fn desktop_app_is_its_main_process_only() {
        let processes = [
            process(1, "/Applications/Claude.app/Contents/MacOS/Claude"),
            process(
                2,
                "/Applications/Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS/Claude Helper",
            ),
            process(3, "/Users/me/.local/share/claude/versions/2.1.287"),
            process(
                4,
                "C:/Users/me/AppData/Local/AnthropicClaude/app-1.0/Claude.exe",
            ),
        ];
        assert_eq!(desktop_apps(&processes), [1, 4]);
    }
}
