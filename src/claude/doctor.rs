//! Which account each part of Claude Code uses, and the caches `.claude.json` keeps for
//! accounts that are no longer in use.
//!
//! The desktop app passes its own account to every Code session it starts. `.claude.json`
//! names the last account any Claude Code logged in with, so after switching accounts the two
//! can differ without anything being wrong; a session started before the switch, though, keeps
//! the old account until it is restarted. Claude Code also keeps a few caches per account or
//! organization in `.claude.json` and never drops the entries of an account it no longer uses.

use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Color};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use super::accounts::{self, Listing};
use super::identity::{self, Book, Seen};
use super::ops::write_keeping_mode;
use super::{Runtime, config, live};
use crate::engine::Report;
use crate::ui::{self, Theme};

/// What `accounts doctor` looked at.
#[derive(Debug, Clone)]
pub struct Checkup {
    /// Whether the desktop app keeps data on this machine at all.
    pub desktop_app: bool,
    /// The account the desktop app is signed in with.
    pub desktop: Option<String>,
    /// The organizations that account lists chats in.
    pub desktop_organizations: Vec<String>,
    /// The account of the Claude Code session chatkeep runs in.
    pub session: Option<Seen>,
    /// The last login in `.claude.json`.
    pub login: Option<Seen>,
    /// The `claude` command on `PATH`.
    pub cli: Option<PathBuf>,
    /// Cache entries `accounts prune` would remove.
    pub stale: usize,
}

pub fn checkup(rt: &Runtime, cli: Option<PathBuf>) -> Result<Checkup> {
    let listings = accounts::listings(&rt.layout)?;
    let desktop = accounts::current(&rt.layout);
    let stale = match keep(rt, &listings)? {
        Some(keep) => match config::read(&rt.layout.config_file)? {
            Some(json) => stale_entries(&json, &keep).len(),
            None => 0,
        },
        None => 0,
    };
    Ok(Checkup {
        desktop_app: rt.layout.desktop_dir.is_some(),
        desktop_organizations: listings
            .iter()
            .filter(|listing| Some(&listing.account) == desktop.as_ref())
            .map(|listing| listing.organization.clone())
            .collect(),
        desktop,
        session: rt.session_account.clone(),
        login: identity::last_login(&rt.layout)?,
        cli,
        stale,
    })
}

/// The `claude` command the shell would run, from `PATH`.
pub fn cli_on_path() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd"]
    } else {
        &["claude"]
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}

fn who(book: &Book, account: &str, email: Option<&str>) -> String {
    let label = book.label(account);
    match email {
        Some(email) if !label.eq_ignore_ascii_case(email) => {
            format!("{label} <{email}> ({})", identity::short(account))
        }
        _ => format!("{label} ({})", identity::short(account)),
    }
}

fn organization(id: Option<&str>, name: Option<&str>) -> String {
    match (name, id) {
        (Some(name), Some(id)) => format!("{name} ({})", identity::short(id)),
        (None, Some(id)) => identity::short(id).to_string(),
        (Some(name), None) => name.to_string(),
        (None, None) => String::new(),
    }
}

pub fn render(theme: Theme, checkup: &Checkup, book: &Book) -> String {
    let heading = ui::section_line(theme, "Claude accounts on this machine");
    let none = |text: &str| theme.cell(text, None, &[Attribute::Dim]);
    let mut rows = Vec::new();
    rows.push((
        "desktop app",
        match (&checkup.desktop, checkup.desktop_app) {
            (Some(account), _) => {
                let orgs: Vec<&str> = checkup
                    .desktop_organizations
                    .iter()
                    .map(|org| identity::short(org))
                    .collect();
                let mut text = who(book, account, book.email(account));
                if !orgs.is_empty() {
                    text.push_str(&format!(", org {}", orgs.join(", ")));
                }
                theme.cell(text, Some(Color::Magenta), &[Attribute::Bold])
            }
            (None, true) => none("signed in with no account chatkeep can see"),
            (None, false) => none("not on this machine"),
        },
    ));
    if let Some(session) = &checkup.session {
        let mut text = who(book, &session.account, session.email.as_deref());
        let org = organization(session.organization.as_deref(), None);
        if !org.is_empty() {
            text.push_str(&format!(", org {org}"));
        }
        rows.push(("this session", theme.cell(text, Some(Color::Magenta), &[])));
    }
    rows.push((
        "last login",
        match &checkup.login {
            Some(login) => {
                let mut text = who(book, &login.account, login.email.as_deref());
                let org = organization(
                    login.organization.as_deref(),
                    login.organization_name.as_deref(),
                );
                if !org.is_empty() {
                    text.push_str(&format!(", org {org}"));
                }
                theme.cell(text, None, &[])
            }
            None => none("none in .claude.json"),
        },
    ));
    rows.push((
        "claude command",
        match &checkup.cli {
            Some(path) => theme.cell(
                ui::home_relative(&path.display().to_string()),
                Some(Color::Cyan),
                &[],
            ),
            None => none("not on PATH"),
        },
    ));
    rows.push((
        "stale caches",
        if checkup.stale == 0 {
            none("none")
        } else {
            theme.cell(
                ui::plural(checkup.stale, "entry", "entries"),
                Some(Color::Yellow),
                &[],
            )
        },
    ));
    let mut out = format!("{heading}\n{}\n", ui::panel(theme, rows));
    for finding in findings(checkup, book) {
        out.push_str(&finding.line(theme));
        out.push('\n');
    }
    out
}

enum Finding {
    Ok(String),
    Warn(String),
    Info(String),
    Hint(String),
}

impl Finding {
    /// `ok`, `warn`, `info`, or `hint`, with the text.
    fn parts(&self) -> (&'static str, &str) {
        match self {
            Finding::Ok(text) => ("ok", text),
            Finding::Warn(text) => ("warn", text),
            Finding::Info(text) => ("info", text),
            Finding::Hint(text) => ("hint", text),
        }
    }

    fn line(&self, theme: Theme) -> String {
        match self {
            Finding::Ok(text) => ui::ok_line(theme, text),
            Finding::Warn(text) => ui::warn_line(theme, text),
            Finding::Info(text) => ui::info_line(theme, text),
            Finding::Hint(text) => ui::hint_line(theme, text),
        }
    }
}

/// What `accounts doctor` concludes, as `(level, text)`: `ok`, `warn`, `info`, or `hint`.
pub fn conclusions(checkup: &Checkup, book: &Book) -> Vec<(&'static str, String)> {
    findings(checkup, book)
        .iter()
        .map(|finding| {
            let (level, text) = finding.parts();
            (level, text.to_string())
        })
        .collect()
}

fn findings(checkup: &Checkup, book: &Book) -> Vec<Finding> {
    let mut found = Vec::new();
    let desktop = checkup.desktop.as_deref();
    if let (Some(session), Some(desktop)) = (&checkup.session, desktop)
        && session.account != desktop
    {
        found.push(Finding::Warn(format!(
            "This session runs as {}, but the desktop app is signed in as {} now. Start a new session to use {}.",
            book.label(&session.account),
            book.label(desktop),
            book.label(desktop),
        )));
    }
    if let (Some(login), Some(desktop)) = (&checkup.login, desktop)
        && login.account != desktop
    {
        found.push(Finding::Info(format!(
            "The last login in .claude.json is {}, from an earlier Claude Code login. Desktop sessions get the app's account ({}) passed in, so it does not decide which account they use.",
            book.label(&login.account),
            book.label(desktop),
        )));
    }
    if checkup.cli.is_none() && checkup.desktop_app {
        found.push(Finding::Info(
            "No claude command on PATH: only the desktop app's own copy of Claude Code runs here."
                .to_string(),
        ));
    }
    let agree = checkup
        .session
        .as_ref()
        .is_none_or(|session| Some(session.account.as_str()) == desktop);
    if desktop.is_some() && agree {
        found.push(Finding::Ok(
            "Desktop sessions use the account the desktop app is signed in with.".to_string(),
        ));
    }
    if checkup.stale > 0 {
        found.push(Finding::Hint(format!(
            "Remove the cache entries of accounts not in use: chatkeep claude accounts prune ({})",
            ui::plural(checkup.stale, "entry", "entries")
        )));
    }
    found
}

/// The accounts and organizations whose cache entries stay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keep {
    pub accounts: BTreeSet<String>,
    pub organizations: BTreeSet<String>,
}

/// Every account in use: the desktop app's, this session's, and the last login. `None` when
/// chatkeep sees none of them, because then it cannot tell what is stale.
fn keep(rt: &Runtime, listings: &[Listing]) -> Result<Option<Keep>> {
    let mut keep = Keep::default();
    let seen: Vec<Seen> = identity::sightings(rt)?;
    for seen in &seen {
        keep.accounts.insert(seen.account.clone());
        keep.organizations.extend(seen.organization.clone());
    }
    keep.accounts.extend(accounts::current(&rt.layout));
    for listing in listings {
        if keep.accounts.contains(&listing.account) {
            keep.organizations.insert(listing.organization.clone());
        }
    }
    Ok((!keep.accounts.is_empty()).then_some(keep))
}

/// One cache entry of an account or organization that is not in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stale {
    /// The top-level key of `.claude.json`.
    pub cache: &'static str,
    /// The entry inside it, or `None` when the whole cache belongs to that account.
    pub key: Option<String>,
    pub account: Option<String>,
    pub organization: Option<String>,
}

/// How a cache in `.claude.json` says whose an entry is.
#[derive(Debug, Clone, Copy)]
enum Owner {
    /// Keyed by account id.
    AccountKey,
    /// Keyed by organization id.
    OrganizationKey,
    /// Keyed `acct:<account>/<organization>`.
    PrincipalKey,
    /// Each entry holds its organization in `org`.
    OrganizationField,
    /// The whole cache is one account's, named in `accountUuid`.
    Whole,
}

/// The caches Claude Code keeps per account or organization. A key outside this list, or an
/// entry in a shape chatkeep does not know, is always left alone.
const CACHES: [(&str, Owner); 5] = [
    ("groveConfigCache", Owner::AccountKey),
    ("passesEligibilityCache", Owner::OrganizationKey),
    ("metricsStatusCacheByPrincipal", Owner::PrincipalKey),
    ("clientDataCacheSlots", Owner::OrganizationField),
    ("cachedArtifactRoster", Owner::Whole),
];

pub fn stale_entries(json: &Value, keep: &Keep) -> Vec<Stale> {
    let mut found = Vec::new();
    for (cache, owner) in CACHES {
        let Some(value) = json.get(cache) else {
            continue;
        };
        if let Owner::Whole = owner {
            if let Some(account) = value.get("accountUuid").and_then(Value::as_str)
                && !keep.accounts.contains(account)
            {
                found.push(Stale {
                    cache,
                    key: None,
                    account: Some(account.to_string()),
                    organization: value
                        .get("organizationUuid")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
            continue;
        }
        let Some(entries) = value.as_object() else {
            continue;
        };
        for (key, entry) in entries {
            let (account, organization) = match owner {
                Owner::AccountKey => (Some(key.clone()), None),
                Owner::OrganizationKey => (None, Some(key.clone())),
                Owner::PrincipalKey => match key
                    .strip_prefix("acct:")
                    .and_then(|rest| rest.split_once('/'))
                {
                    Some((account, organization)) => {
                        (Some(account.to_string()), Some(organization.to_string()))
                    }
                    None => continue,
                },
                Owner::OrganizationField => match entry.get("org").and_then(Value::as_str) {
                    Some(organization) => (None, Some(organization.to_string())),
                    None => continue,
                },
                Owner::Whole => unreachable!("handled above"),
            };
            let gone = match &account {
                Some(account) => !keep.accounts.contains(account),
                None => organization
                    .as_ref()
                    .is_some_and(|organization| !keep.organizations.contains(organization)),
            };
            if gone {
                found.push(Stale {
                    cache,
                    key: Some(key.clone()),
                    account,
                    organization,
                });
            }
        }
    }
    found
}

fn remove_entries(json: &mut Value, stale: &[Stale]) {
    let Some(root) = json.as_object_mut() else {
        return;
    };
    for entry in stale {
        match &entry.key {
            None => {
                root.shift_remove(entry.cache);
            }
            Some(key) => {
                if let Some(cache) = root.get_mut(entry.cache).and_then(Value::as_object_mut) {
                    cache.shift_remove(key);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrunePlan {
    pub keep: Keep,
    pub stale: Vec<Stale>,
}

pub fn plan_prune(rt: &Runtime) -> Result<PrunePlan> {
    let listings = accounts::listings(&rt.layout)?;
    let Some(keep) = keep(rt, &listings)? else {
        return Err(ui::hinted(
            "chatkeep cannot tell which Claude account is in use, so it keeps every cache entry",
            "Sign in to the desktop app or to Claude Code once, then run this again.",
        ));
    };
    let stale = match config::read(&rt.layout.config_file)? {
        Some(json) => stale_entries(&json, &keep),
        None => Vec::new(),
    };
    Ok(PrunePlan { keep, stale })
}

impl PrunePlan {
    /// One row per entry: the cache and whose entry it is.
    pub fn rows(&self, book: &Book) -> Vec<Vec<String>> {
        self.stale
            .iter()
            .map(|entry| {
                let owner = match (&entry.account, &entry.organization) {
                    (Some(account), _) => format!("account {}", book.label(account)),
                    (None, Some(organization)) => {
                        format!("organization {}", identity::short(organization))
                    }
                    (None, None) => String::new(),
                };
                vec![entry.cache.to_string(), owner]
            })
            .collect()
    }
}

/// Remove the stale entries. `.claude.json` is read again right before the write, so whatever
/// Claude Code saved since the plan stays, and a copy of the file is kept in the backups.
pub fn execute_prune(rt: &Runtime, plan: &PrunePlan) -> Result<Report> {
    let mut report = Report::default();
    if plan.stale.is_empty() {
        return Ok(report);
    }
    let path = &rt.layout.config_file;
    report.applied.push(format!(
        "remove {} from {}",
        ui::plural(plan.stale.len(), "cache entry", "cache entries"),
        ui::home_relative(&path.display().to_string())
    ));
    if rt.dry_run {
        return Ok(report);
    }
    let Some(mut json) = config::read(path)? else {
        bail!("{} is gone", path.display());
    };
    let stale = stale_entries(&json, &plan.keep);
    if stale.is_empty() {
        return Ok(Report::default());
    }
    let backups = rt.layout.backup_root();
    fs::create_dir_all(&backups)
        .with_context(|| format!("failed to create {}", backups.display()))?;
    let backup = backups.join(format!(
        "claude-json-{}.json",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::copy(path, &backup).with_context(|| format!("failed to back up {}", path.display()))?;
    remove_entries(&mut json, &stale);
    write_keeping_mode(path, serde_json::to_string_pretty(&json)?.as_bytes())?;
    report.applied.push(format!(
        "backup {}",
        ui::home_relative(&backup.display().to_string())
    ));
    let processes = rt.processes.processes().unwrap_or_default();
    if !live::sessions(&rt.layout, &processes)?.is_empty()
        || !live::desktop_apps(&processes).is_empty()
    {
        report.warnings.push(
            "Claude Code is running: it may write the entries of the account it runs with back, which is harmless".to_string(),
        );
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keep() -> Keep {
        Keep {
            accounts: BTreeSet::from(["me".to_string()]),
            organizations: BTreeSet::from(["my-org".to_string()]),
        }
    }

    fn config() -> Value {
        json!({
            "oauthAccount": {"accountUuid": "me"},
            "groveConfigCache": {"me": {}, "old": {}},
            "passesEligibilityCache": {"my-org": {}, "old-org": {}},
            "metricsStatusCacheByPrincipal": {"acct:me/my-org": {}, "acct:old/my-org": {}, "other": {}},
            "clientDataCacheSlots": {"a": {"org": "my-org"}, "b": {"org": "old-org"}, "c": {}},
            "cachedArtifactRoster": {"accountUuid": "old", "organizationUuid": "old-org"},
            "projects": {"/old": {}},
        })
    }

    #[test]
    fn only_entries_of_accounts_and_organizations_not_in_use_are_stale() {
        let stale = stale_entries(&config(), &keep());
        let named: Vec<(&str, Option<&str>)> = stale
            .iter()
            .map(|entry| (entry.cache, entry.key.as_deref()))
            .collect();
        assert_eq!(
            named,
            vec![
                ("groveConfigCache", Some("old")),
                ("passesEligibilityCache", Some("old-org")),
                ("metricsStatusCacheByPrincipal", Some("acct:old/my-org")),
                ("clientDataCacheSlots", Some("b")),
                ("cachedArtifactRoster", None),
            ]
        );
    }

    #[test]
    fn removing_keeps_everything_else() {
        let mut json = config();
        let stale = stale_entries(&json, &keep());
        remove_entries(&mut json, &stale);
        assert_eq!(
            json,
            json!({
                "oauthAccount": {"accountUuid": "me"},
                "groveConfigCache": {"me": {}},
                "passesEligibilityCache": {"my-org": {}},
                "metricsStatusCacheByPrincipal": {"acct:me/my-org": {}, "other": {}},
                "clientDataCacheSlots": {"a": {"org": "my-org"}, "c": {}},
                "projects": {"/old": {}},
            })
        );
        assert!(stale_entries(&json, &keep()).is_empty());
    }
}
