//! Persistent command queue for write commands deferred while Cursor is open.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use comfy_table::{Attribute, Cell, CellAlignment, Color};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use uuid::Uuid;

use super::fsops::write_atomic;
use super::index::now_ms;
use crate::ui::{self, Align, Sheet, Theme};

pub const SCHEMA: u32 = 1;
const MAX_LOCK_AGE_MS: i64 = 6 * 60 * 60 * 1000;
const UNREADABLE_GRACE: Duration = Duration::from_secs(10);

pub fn queue_path(home: &Path) -> PathBuf {
    home.join("queue.jsonl")
}

pub fn lock_path(home: &Path) -> PathBuf {
    home.join("queue.lock")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Running,
    Failed,
    Skipped,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub schema: u32,
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub created_at: String,
    #[serde(alias = "crepath_version")]
    pub tool_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

pub fn load(home: &Path) -> Result<Vec<Entry>> {
    let path = queue_path(home);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let text = String::from_utf8(raw)
        .with_context(|| format!("queue file is not UTF-8: {}", path.display()))?;
    parse_jsonl(&text, &path)
}

fn parse_jsonl(text: &str, path: &Path) -> Result<Vec<Entry>> {
    let incomplete_tail = !text.is_empty() && !text.ends_with('\n');
    let chunks: Vec<&str> = text.split_inclusive('\n').collect();
    let last = chunks.len().saturating_sub(1);
    let mut entries = Vec::new();
    for (index, chunk) in chunks.iter().enumerate() {
        let line_no = index + 1;
        let content = chunk.strip_suffix('\n').unwrap_or(chunk);
        if content.trim().is_empty() {
            continue;
        }
        let truncated = incomplete_tail && index == last;
        match serde_json::from_str::<Entry>(content) {
            Ok(entry) if entry.schema != SCHEMA => {
                bail!(
                    "unsupported queue schema {} on line {line_no} of {} (want {SCHEMA})",
                    entry.schema,
                    path.display()
                );
            }
            Ok(entry) => entries.push(entry),
            Err(err) if truncated => {
                bail!(
                    "queue.jsonl line {line_no} is truncated (incomplete write) in {}: {err}",
                    path.display()
                );
            }
            Err(err) => {
                bail!(
                    "queue.jsonl line {line_no} is malformed in {}: {err}",
                    path.display()
                );
            }
        }
    }
    Ok(entries)
}

fn rewrite(home: &Path, entries: &[Entry]) -> Result<()> {
    fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
    let mut bytes = Vec::new();
    for entry in entries {
        serde_json::to_writer(&mut bytes, entry)?;
        bytes.push(b'\n');
    }
    let path = queue_path(home);
    write_atomic(&path, &bytes)?;
    restrict_mode(&path)
}

fn restrict_mode(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to set permissions on {}", path.display()))?;
    }
    let _ = path;
    Ok(())
}

pub fn require_lock(home: &Path) -> Result<Lock> {
    match Lock::acquire(home)? {
        Ok(lock) => Ok(lock),
        Err(holder) => Err(ui::hinted(
            format!(
                "another chatkeep queue command is running (pid {})",
                holder.pid
            ),
            "Wait for it to finish and retry.",
        )),
    }
}

pub fn pending(entries: &[Entry]) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|entry| entry.status == Status::Pending)
        .collect()
}

pub fn add(
    home: &Path,
    argv: Vec<String>,
    cwd: PathBuf,
    profile: Option<String>,
    warnings: Vec<String>,
) -> Result<(Entry, usize)> {
    let _lock = require_lock(home)?;
    let entries = load(home)?;
    let entry = Entry {
        schema: SCHEMA,
        id: short_id(),
        argv,
        cwd,
        created_at: Utc::now().to_rfc3339(),
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        profile,
        status: Status::Pending,
        warnings,
        last_error: None,
    };
    append_line(home, &entry)?;
    let position = pending(&entries).len() + 1;
    Ok((entry, position))
}

fn append_line(home: &Path, entry: &Entry) -> Result<()> {
    fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
    let path = queue_path(home);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    if file.metadata()?.len() > 0 {
        let mut tail = OpenOptions::new()
            .read(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        use std::io::{Read, Seek, SeekFrom};
        tail.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        tail.read_exact(&mut last)?;
        if last[0] != b'\n' {
            file.write_all(b"\n")
                .with_context(|| format!("failed to append {}", path.display()))?;
        }
    }
    let mut line = serde_json::to_vec(entry)?;
    line.push(b'\n');
    file.write_all(&line)
        .with_context(|| format!("failed to append {}", path.display()))?;
    file.flush()?;
    file.sync_all()?;
    restrict_mode(&path)
}

pub fn remove(home: &Path, ids: &[String]) -> Result<Vec<String>> {
    let _lock = require_lock(home)?;
    remove_locked(home, ids)
}

/// `remove` for a caller that already holds the queue lock.
pub fn remove_locked(home: &Path, ids: &[String]) -> Result<Vec<String>> {
    let mut entries = load(home)?;
    let mut removed = Vec::new();
    for id in ids {
        let Some(index) = entries.iter().position(|entry| entry.id == *id) else {
            bail!("no queue entry {id}");
        };
        removed.push(entries.remove(index).id);
    }
    rewrite(home, &entries)?;
    Ok(removed)
}

pub fn clear_all(home: &Path) -> Result<usize> {
    let _lock = require_lock(home)?;
    let entries = load(home)?;
    let removed = entries.len();
    rewrite(home, &[])?;
    Ok(removed)
}

pub fn retry(home: &Path, ids: Option<&[String]>) -> Result<Vec<String>> {
    let _lock = require_lock(home)?;
    let mut entries = load(home)?;
    let mut reset = Vec::new();
    match ids {
        Some(ids) => {
            for id in ids {
                let entry = entries
                    .iter_mut()
                    .find(|entry| entry.id == *id)
                    .ok_or_else(|| anyhow::anyhow!("no queue entry {id}"))?;
                if !matches!(entry.status, Status::Failed | Status::Skipped) {
                    bail!(
                        "queue entry {id} is {} (only failed or skipped entries can be retried)",
                        entry.status.as_str()
                    );
                }
                entry.status = Status::Pending;
                entry.last_error = None;
                reset.push(id.clone());
            }
        }
        None => {
            for entry in &mut entries {
                if matches!(entry.status, Status::Failed | Status::Skipped) {
                    entry.status = Status::Pending;
                    entry.last_error = None;
                    reset.push(entry.id.clone());
                }
            }
        }
    }
    rewrite(home, &entries)?;
    Ok(reset)
}

const INTERRUPTED: &str =
    "interrupted during a previous run; check chatkeep history, then retry or rm";

pub fn fail_interrupted(home: &Path) -> Result<Vec<String>> {
    let _lock = require_lock(home)?;
    fail_interrupted_locked(home)
}

pub fn fail_interrupted_locked(home: &Path) -> Result<Vec<String>> {
    let mut entries = load(home)?;
    let mut marked = Vec::new();
    for entry in &mut entries {
        if entry.status == Status::Running {
            entry.status = Status::Failed;
            entry.last_error = Some(INTERRUPTED.into());
            marked.push(entry.id.clone());
        }
    }
    if !marked.is_empty() {
        rewrite(home, &entries)?;
    }
    Ok(marked)
}

pub fn update(home: &Path, id: &str, status: Status, last_error: Option<String>) -> Result<()> {
    let _lock = require_lock(home)?;
    update_locked(home, id, status, last_error)
}

/// `update` for a caller that already holds the queue lock.
pub fn update_locked(
    home: &Path,
    id: &str,
    status: Status,
    last_error: Option<String>,
) -> Result<()> {
    let mut entries = load(home)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow::anyhow!("no queue entry {id}"))?;
    entry.status = status;
    entry.last_error = last_error;
    rewrite(home, &entries)
}

/// Replace the stored command of an entry, for a caller that already holds the queue lock.
pub fn set_argv_locked(home: &Path, id: &str, argv: Vec<String>) -> Result<()> {
    let mut entries = load(home)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == id)
        .ok_or_else(|| anyhow::anyhow!("no queue entry {id}"))?;
    entry.argv = argv;
    rewrite(home, &entries)
}

fn short_id() -> String {
    let id = Uuid::new_v4().simple().to_string();
    id[..8].to_string()
}

/// Paths this short stay whole; the table wraps the command instead of shortening them.
const MIN_PATH: usize = 20;

pub fn render_list(theme: Theme, entries: &[Entry], full: bool) -> String {
    render_list_at(theme, entries, full, ui::table_width())
}

pub fn render_list_at(theme: Theme, entries: &[Entry], full: bool, total: Option<usize>) -> String {
    if entries.is_empty() {
        return format!("{}\n", ui::info_line(theme, "Queue is empty."));
    }
    let ellipsis = theme.icons().ellipsis;
    let mut sheet = Sheet::new(
        theme,
        &[
            ("id", Align::Left),
            ("pos", Align::Right),
            ("command", Align::Left),
            ("profile", Align::Left),
            ("created", Align::Left),
            ("status", Align::Left),
        ],
    )
    .flex(2)
    .min_flex(20)
    .optional(&[4, 1])
    .collapsible(&[1, 3])
    .at(total);
    let mut pending_pos = 0usize;
    let mut shortened = false;
    for entry in entries {
        let position = if entry.status == Status::Pending {
            pending_pos += 1;
            theme
                .cell(ui::count(pending_pos as u64), None, &[Attribute::Bold])
                .set_alignment(CellAlignment::Right)
        } else {
            theme.cell("-", None, &[Attribute::Dim])
        };
        let profile = match &entry.profile {
            Some(name) => super::view::profile_cell(theme, name),
            None => theme.cell("-", None, &[Attribute::Dim]),
        };
        let cells = |command: String| {
            vec![
                theme.cell(&entry.id, Some(Color::Cyan), &[]),
                position.clone(),
                Cell::new(command),
                profile.clone(),
                theme.cell(format_created(&entry.created_at), None, &[Attribute::Dim]),
                status_cell(theme, entry),
            ]
        };
        let exact = shell_join(&entry.argv);
        if full {
            sheet.row(cells(exact));
            continue;
        }
        let tokens = display_tokens(entry);
        let display = tokens.join(" ");
        shortened |= display != exact;
        sheet.fitted_row(cells(display), move |room| {
            Cell::new(fit_tokens(&tokens, room, ellipsis))
        });
    }
    let mut out = format!("{}\n{sheet}\n", ui::section_line(theme, "Queue"));
    out.push_str(&list_footer(theme, entries, total));
    if shortened {
        out.push_str(&ui::layout::wrap_indented(
            "Commands are shortened; queue list --full shows them as stored.",
            2,
            total.unwrap_or(usize::MAX / 2),
            |line| theme.dim(line),
        ));
    }
    out.push_str(&error_lines(theme, entries, total));
    out
}

fn list_footer(theme: Theme, entries: &[Entry], total: Option<usize>) -> String {
    let count = |status: Status| {
        entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    };
    let mut parts = vec![
        theme.bold(ui::plural(entries.len(), "entry", "entries")),
        format!("{} pending", ui::count(count(Status::Pending) as u64)),
    ];
    for status in [Status::Running, Status::Failed, Status::Skipped] {
        let found = count(status);
        if found == 0 {
            continue;
        }
        let text = format!("{} {}", ui::count(found as u64), status.as_str());
        parts.push(match status {
            Status::Failed => theme.bad(text),
            _ => theme.caution(text),
        });
    }
    ui::summary_at(theme, &parts, total)
}

fn error_lines(theme: Theme, entries: &[Entry], total: Option<usize>) -> String {
    let icons = theme.icons();
    let mut out = String::new();
    for entry in entries {
        let Some(error) = entry.last_error.as_deref() else {
            continue;
        };
        let icon = if entry.status == Status::Skipped {
            theme.caution(icons.warn)
        } else {
            theme.bad(icons.cross)
        };
        let lead = format!("{icon} {} ", theme.id(&entry.id));
        let indent = ui::layout::width(&lead);
        let room = total.map_or(usize::MAX / 2, |total| total.saturating_sub(indent).max(16));
        for (index, line) in ui::layout::wrap_words(error, room).iter().enumerate() {
            if index == 0 {
                out.push_str(&lead);
            } else {
                out.push_str(&" ".repeat(indent));
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// The command as `queue list` shows it: home as `~`, short workspace hashes, and without the
/// `--profile` (it has its own column) and `-y` (queue execute never prompts) that it repeats.
pub fn display_tokens(entry: &Entry) -> Vec<String> {
    let profile = entry.profile.as_deref();
    let mut out = Vec::new();
    let mut args = entry.argv.iter().peekable();
    let mut positional = false;
    while let Some(arg) = args.next() {
        if !positional {
            match arg.as_str() {
                "--" => positional = true,
                "-y" | "--yes" => continue,
                "--profile" if args.peek().map(|value| value.as_str()) == profile => {
                    args.next();
                    continue;
                }
                "--replace" => {
                    out.push(arg.clone());
                    for value in args.by_ref().take(2) {
                        out.push(display_token(value));
                    }
                    continue;
                }
                other if profile.is_some() && other.strip_prefix("--profile=") == profile => {
                    continue;
                }
                _ => {}
            }
        }
        out.push(display_token(arg));
    }
    out
}

fn display_token(arg: &str) -> String {
    let short = super::view::short_hash(arg);
    if short != arg {
        return short.to_string();
    }
    let relative = ui::home_relative(arg);
    if shell_quote(arg) == arg {
        relative
    } else {
        shell_quote(&relative)
    }
}

fn is_path(token: &str) -> bool {
    token.contains('/') || token.contains('\\')
}

/// Shrink the longest paths until the command fits `room`; whatever is left wraps.
fn fit_tokens(tokens: &[String], room: usize, ellipsis: &str) -> String {
    let mut tokens = tokens.to_vec();
    loop {
        let joined = tokens.join(" ");
        let excess = ui::layout::width(&joined).saturating_sub(room);
        if excess == 0 {
            return joined;
        }
        let Some((index, current)) = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| is_path(token))
            .map(|(index, token)| (index, ui::layout::width(token)))
            .filter(|(_, width)| *width > MIN_PATH)
            .max_by_key(|(_, width)| *width)
        else {
            return joined;
        };
        let target = current.saturating_sub(excess).max(MIN_PATH);
        let shorter = ui::layout::shorten_path(&tokens[index], target, ellipsis);
        if ui::layout::width(&shorter) >= current {
            return joined;
        }
        tokens[index] = shorter;
    }
}

fn status_cell(theme: Theme, entry: &Entry) -> Cell {
    let status = entry.status;
    match status {
        Status::Pending => theme.cell(status.as_str(), Some(Color::Yellow), &[]),
        Status::Running => theme.cell(status.as_str(), Some(Color::Blue), &[]),
        Status::Failed if entry.last_error.as_deref() == Some(INTERRUPTED) => {
            theme.cell("interrupted", Some(Color::Red), &[])
        }
        Status::Failed => theme.cell(status.as_str(), Some(Color::Red), &[]),
        Status::Skipped => theme.cell(status.as_str(), Some(Color::DarkGrey), &[]),
    }
}

fn format_created(raw: &str) -> String {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| {
            stamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| raw.to_string())
}

pub fn shell_quote(arg: &str) -> String {
    if cfg!(windows) {
        powershell_quote(arg)
    } else {
        posix_quote(arg)
    }
}

fn posix_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.' | '/' | ':' | '@' | '=' | '+' | ',')
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn powershell_quote(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '-' | '_' | '.' | '/' | ':' | '@' | '=' | '+' | ',' | '\\'
            )
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "''"))
}

pub fn shell_join(args: &[String]) -> String {
    args.iter()
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn queue_add_command(args: &[String]) -> String {
    format!("chatkeep queue add -- {}", shell_join(args))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub started: i64,
    pub alive: bool,
}

fn process_alive(pid: u32, started: i64) -> bool {
    if pid == std::process::id() {
        return true;
    }
    if pid == 0 || !sysinfo::IS_SUPPORTED_SYSTEM {
        return false;
    }
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(pid).is_some_and(|process| {
        let born = i64::try_from(process.start_time()).unwrap_or(i64::MAX);
        born.saturating_mul(1000) <= started + 2_000
    })
}

pub fn holder(home: &Path) -> Option<Holder> {
    let path = lock_path(home);
    let raw = fs::read_to_string(&path).ok()?;
    let parsed = serde_json::from_str::<serde_json::Value>(&raw)
        .ok()
        .and_then(|json| {
            Some((
                u32::try_from(json.get("pid")?.as_u64()?).ok()?,
                json.get("started")?.as_i64()?,
            ))
        });
    let Some((pid, started)) = parsed else {
        let young = fs::metadata(&path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < UNREADABLE_GRACE);
        return Some(Holder {
            pid: 0,
            started: 0,
            alive: young,
        });
    };
    let alive = now_ms() - started < MAX_LOCK_AGE_MS && process_alive(pid, started);
    Some(Holder {
        pid,
        started,
        alive,
    })
}

#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    pid: u32,
}

impl Lock {
    pub fn acquire(home: &Path) -> Result<std::result::Result<Lock, Holder>> {
        fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
        let path = lock_path(home);
        for _ in 0..3 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let pid = std::process::id();
                    file.write_all(
                        serde_json::json!({"pid": pid, "started": now_ms()})
                            .to_string()
                            .as_bytes(),
                    )?;
                    file.sync_all()?;
                    return Ok(Ok(Lock { path, pid }));
                }
                Err(err) if err.kind() == ErrorKind::AlreadyExists => match holder(home) {
                    Some(holder) if holder.alive => return Ok(Err(holder)),
                    _ => {
                        let _ = fs::remove_file(&path);
                    }
                },
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("failed to create {}", path.display()));
                }
            }
        }
        Ok(Err(holder(home).unwrap_or(Holder {
            pid: 0,
            started: 0,
            alive: true,
        })))
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let ours = fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .and_then(|json| json.get("pid").and_then(serde_json::Value::as_u64))
            == Some(u64::from(self.pid));
        if ours {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_keeps_safe_tokens_and_quotes_the_rest() {
        assert_eq!(shell_quote("mv"), "mv");
        assert_eq!(shell_quote("/Machines/older/"), "/Machines/older/");
        assert_eq!(shell_quote("a b"), "'a b'");
        #[cfg(unix)]
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        #[cfg(windows)]
        assert_eq!(shell_quote("it's"), "'it''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[cfg(windows)]
    #[test]
    fn shell_quote_uses_powershell_quoting() {
        assert_eq!(shell_quote(r"C:\Users\a"), r"C:\Users\a");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it''s'");
    }

    #[cfg(unix)]
    #[test]
    fn shell_quote_round_trips_special_characters_through_sh() {
        for arg in [
            "", "$HOME", "*", "?", "~", "`uname`", "it's", "a\nb", "plain",
        ] {
            let quoted = shell_quote(arg);
            let script = format!(r#"sh -c 'printf "%s\n" "$@"' _ {quoted}"#);
            let output = std::process::Command::new("sh")
                .arg("-c")
                .arg(&script)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "sh failed for {arg:?} as {quoted}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let got = String::from_utf8(output.stdout).unwrap();
            let want = format!("{arg}\n");
            assert_eq!(got, want, "arg={arg:?} quoted={quoted}");
        }
    }

    fn listed(id: &str, argv: &[&str], profile: Option<&str>, status: Status) -> Entry {
        Entry {
            schema: SCHEMA,
            id: id.to_string(),
            argv: argv.iter().map(|arg| arg.to_string()).collect(),
            cwd: PathBuf::from("/tmp"),
            created_at: "2026-10-01T10:20:00+00:00".to_string(),
            tool_version: "1.1.0".to_string(),
            profile: profile.map(str::to_string),
            status,
            warnings: Vec::new(),
            last_error: None,
        }
    }

    fn sample_queue() -> Vec<Entry> {
        let home = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/home/me"))
            .display()
            .to_string();
        let deep = format!("{home}/Machines/upgraded/home/me/projects/workshop/api-demo-isolated");
        vec![
            listed(
                "aaaa1111",
                &[
                    "mv",
                    "--replace",
                    "/Machines/older/",
                    "/Machines/upgraded/",
                    "--profile",
                    "daily",
                    "-y",
                ],
                Some("daily"),
                Status::Pending,
            ),
            listed(
                "bbbb2222",
                &[
                    "mv",
                    "--profile",
                    "workshop",
                    "1f2e3d4c5b6a79880123456789abcdef",
                    &deep,
                    "-y",
                ],
                Some("workshop"),
                Status::Pending,
            ),
            listed(
                "cccc3333",
                &[
                    "combine",
                    "--move",
                    "--profile=daily-2",
                    &format!("{home}/Machines/upgraded/home/me/projects/workshop/landing"),
                    "a1a1a1a1b2b2b2b2c3c3c3c3d4d4d4d4",
                    "e5e5e5e5f6f6f6f6a7a7a7a7b8b8b8b8",
                    "--yes",
                ],
                Some("daily-2"),
                Status::Pending,
            ),
        ]
    }

    fn table_lines(out: &str) -> Vec<&str> {
        out.lines()
            .filter(|line| line.starts_with(['╭', '│', '├', '╰', '+', '|']))
            .collect()
    }

    #[test]
    fn queue_list_fits_every_width_with_a_closed_frame() {
        let mut entries = sample_queue();
        entries[0].status = Status::Failed;
        entries[0].last_error = Some(format!(
            "queued cwd no longer exists: {} and the rest of a long explanation follows here",
            "/x".repeat(20)
        ));
        for theme in [Theme::plain(), Theme::colored(), Theme::ascii()] {
            for total in [200, 120, 80, 60] {
                for full in [false, true] {
                    let out = render_list_at(theme, &entries, full, Some(total));
                    for line in out.lines() {
                        assert!(ui::layout::width(line) <= total, "{total}: {line}");
                    }
                    let table = table_lines(&out);
                    assert!(table.len() > 4);
                    let first = ui::layout::width(table[0]);
                    for line in &table {
                        assert_eq!(ui::layout::width(line), first, "{total}: {line}");
                    }
                    let (top, bottom) = if theme.unicode() {
                        ('╭', '╰')
                    } else {
                        ('+', '+')
                    };
                    assert!(table[0].starts_with(top));
                    assert!(table[table.len() - 1].starts_with(bottom));
                    if !theme.unicode() {
                        assert!(out.is_ascii(), "{out}");
                    }
                }
            }
            let colored = render_list_at(theme, &entries, false, Some(120));
            let plain = render_list_at(
                if theme.unicode() {
                    Theme::plain()
                } else {
                    Theme::ascii()
                },
                &entries,
                false,
                Some(120),
            );
            assert_eq!(ui::layout::strip_ansi(&colored), plain);
        }
    }

    #[test]
    fn queue_list_drops_the_repeated_profile_and_yes_flag() {
        let entries = sample_queue();
        let tokens = display_tokens(&entries[0]);
        assert_eq!(
            tokens,
            ["mv", "--replace", "/Machines/older/", "/Machines/upgraded/"]
        );
        let tokens = display_tokens(&entries[1]);
        assert_eq!(tokens[..2], ["mv", "1f2e3d4c"]);
        assert!(!tokens.contains(&"--profile".to_string()));
        assert!(!tokens.contains(&"-y".to_string()));
        if dirs::home_dir().is_some() {
            assert!(tokens[2].starts_with("~/Machines/"), "{tokens:?}");
        }
        let tokens = display_tokens(&entries[2]);
        assert!(!tokens.iter().any(|token| token.starts_with("--profile")));
        assert!(!tokens.contains(&"--yes".to_string()));
        assert_eq!(tokens[tokens.len() - 2..], ["a1a1a1a1", "e5e5e5e5"]);

        let mut other = entries[1].clone();
        other.argv[2] = "work".to_string();
        assert!(display_tokens(&other).contains(&"--profile".to_string()));
        let after_dash = listed("x", &["rm", "--", "-y"], None, Status::Pending);
        assert_eq!(display_tokens(&after_dash), ["rm", "--", "-y"]);

        let out = render_list_at(Theme::plain(), &entries, false, None);
        assert!(!out.contains("--profile"));
        assert!(!out.contains(" -y"));
        assert!(out.contains("│ daily "));
        assert!(out.contains("queue list --full"));
        let full = render_list_at(Theme::plain(), &entries, true, None);
        for entry in &entries {
            assert!(full.contains(&shell_join(&entry.argv)), "{full}");
        }
        assert!(!full.contains("queue list --full"));
    }

    #[test]
    fn queue_list_hides_empty_columns_and_lists_errors_below() {
        let entries = sample_queue();
        let clean = render_list_at(Theme::plain(), &entries, false, Some(200));
        assert!(!clean.contains("error"));
        assert!(clean.contains("│ id       │ pos │ command"));
        assert!(clean.contains("3 entries · 3 pending"));

        let mut broken = entries.clone();
        broken[0].status = Status::Failed;
        broken[0].last_error = Some(INTERRUPTED.to_string());
        broken[1].status = Status::Skipped;
        broken[1].last_error = Some("no workspace matches 1f2e3d4c".to_string());
        broken[2].profile = None;
        let out = render_list_at(Theme::plain(), &broken, false, Some(200));
        assert!(out.contains("│ interrupted │") || out.contains(" interrupted "));
        assert!(out.contains("1 pending · 1 failed · 1 skipped"));
        assert!(out.contains(&format!("\n✖ aaaa1111 {INTERRUPTED}\n")));
        assert!(out.contains("\n⚠ bbbb2222 no workspace matches 1f2e3d4c\n"));
        let rows: Vec<&str> = table_lines(&out);
        assert!(rows.iter().any(|row| row.contains("│   - │")), "{out}");
        assert!(rows.iter().any(|row| row.contains("│ -        │")), "{out}");

        let none_pending: Vec<Entry> = broken[..2].to_vec();
        let out = render_list_at(Theme::plain(), &none_pending, false, Some(200));
        assert!(!out.contains("pos"), "{out}");
        assert!(render_list_at(Theme::plain(), &[], false, Some(80)).contains("Queue is empty."));
    }

    #[test]
    fn store_round_trips_and_assigns_pending_positions() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let (first, pos) = add(
            home,
            vec!["mv".into(), "/a".into(), "/b".into()],
            PathBuf::from("/tmp"),
            Some("workshop".into()),
            vec!["warn".into()],
        )
        .unwrap();
        assert_eq!(pos, 1);
        assert_eq!(first.profile.as_deref(), Some("workshop"));
        assert_eq!(first.warnings, ["warn"]);
        assert_eq!(first.schema, SCHEMA);
        let (second, pos) = add(
            home,
            vec!["rm".into(), "hash".into()],
            PathBuf::from("/tmp"),
            None,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(pos, 2);
        update(home, &first.id, Status::Failed, Some("boom".into())).unwrap();
        let entries = load(home).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, Status::Failed);
        assert_eq!(pending(&entries)[0].id, second.id);
        retry(home, None).unwrap();
        assert_eq!(pending(&load(home).unwrap()).len(), 2);
        clear_all(home).unwrap();
        assert!(load(home).unwrap().is_empty());
    }

    #[test]
    fn lock_blocks_a_second_holder() {
        let temp = tempfile::tempdir().unwrap();
        let lock = Lock::acquire(temp.path()).unwrap().unwrap();
        let holder = Lock::acquire(temp.path()).unwrap().unwrap_err();
        assert_eq!(holder.pid, std::process::id());
        assert!(holder.alive);
        drop(lock);
        assert!(Lock::acquire(temp.path()).unwrap().is_ok());
    }

    #[test]
    fn append_keeps_prior_lines_byte_identical() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        add(
            home,
            vec!["rm".into(), "a".into()],
            PathBuf::from("/tmp"),
            None,
            Vec::new(),
        )
        .unwrap();
        let before = fs::read(queue_path(home)).unwrap();
        assert!(before.ends_with(b"\n"));
        add(
            home,
            vec!["rm".into(), "b".into()],
            PathBuf::from("/tmp"),
            None,
            Vec::new(),
        )
        .unwrap();
        let after = fs::read(queue_path(home)).unwrap();
        assert_eq!(&after[..before.len()], before.as_slice());
        let appended = std::str::from_utf8(&after[before.len()..]).unwrap();
        let entry: Entry = serde_json::from_str(appended.strip_suffix('\n').unwrap()).unwrap();
        assert_eq!(entry.argv, ["rm", "b"]);
        assert_eq!(load(home).unwrap().len(), 2);
    }

    #[test]
    fn append_inserts_a_newline_when_the_file_has_none() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        fs::create_dir_all(home).unwrap();
        let path = queue_path(home);
        fs::write(&path, entry_line("a")).unwrap();
        add(
            home,
            vec!["rm".into(), "b".into()],
            PathBuf::from("/tmp"),
            None,
            Vec::new(),
        )
        .unwrap();
        let entries = load(home).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "a");
        assert_eq!(entries[1].argv, ["rm", "b"]);
    }

    fn entry_line(id: &str) -> String {
        serde_json::json!({
            "schema": SCHEMA,
            "id": id,
            "argv": ["rm", id],
            "cwd": "/tmp",
            "created_at": "2026-01-01T00:00:00Z",
            "tool_version": "1.0.0",
            "status": "pending"
        })
        .to_string()
    }

    #[test]
    fn entries_written_by_crepath_still_load() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let line = entry_line("a").replace("\"tool_version\"", "\"crepath_version\"");
        fs::write(queue_path(home), format!("{line}\n")).unwrap();
        let entries = load(home).unwrap();
        assert_eq!(entries[0].tool_version, "1.0.0");
    }

    #[test]
    fn truncated_last_line_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let path = queue_path(home);
        fs::write(&path, format!("{}\n{{\"id\":\"b", entry_line("a"))).unwrap();
        assert_eq!(
            load(home).unwrap_err().to_string(),
            format!(
                "queue.jsonl line 2 is truncated (incomplete write) in {}: EOF while parsing a string at line 1 column 8",
                path.display()
            )
        );
    }

    #[test]
    fn malformed_middle_line_is_a_hard_error() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let path = queue_path(home);
        fs::write(
            &path,
            format!("{}\n{{not-json\n{}\n", entry_line("a"), entry_line("b")),
        )
        .unwrap();
        assert_eq!(
            load(home).unwrap_err().to_string(),
            format!(
                "queue.jsonl line 2 is malformed in {}: key must be a string at line 1 column 2",
                path.display()
            )
        );
    }

    #[test]
    fn unknown_schema_version_is_a_hard_error() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let line = serde_json::json!({
            "schema": 99,
            "id": "deadbeef",
            "argv": ["rm", "x"],
            "cwd": "/tmp",
            "created_at": "2026-01-01T00:00:00Z",
            "tool_version": "1.0.0",
            "status": "pending"
        });
        fs::create_dir_all(home).unwrap();
        fs::write(queue_path(home), format!("{line}\n")).unwrap();
        assert_eq!(
            load(home).unwrap_err().to_string(),
            format!(
                "unsupported queue schema 99 on line 1 of {} (want {SCHEMA})",
                queue_path(home).display()
            )
        );
    }
}
