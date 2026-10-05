//! Two-way sync of the Claude desktop app's chat lists between accounts.
//!
//! A sync profile names the accounts that share their chats. Inside a profile, every chat one
//! account lists is listed by all of them, and the entry that was changed last (by its file's
//! modification time) decides the title and the rest of what the list shows. Only list
//! entries are copied; the chats themselves live in `~/.claude/projects` and belong to no
//! account.
//!
//! A chat that an account listed at the last sync and no longer lists was deleted there. It
//! is never added back, and it is only removed from the other accounts after the user says so.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::accounts::{self, ACCOUNT_BOUND, Listing};
use super::ops::in_journal;
use super::{Runtime, live, store};
use crate::engine::Report;
use crate::engine::fsops::{exists, write_atomic};
use crate::ui;

/// Words `chatkeep claude sync` takes as actions, so no profile may carry them.
const RESERVED: [&str; 7] = ["profiles", "ls", "list", "set", "rm", "watch", "auto"];

pub fn config_path(home: &Path) -> PathBuf {
    home.join("sync.json")
}

/// One line per change a sync made, newest last.
pub fn log_path(home: &Path) -> PathBuf {
    home.join("sync-log.jsonl")
}

/// The log grows to this size, then keeps its newest part.
const LOG_LIMIT: u64 = 512 * 1024;
const LOG_KEEP: usize = 256 * 1024;

/// What a sync did to one account's list, or a note from the watcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// When, as an RFC 3339 time.
    pub at: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    /// `add`, `update`, `remove`, or `note`.
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chat: String,
    /// The chat's title, or the text of a note.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub account: String,
    /// `manual` for a sync someone ran, `watch` for the watcher.
    pub source: String,
    /// For an update: what differed, e.g. `title`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    /// How often this happened in a row; `at` is the last time.
    #[serde(default = "once", skip_serializing_if = "is_once")]
    pub times: usize,
}

fn once() -> usize {
    1
}

fn is_once(times: &usize) -> bool {
    *times == 1
}

impl LogEntry {
    /// Whether `next` is this same change again: an open chat is saved by the app every few
    /// seconds, and each save is one more update of the same entry.
    fn repeats(&self, next: &LogEntry) -> bool {
        self.action == next.action
            && self.chat == next.chat
            && self.account == next.account
            && self.profile == next.profile
            && self.source == next.source
            && (self.action == "update" || (self.action == "note" && self.title == next.title))
    }

    /// Fold `next` into this entry: the newest time and title, every field that changed.
    fn absorb(&mut self, next: LogEntry) {
        let mut changed = std::mem::take(&mut self.changed);
        for key in &next.changed {
            if !changed.contains(key) {
                changed.push(key.clone());
            }
        }
        let times = self.times + next.times;
        *self = next;
        self.changed = changed;
        self.times = times;
    }
}

/// Add `entries` to the log. A change that repeats the newest line is folded into that line,
/// so hours of the same update do not push everything else out of the log.
fn append_log(home: &Path, entries: &[LogEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(home)?;
    let path = log_path(home);
    let existing = if path.is_file() {
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?
    } else {
        String::new()
    };
    let body = existing.trim_end_matches('\n');
    let (mut out, last_line) = match body.rsplit_once('\n') {
        Some((head, tail)) => (format!("{head}\n"), tail),
        None => (String::new(), body),
    };
    let mut tail: Vec<LogEntry> = Vec::new();
    match serde_json::from_str::<LogEntry>(last_line) {
        Ok(entry) => tail.push(entry),
        Err(_) if !last_line.is_empty() => {
            out.push_str(last_line);
            out.push('\n');
        }
        Err(_) => {}
    }
    for entry in entries {
        match tail.last_mut() {
            Some(last) if last.repeats(entry) => last.absorb(entry.clone()),
            _ => tail.push(entry.clone()),
        }
    }
    for entry in &tail {
        out.push_str(&serde_json::to_string(entry)?);
        out.push('\n');
    }
    write_atomic(&path, out.as_bytes())?;
    crate::engine::index::trim_log(&path, LOG_LIMIT, LOG_KEEP)?;
    // The plain-text log of the first release is superseded by this one.
    let _ = fs::remove_file(home.join("sync.log"));
    Ok(())
}

/// A note from the watcher: that it started, or what it is waiting for.
pub fn log_note(home: &Path, text: &str) -> Result<()> {
    append_log(
        home,
        &[LogEntry {
            at: chrono::Utc::now().to_rfc3339(),
            profile: String::new(),
            action: "note".to_string(),
            chat: String::new(),
            title: text.to_string(),
            account: String::new(),
            source: "watch".to_string(),
            changed: Vec::new(),
            times: 1,
        }],
    )
}

/// Every entry of the log, oldest first. A line cut in half by trimming is skipped.
pub fn read_log(home: &Path) -> Result<Vec<LogEntry>> {
    let path = log_path(home);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok(raw
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// One group of accounts that share their chats.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// `account/organization`, the folders whose lists are kept the same.
    pub accounts: Vec<String>,
    /// Per account, the chats it listed after the last sync. A chat that is in here and no
    /// longer listed was deleted in that account.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub known: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

pub fn load(home: &Path) -> Result<Config> {
    let path = config_path(home);
    if !path.is_file() {
        return Ok(Config::default());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("{} is not valid JSON", path.display()))
}

pub fn save(home: &Path, config: &Config) -> Result<()> {
    fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
    write_atomic(
        &config_path(home),
        serde_json::to_string_pretty(config)?.as_bytes(),
    )
}

fn valid_name(name: &str) -> Result<()> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    if !plain {
        bail!("a profile name uses letters, digits, - and _ only: {name:?}");
    }
    if RESERVED.contains(&name) {
        bail!("{name} is an action of chatkeep claude sync and cannot name a profile");
    }
    Ok(())
}

/// Create or replace the profile `name` with the accounts `specs` name. Each spec is
/// `ACCOUNT` or `ACCOUNT/ORGANIZATION`, an id or the start of one.
pub fn set_profile(rt: &Runtime, name: &str, specs: &[String]) -> Result<Profile> {
    valid_name(name)?;
    let listings = accounts::listings(&rt.layout)?;
    let mut labels: Vec<String> = Vec::new();
    for spec in specs {
        let found = accounts::matching(&listings, spec)?;
        let [listing] = found.as_slice() else {
            bail!(
                "account {spec} has several organizations; pass one of: {}",
                found
                    .iter()
                    .map(|listing| listing.label())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        let label = listing.label();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    if labels.len() < 2 {
        bail!("a sync profile needs at least two different accounts");
    }
    let home = &rt.layout.chatkeep_home;
    let mut config = load(home)?;
    for (other, profile) in &config.profiles {
        if other == name {
            continue;
        }
        if let Some(shared) = labels.iter().find(|label| profile.accounts.contains(label)) {
            bail!("account {shared} already syncs in profile {other}");
        }
    }
    let known = config
        .profiles
        .get(name)
        .map(|old| {
            old.known
                .iter()
                .filter(|(label, _)| labels.contains(label))
                .map(|(label, keys)| (label.clone(), keys.clone()))
                .collect()
        })
        .unwrap_or_default();
    let profile = Profile {
        accounts: labels,
        known,
    };
    if !rt.dry_run {
        config.profiles.insert(name.to_string(), profile.clone());
        save(home, &config)?;
    }
    Ok(profile)
}

/// Remove the profile `name`. Returns whether it existed.
pub fn remove_profile(rt: &Runtime, name: &str) -> Result<bool> {
    let home = &rt.layout.chatkeep_home;
    let mut config = load(home)?;
    let existed = config.profiles.contains_key(name);
    if existed && !rt.dry_run {
        config.profiles.remove(name);
        save(home, &config)?;
    }
    Ok(existed)
}

/// The profiles a run covers: the named one, or all of them.
pub fn selected(config: &Config, name: Option<&str>) -> Result<Vec<(String, Profile)>> {
    match name {
        Some(name) => {
            let profile = config.profiles.get(name).ok_or_else(|| {
                ui::hinted(
                    format!("no sync profile named {name}"),
                    "Run chatkeep claude sync profiles to see the ones that exist.",
                )
            })?;
            Ok(vec![(name.to_string(), profile.clone())])
        }
        None if config.profiles.is_empty() => Err(ui::hinted(
            "no sync profile exists yet",
            "Create one with: chatkeep claude sync set NAME ACCOUNT ACCOUNT (chatkeep claude accounts ls shows the accounts).",
        )),
        None => Ok(config
            .profiles
            .iter()
            .map(|(name, profile)| (name.clone(), profile.clone()))
            .collect()),
    }
}

/// One list entry as it is on disk.
#[derive(Debug, Clone)]
struct Entry {
    file: PathBuf,
    json: Value,
    modified: SystemTime,
}

impl Entry {
    /// What two entries of the same chat have to agree on: everything except the entry's own
    /// name and what belongs to its account.
    fn shared(&self) -> Value {
        let mut json = self.json.clone();
        if let Some(object) = json.as_object_mut() {
            object.shift_remove("sessionId");
            for key in ACCOUNT_BOUND {
                object.shift_remove(key);
            }
        }
        json
    }

    fn title(&self) -> String {
        self.json
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }
}

/// The chat an entry belongs to: its Claude Code session, or the entry's own name for an
/// entry that has none yet.
fn chat_key(file: &Path, json: &Value) -> String {
    json.get("cliSessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            file.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

/// The entries of one account, by chat. When an account lists a chat twice, the entry that
/// changed last stands for it.
fn read_entries(dir: &Path) -> Result<BTreeMap<String, Entry>> {
    let mut found: BTreeMap<String, Entry> = BTreeMap::new();
    for item in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let file = item?.path();
        let is_entry = file
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("local_") && name.ends_with(".json"));
        if !is_entry {
            continue;
        }
        let Ok(raw) = fs::read_to_string(&file) else {
            continue;
        };
        // The app may be writing this entry right now; the next sync picks it up.
        let Ok(json) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if !json.is_object() {
            continue;
        }
        let modified = fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let key = chat_key(&file, &json);
        let entry = Entry {
            file,
            json,
            modified,
        };
        match found.get(&key) {
            Some(existing) if existing.modified >= entry.modified => {}
            _ => {
                found.insert(key, entry);
            }
        }
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Add,
    Update,
    Remove,
}

impl Action {
    pub fn verb(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Update => "update",
            Self::Remove => "remove",
        }
    }
}

/// One change to one account's list.
#[derive(Debug, Clone)]
pub struct Step {
    pub action: Action,
    pub chat: String,
    pub title: String,
    /// `account/organization` whose list changes.
    pub account: String,
    dest: PathBuf,
    /// What the entry holds afterwards, and the time it keeps.
    content: Option<(String, SystemTime)>,
    /// The app is open with this account and would write its own copy back; this step waits.
    pub waits: bool,
    /// For an update: the fields that differ from the entry that wins.
    pub changed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    pub name: String,
    profile: Profile,
    /// Additions and updates.
    pub steps: Vec<Step>,
    /// Entries of chats that were deleted in another account. They only go after a yes.
    pub removals: Vec<Step>,
    /// Chats other accounts list that have no transcript on this machine.
    pub without_transcript: usize,
    listings: Vec<(String, PathBuf)>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.removals.is_empty()
    }

    /// Steps that can run now.
    pub fn ready(&self) -> impl Iterator<Item = &Step> {
        self.steps.iter().filter(|step| !step.waits)
    }

    pub fn waiting(&self) -> usize {
        self.steps
            .iter()
            .chain(&self.removals)
            .filter(|step| step.waits)
            .count()
    }
}

fn short(account: &str) -> &str {
    account.split('/').next().unwrap_or(account)
}

/// The entry `winner` becomes in another account: the same chat, under that account's own
/// entry name, keeping what belongs to that account.
fn merged(winner: &Entry, target: Option<&Entry>, name: &str) -> Result<String> {
    let mut json = winner.json.clone();
    let object = json
        .as_object_mut()
        .context("list entry is not an object")?;
    for key in ACCOUNT_BOUND {
        object.shift_remove(key);
        if let Some(own) = target.and_then(|entry| entry.json.get(key)) {
            object.insert(key.to_string(), own.clone());
        }
    }
    object.insert("sessionId".to_string(), Value::String(name.to_string()));
    Ok(serde_json::to_string(&json)?)
}

/// The fields two entries of one chat disagree on.
fn differing(old: &Value, new: &Value) -> Vec<String> {
    let (Some(old), Some(new)) = (old.as_object(), new.as_object()) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = new
        .iter()
        .filter(|(key, value)| old.get(*key) != Some(value))
        .map(|(key, _)| key.clone())
        .collect();
    keys.extend(old.keys().filter(|key| !new.contains_key(*key)).cloned());
    keys
}

/// Work out what brings the accounts of `profile` in step. Nothing is written.
pub fn plan(rt: &Runtime, name: &str, profile: &Profile) -> Result<Plan> {
    let all = accounts::listings(&rt.layout)?;
    let mut listings: Vec<(String, PathBuf)> = Vec::new();
    for label in &profile.accounts {
        let Some(found) = all
            .iter()
            .find(|listing: &&Listing| &listing.label() == label)
        else {
            return Err(ui::hinted(
                format!("sync profile {name}: account {label} has no chat list on this machine"),
                format!(
                    "Set the profile again with: chatkeep claude sync set {name} ACCOUNT ACCOUNT"
                ),
            ));
        };
        listings.push((label.clone(), found.dir.clone()));
    }
    let processes = rt.processes.processes()?;
    let app_open = !live::desktop_apps(&processes).is_empty();
    let signed_in = accounts::current(&rt.layout);
    let in_use = |account: &str| app_open && signed_in.as_deref() == Some(short(account));

    let entries: Vec<BTreeMap<String, Entry>> = listings
        .iter()
        .map(|(_, dir)| read_entries(dir))
        .collect::<Result<_>>()?;
    let on_disk: HashSet<String> = store::discover(rt)?
        .into_iter()
        .flat_map(|project| project.sessions.into_iter().map(|session| session.id))
        .collect();
    let chats: BTreeSet<&String> = entries.iter().flat_map(|found| found.keys()).collect();
    let empty = BTreeSet::new();
    let mut steps = Vec::new();
    let mut removals = Vec::new();
    let mut without_transcript = 0usize;
    for chat in chats {
        let holders: Vec<usize> = (0..listings.len())
            .filter(|index| entries[*index].contains_key(chat))
            .collect();
        let deleted_somewhere = (0..listings.len()).any(|index| {
            !entries[index].contains_key(chat)
                && profile
                    .known
                    .get(&listings[index].0)
                    .unwrap_or(&empty)
                    .contains(chat)
        });
        if deleted_somewhere {
            for index in holders {
                let entry = &entries[index][chat];
                removals.push(Step {
                    action: Action::Remove,
                    chat: chat.clone(),
                    title: entry.title(),
                    account: listings[index].0.clone(),
                    dest: entry.file.clone(),
                    content: None,
                    waits: in_use(&listings[index].0),
                    changed: Vec::new(),
                });
            }
            continue;
        }
        let Some(winner) = holders
            .iter()
            .map(|index| &entries[*index][chat])
            .max_by(|a, b| {
                a.modified
                    .cmp(&b.modified)
                    .then_with(|| b.file.cmp(&a.file))
            })
        else {
            continue;
        };
        let shared = winner.shared();
        for (index, (account, dir)) in listings.iter().enumerate() {
            match entries[index].get(chat) {
                Some(entry) if entry.shared() == shared => {}
                Some(entry) => {
                    let own = entry
                        .json
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            entry
                                .file
                                .file_stem()
                                .map(|stem| stem.to_string_lossy().to_string())
                        })
                        .unwrap_or_default();
                    steps.push(Step {
                        action: Action::Update,
                        chat: chat.clone(),
                        title: winner.title(),
                        account: account.clone(),
                        dest: entry.file.clone(),
                        content: Some((merged(winner, Some(entry), &own)?, winner.modified)),
                        waits: in_use(account),
                        changed: differing(&entry.shared(), &shared),
                    });
                }
                None => {
                    if !on_disk.contains(chat) {
                        without_transcript += 1;
                        continue;
                    }
                    let mut stem = winner
                        .file
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if stem.is_empty() || exists(&dir.join(format!("{stem}.json"))) {
                        stem = format!("local_{}", uuid::Uuid::new_v4());
                    }
                    steps.push(Step {
                        action: Action::Add,
                        chat: chat.clone(),
                        title: winner.title(),
                        account: account.clone(),
                        dest: dir.join(format!("{stem}.json")),
                        content: Some((merged(winner, None, &stem)?, winner.modified)),
                        waits: false,
                        changed: Vec::new(),
                    });
                }
            }
        }
    }
    Ok(Plan {
        name: name.to_string(),
        profile: profile.clone(),
        steps,
        removals,
        without_transcript,
        listings,
    })
}

fn describe(plan: &Plan, steps: &[&Step], report: &mut Report) {
    let mut counts: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for step in steps {
        *counts
            .entry((step.action.verb(), step.account.as_str()))
            .or_default() += 1;
    }
    for ((verb, account), count) in counts {
        report.applied.push(format!(
            "{verb} {} ({}) -> {}",
            ui::plural(count, "chat", "chats"),
            plan.name,
            short(account)
        ));
    }
}

/// Apply `plan`: every addition and update that does not have to wait, and the removals when
/// `remove` says the user agreed. Remembers what each account lists now, and logs every
/// change under `source`: `manual` or `watch`.
pub fn execute(rt: &Runtime, plan: &Plan, remove: bool, source: &str) -> Result<Report> {
    let mut report = Report::default();
    let mut run: Vec<&Step> = plan.ready().collect();
    if remove {
        run.extend(plan.removals.iter().filter(|step| !step.waits));
    }
    describe(plan, &run, &mut report);
    let waiting = plan.waiting();
    if waiting > 0 {
        report.warnings.push(format!(
            "{} for the account the desktop app is open with; quit the app and sync again",
            ui::plural(waiting, "change waits", "changes wait")
        ));
    }
    let pending: BTreeSet<&str> = plan
        .removals
        .iter()
        .filter(|step| !remove || step.waits)
        .map(|step| step.chat.as_str())
        .collect();
    if !pending.is_empty() && !remove {
        report.warnings.push(format!(
            "{} deleted in one account and still listed in another; run chatkeep claude sync {} in a terminal to decide",
            ui::plural(pending.len(), "chat was", "chats were"),
            plan.name
        ));
    }
    if plan.without_transcript > 0 {
        report.warnings.push(format!(
            "skipped {}: no transcript on this machine, so there is nothing to open",
            ui::plural(plan.without_transcript, "chat", "chats")
        ));
    }
    if rt.dry_run {
        return Ok(report);
    }
    if !run.is_empty() {
        in_journal(rt, "claude-sync", |journal| {
            for step in &run {
                match (&step.action, &step.content) {
                    (Action::Remove, _) => journal.stash(&step.dest)?,
                    (action, Some((text, modified))) => {
                        if *action == Action::Add {
                            journal.created(&step.dest)?;
                        } else {
                            journal.save(&step.dest)?;
                        }
                        let permissions =
                            fs::metadata(&step.dest).ok().map(|meta| meta.permissions());
                        write_atomic(&step.dest, text.as_bytes())?;
                        if let Some(permissions) = permissions {
                            fs::set_permissions(&step.dest, permissions)?;
                        }
                        // The copy is as old as the entry it came from, so the next sync
                        // does not mistake it for a newer change.
                        fs::File::options()
                            .write(true)
                            .open(&step.dest)
                            .and_then(|file| file.set_modified(*modified))
                            .with_context(|| {
                                format!("failed to set the time of {}", step.dest.display())
                            })?;
                    }
                    (_, None) => {}
                }
            }
            Ok(())
        })?;
        let at = chrono::Utc::now().to_rfc3339();
        let entries: Vec<LogEntry> = run
            .iter()
            .map(|step| LogEntry {
                at: at.clone(),
                profile: plan.name.clone(),
                action: step.action.verb().to_string(),
                chat: step.chat.clone(),
                title: step.title.clone(),
                account: step.account.clone(),
                source: source.to_string(),
                changed: step.changed.clone(),
                times: 1,
            })
            .collect();
        // The changes are made; a log that cannot be written must not undo them.
        if let Err(err) = append_log(&rt.layout.chatkeep_home, &entries) {
            ui::warn(&format!("the sync log was not written: {err:#}"));
        }
    }
    remember(rt, plan)?;
    Ok(report)
}

/// Record what every account of the profile lists now. A chat an account deleted stays on
/// record for it while another account still lists that chat, so it is not added back.
fn remember(rt: &Runtime, plan: &Plan) -> Result<()> {
    let now: Vec<BTreeSet<String>> = plan
        .listings
        .iter()
        .map(|(_, dir)| Ok(read_entries(dir)?.into_keys().collect()))
        .collect::<Result<_>>()?;
    let listed: BTreeSet<&String> = now.iter().flatten().collect();
    let mut known = BTreeMap::new();
    for (index, (account, _)) in plan.listings.iter().enumerate() {
        let mut keys = now[index].clone();
        if let Some(before) = plan.profile.known.get(account) {
            keys.extend(before.iter().filter(|key| listed.contains(key)).cloned());
        }
        known.insert(account.clone(), keys);
    }
    let home = &rt.layout.chatkeep_home;
    let mut config = load(home)?;
    let Some(profile) = config.profiles.get_mut(&plan.name) else {
        return Ok(());
    };
    if profile.known != known {
        profile.known = known;
        save(home, &config)?;
    }
    Ok(())
}

/// Everything a change to which means a sync may have work: the list entries of every
/// account in a profile, the profiles themselves, and whether the desktop app is open.
pub fn fingerprint(rt: &Runtime) -> Result<String> {
    let home = &rt.layout.chatkeep_home;
    let config = load(home)?;
    let all = accounts::listings(&rt.layout)?;
    let mut parts = vec![serde_json::to_string(
        &config
            .profiles
            .iter()
            .map(|(name, profile)| (name, &profile.accounts))
            .collect::<Vec<_>>(),
    )?];
    for profile in config.profiles.values() {
        for label in &profile.accounts {
            let Some(listing) = all.iter().find(|listing| &listing.label() == label) else {
                continue;
            };
            let mut files: Vec<String> = fs::read_dir(&listing.dir)
                .map(|items| {
                    items
                        .flatten()
                        .filter_map(|item| {
                            let meta = item.metadata().ok()?;
                            let stamp = meta
                                .modified()
                                .ok()?
                                .duration_since(SystemTime::UNIX_EPOCH)
                                .ok()?
                                .as_millis();
                            Some(format!(
                                "{}:{}:{stamp}",
                                item.file_name().to_string_lossy(),
                                meta.len()
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.sort();
            parts.push(format!("{label}={}", files.join(",")));
        }
    }
    let processes = rt.processes.processes()?;
    parts.push(format!(
        "app={}",
        !live::desktop_apps(&processes).is_empty()
    ));
    Ok(parts.join("\n"))
}

/// One round of the watcher: sync every profile when something changed since `last`.
/// Additions and updates only; removals always wait for a person. Returns what it did, or
/// `None` when nothing changed.
pub fn watch_round(rt: &Runtime, only: Option<&str>, last: &mut String) -> Result<Option<Report>> {
    let seen = fingerprint(rt)?;
    if seen == *last {
        return Ok(None);
    }
    let config = load(&rt.layout.chatkeep_home)?;
    let mut report = Report::default();
    for (name, profile) in selected(&config, only)? {
        let found = plan(rt, &name, &profile)?;
        let done = execute(rt, &found, false, "watch")?;
        report.applied.extend(done.applied);
        report.warnings.extend(done.warnings);
    }
    // The sync itself changed files; what counts as "last seen" is the state after it.
    *last = fingerprint(rt)?;
    Ok(Some(report))
}
