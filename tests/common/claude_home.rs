//! A fake `~/.claude`, `~/.claude.json`, and Claude desktop app folder.

use anyhow::Result;
use chatkeep::claude::{self, ops, slug::project_slug};
use chatkeep::cursor::process::{ProcessInfo, ProcessSource};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const SESSION: &str = "11111111-1111-4111-8111-111111111111";
pub const OTHER: &str = "22222222-2222-4222-8222-222222222222";
pub const ACCOUNT: &str = "acct";
pub const ORG: &str = "org";

pub struct Processes(pub Vec<ProcessInfo>);

impl ProcessSource for Processes {
    fn processes(&self) -> Result<Vec<ProcessInfo>> {
        Ok(self.0.clone())
    }
}

pub fn process(pid: u32, exe: &str) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: exe.rsplit('/').next().unwrap_or(exe).to_string(),
        exe: Some(PathBuf::from(exe)),
        args: Vec::new(),
    }
}

pub struct Home {
    _root: tempfile::TempDir,
    pub root: PathBuf,
    pub rt: claude::Runtime,
}

impl Home {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let layout = claude::Layout {
            config_dir: path.join(".claude"),
            config_file: path.join(".claude.json"),
            desktop_dir: Some(path.join("Claude")),
            chatkeep_home: path.join("chatkeep"),
        };
        fs::create_dir_all(layout.projects_dir()).unwrap();
        let rt = claude::Runtime {
            layout,
            dry_run: false,
            yes: true,
            quiet: true,
            interactive: false,
            cwd: path.clone(),
            processes: Arc::new(Processes(Vec::new())),
            cache: true,
            fresh: false,
        };
        Self {
            _root: root,
            root: path,
            rt,
        }
    }

    pub fn folder(&self, name: &str) -> PathBuf {
        let dir = self.root.join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn project_dir(&self, path: &Path) -> PathBuf {
        self.rt
            .layout
            .projects_dir()
            .join(project_slug(&path.to_string_lossy()))
    }

    /// A session started in `path`, with one subagent and a memory file mentioning the path.
    pub fn session(&self, path: &Path, id: &str) {
        let dir = self.project_dir(path);
        let cwd = path.to_string_lossy();
        let lines = [
            json!({"type": "user", "cwd": cwd, "sessionId": id, "message": {"content": format!("edit {cwd}/src/main.rs")}}),
            json!({"type": "custom-title", "customTitle": "Work", "sessionId": id}),
            json!({"type": "assistant", "cwd": cwd, "sessionId": id, "message": {"content": format!("see {cwd}-old and {cwd}/README")}}),
        ];
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::create_dir_all(dir.join(id).join("subagents")).unwrap();
        fs::write(dir.join(format!("{id}.jsonl")), text).unwrap();
        fs::write(
            dir.join(id).join("subagents").join("agent-a1.jsonl"),
            format!("{}\n", json!({"cwd": cwd, "isSidechain": true})),
        )
        .unwrap();
    }

    /// A session with exactly these transcript lines.
    pub fn transcript(&self, path: &Path, id: &str, lines: &[Value]) -> PathBuf {
        let dir = self.project_dir(path);
        fs::create_dir_all(&dir).unwrap();
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        let file = dir.join(format!("{id}.jsonl"));
        fs::write(&file, text).unwrap();
        file
    }

    /// Ids of the sessions the project of `path` holds, sorted.
    pub fn session_ids(&self, path: &Path) -> Vec<String> {
        let mut ids: Vec<String> = fs::read_dir(self.project_dir(path))
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| {
                        entry
                            .file_name()
                            .to_str()
                            .and_then(|name| name.strip_suffix(".jsonl"))
                            .map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        ids
    }

    /// The desktop app's entries of one account, parsed, sorted by file name.
    pub fn desktop_entries(&self, account: &str, organization: &str) -> Vec<Value> {
        let dir = self
            .rt
            .layout
            .desktop_sessions_dir()
            .unwrap()
            .join(account)
            .join(organization);
        let mut files: Vec<PathBuf> = fs::read_dir(dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        files.sort();
        files
            .iter()
            .map(|file| serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap())
            .collect()
    }

    /// A desktop app entry under any account, with the fields an account owns.
    pub fn desktop_entry(
        &self,
        account: &str,
        organization: &str,
        name: &str,
        cli: &str,
        cwd: &Path,
    ) -> PathBuf {
        let dir = self
            .rt
            .layout
            .desktop_sessions_dir()
            .unwrap()
            .join(account)
            .join(organization);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("local_{name}.json"));
        fs::write(
            &file,
            serde_json::to_string(&json!({
                "sessionId": format!("local_{name}"),
                "cliSessionId": cli,
                "cwd": cwd.to_string_lossy(),
                "originCwd": cwd.to_string_lossy(),
                "title": format!("Chat {name}"),
                "bridgeSessionIds": ["session_remote"],
                "remoteMcpServersConfig": [{"name": "mail"}]
            }))
            .unwrap(),
        )
        .unwrap();
        file
    }

    /// Which account the desktop app is signed in with.
    pub fn signed_in(&self, account: &str) {
        let dir = self.rt.layout.desktop_dir.clone().unwrap();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("config.json"),
            json!({"lastKnownAccountUuid": account}).to_string(),
        )
        .unwrap();
    }

    pub fn memory(&self, path: &Path, name: &str, text: &str) {
        let dir = self.project_dir(path).join("memory");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(name), text).unwrap();
    }

    pub fn config(&self, value: Value) {
        fs::write(
            &self.rt.layout.config_file,
            serde_json::to_string_pretty(&value).unwrap(),
        )
        .unwrap();
    }

    pub fn read_config(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(&self.rt.layout.config_file).unwrap()).unwrap()
    }

    pub fn prompts(&self, lines: &[Value]) {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(self.rt.layout.prompt_history(), text).unwrap();
    }

    pub fn desktop_session(&self, name: &str, cli: &str, cwd: &Path) -> PathBuf {
        let dir = self
            .rt
            .layout
            .desktop_sessions_dir()
            .unwrap()
            .join(ACCOUNT)
            .join(ORG);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("local_{name}.json"));
        let cwd = cwd.to_string_lossy();
        fs::write(
            &file,
            serde_json::to_string(&json!({
                "sessionId": format!("local_{name}"),
                "cliSessionId": cli,
                "cwd": cwd,
                "originCwd": cwd,
                "title": "Desk",
                "writtenBranches": [format!("{cwd}\u{0}main")]
            }))
            .unwrap(),
        )
        .unwrap();
        file
    }

    pub fn live(&mut self, pid: u32, cwd: &Path, exe: &str) {
        let dir = self.rt.layout.live_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(format!("{pid}.json")),
            json!({"pid": pid, "sessionId": SESSION, "cwd": cwd, "name": "Busy"}).to_string(),
        )
        .unwrap();
        self.rt.processes = Arc::new(Processes(vec![process(pid, exe)]));
    }

    pub fn running(&mut self, processes: Vec<ProcessInfo>) {
        self.rt.processes = Arc::new(Processes(processes));
    }

    pub fn mv(&self, from: &Path, to: &Path) -> Result<chatkeep::engine::Report> {
        let pairs = [(
            from.to_string_lossy().into_owned(),
            to.to_string_lossy().into_owned(),
        )];
        let (plans, report) = ops::plan_move(&self.rt, &pairs, None, false, false)?;
        ops::execute_move(&self.rt, plans, report)
    }

    /// Every file under the fake home, with its bytes, to prove a run changed nothing.
    pub fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out.retain(|(path, _)| !path.starts_with(self.root.join("chatkeep")));
        out.sort();
        out
    }
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.push((path.clone(), Vec::new()));
            collect(&path, out);
        } else {
            out.push((path.clone(), fs::read(&path).unwrap()));
        }
    }
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap()
}
