//! Claude Code: projects in `~/.claude/projects`, the per-path settings in `~/.claude.json`,
//! the prompt history, and the session list of the Claude desktop app.

pub mod accounts;
pub mod archive;
pub mod config;
pub mod desktop;
pub mod index;
pub mod live;
pub mod ops;
pub mod slug;
pub mod stats;
pub mod store;
pub mod transfer;
pub mod view;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cursor::process::{NativeProcesses, ProcessSource};
use crate::cursor::uri::normalize_path_in;

/// Where one Claude Code installation keeps its data, plus chatkeep's own state folder.
#[derive(Debug, Clone)]
pub struct Layout {
    /// `CLAUDE_CONFIG_DIR`, or `~/.claude`.
    pub config_dir: PathBuf,
    /// `.claude.json` in `CLAUDE_CONFIG_DIR`, or in the home folder.
    pub config_file: PathBuf,
    /// The Claude desktop app's data folder, when the app keeps one on this platform.
    pub desktop_dir: Option<PathBuf>,
    pub chatkeep_home: PathBuf,
}

impl Layout {
    /// The layout Claude Code itself uses: `CLAUDE_CONFIG_DIR` moves both the data folder and
    /// `.claude.json`.
    pub fn system(chatkeep_home: PathBuf) -> Result<Self> {
        let home = dirs::home_dir().context("Could not determine home directory")?;
        let custom = std::env::var_os("CLAUDE_CONFIG_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from);
        let (config_dir, config_file) = match custom {
            Some(dir) => (dir.clone(), dir.join(".claude.json")),
            None => (home.join(".claude"), home.join(".claude.json")),
        };
        Ok(Self {
            config_dir,
            config_file,
            desktop_dir: desktop_dir(),
            chatkeep_home,
        })
    }

    /// Whether Claude Code ever ran here.
    pub fn exists(&self) -> bool {
        self.projects_dir().is_dir() || self.config_file.is_file()
    }

    pub fn projects_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }

    /// One `<pid>.json` per running session.
    pub fn live_dir(&self) -> PathBuf {
        self.config_dir.join("sessions")
    }

    /// Prompt history: one JSON line per prompt, with the project path it was typed in.
    pub fn prompt_history(&self) -> PathBuf {
        self.config_dir.join("history.jsonl")
    }

    /// Folders and files keyed by session id rather than by path.
    pub fn session_extras(&self, id: &str) -> Vec<PathBuf> {
        let mut found = vec![
            self.config_dir.join("file-history").join(id),
            self.config_dir.join("session-env").join(id),
        ];
        let todos = self.config_dir.join("todos");
        if let Ok(entries) = std::fs::read_dir(&todos) {
            let mut names: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with(id))
                })
                .collect();
            names.sort();
            found.extend(names);
        }
        found.into_iter().filter(|path| path.exists()).collect()
    }

    pub fn desktop_sessions_dir(&self) -> Option<PathBuf> {
        self.desktop_dir
            .as_ref()
            .map(|dir| dir.join("claude-code-sessions"))
    }

    pub fn backup_root(&self) -> PathBuf {
        self.chatkeep_home.join("backups")
    }

    pub fn history_file(&self) -> PathBuf {
        self.chatkeep_home.join("history.jsonl")
    }
}

/// The Claude desktop app's data folder: `Claude` next to every other Electron app's.
fn desktop_dir() -> Option<PathBuf> {
    crate::config::app_data_dir()
        .ok()
        .map(|dir| dir.join("Claude"))
        .filter(|dir| dir.is_dir())
}

#[derive(Clone)]
pub struct Runtime {
    pub layout: Layout,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
    pub interactive: bool,
    /// Directory every relative path the user typed resolves against.
    pub cwd: PathBuf,
    pub processes: Arc<dyn ProcessSource>,
    /// Remember what each transcript says in `claude-index.db`; off with `CHATKEEP_NO_INDEX`.
    pub cache: bool,
    /// Read every transcript again instead of trusting the cache (`--fresh`).
    pub fresh: bool,
}

impl Runtime {
    pub fn system(chatkeep_home: PathBuf) -> Result<Self> {
        Ok(Self {
            layout: Layout::system(chatkeep_home)?,
            dry_run: false,
            yes: false,
            quiet: false,
            interactive: crate::ui::interactive(),
            cwd: std::env::current_dir().context("cannot read the current directory")?,
            processes: Arc::new(NativeProcesses),
            cache: crate::engine::index::Config::from_env(
                std::env::var("CHATKEEP_NO_INDEX").ok().as_deref(),
            )
            .is_some(),
            fresh: false,
        })
    }

    /// The same runtime, reading every transcript again instead of trusting the index.
    pub fn fresh(&self) -> Runtime {
        let mut rt = self.clone();
        rt.fresh = true;
        rt
    }

    /// The cache of this runtime, when it is on and can be opened.
    pub fn open_cache(&self) -> Option<index::Cache> {
        if !self.cache {
            return None;
        }
        match index::Cache::open(&self.layout.chatkeep_home, self.fresh) {
            Ok(cache) => Some(cache),
            Err(err) => {
                crate::ui::warn(&format!(
                    "the Claude Code index is unavailable, reading live data: {err:#}"
                ));
                None
            }
        }
    }

    pub fn resolve_path(&self, path: impl AsRef<Path>) -> PathBuf {
        normalize_path_in(path.as_ref(), &self.cwd)
    }
}

/// Why a write command cannot run yet: something still uses the chats it would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Busy {
    pub summary: String,
    /// What the user closes to free the chats.
    pub remedy: String,
    /// Set when `queue execute` hit this: how many entries still wait.
    pub pending: Option<usize>,
}

impl Busy {
    pub fn sessions(labels: Vec<String>) -> Self {
        Self {
            summary: format!(
                "Claude Code is running in this project: {}.",
                labels.join(", ")
            ),
            remedy: "Close those sessions".to_string(),
            pending: None,
        }
    }

    pub fn desktop_app(pids: &[u32]) -> Self {
        Self {
            summary: format!(
                "The Claude desktop app is running (pid {}) and lists sessions this command changes.",
                pids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            remedy: "Quit the desktop app".to_string(),
            pending: None,
        }
    }
}

impl std::fmt::Display for Busy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)
    }
}

impl std::error::Error for Busy {}
