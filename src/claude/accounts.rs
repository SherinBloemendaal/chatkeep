//! The accounts of the Claude desktop app.
//!
//! The app keeps its chat list per account: one `local_<id>.json` per chat in
//! `claude-code-sessions/<account>/<organization>/`. The chat itself, its transcript, lives
//! in `~/.claude/projects` and belongs to no account. So a chat started under one account
//! shows up under another as soon as that account's folder lists it too.

use anyhow::{Context, Result, bail};
use comfy_table::{Attribute, Color};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::desktop::{self, DesktopSession};
use super::identity::{self, Book};
use super::ops::{ensure_dir, in_journal};
use super::store;
use super::{Layout, Runtime, live};
use crate::engine::Report;
use crate::engine::fsops::{exists, write_atomic};
use crate::ui::{self, Align, Sheet, Theme};

/// What an entry holds about the account it was made under. A copy for another account
/// starts without them: the remote session and the connectors belong to the first account.
pub(super) const ACCOUNT_BOUND: [&str; 2] = ["bridgeSessionIds", "remoteMcpServersConfig"];

/// One organization of one account: the folder the app lists its chats in.
#[derive(Debug, Clone)]
pub struct Listing {
    pub account: String,
    pub organization: String,
    pub dir: PathBuf,
    pub sessions: Vec<DesktopSession>,
    /// The account the app was last signed in with.
    pub current: bool,
}

impl Listing {
    pub fn label(&self) -> String {
        format!("{}/{}", self.account, self.organization)
    }

    /// The entry the app wrote last: the chat that was active most recently.
    fn latest(&self) -> Option<&DesktopSession> {
        self.sessions
            .iter()
            .filter(|session| session.modified.is_some())
            .max_by_key(|session| session.modified)
    }
}

/// The account the desktop app was last signed in with, from its `config.json`.
pub fn current(layout: &Layout) -> Option<String> {
    let file = layout.desktop_dir.as_ref()?.join("config.json");
    let json: Value = serde_json::from_str(&fs::read_to_string(file).ok()?).ok()?;
    json.get("lastKnownAccountUuid")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub fn listings(layout: &Layout) -> Result<Vec<Listing>> {
    let Some(root) = layout.desktop_sessions_dir() else {
        return Ok(Vec::new());
    };
    let signed_in = current(layout);
    let sessions = desktop::sessions(layout)?;
    let mut found = Vec::new();
    for account in desktop::subdirs(&root)? {
        for organization in desktop::subdirs(&account)? {
            let account_id = desktop::name_of(&account);
            let organization_id = desktop::name_of(&organization);
            found.push(Listing {
                sessions: sessions
                    .iter()
                    .filter(|session| {
                        session.account == account_id && session.organization == organization_id
                    })
                    .cloned()
                    .collect(),
                current: signed_in.as_deref() == Some(account_id.as_str()),
                account: account_id,
                organization: organization_id,
                dir: organization,
            });
        }
    }
    Ok(found)
}

/// Session ids that still have a transcript on this machine.
fn known_sessions(rt: &Runtime) -> Result<HashSet<String>> {
    Ok(store::discover(rt)?
        .into_iter()
        .flat_map(|project| project.sessions.into_iter().map(|session| session.id))
        .collect())
}

fn on_disk(listing: &Listing, known: &HashSet<String>) -> usize {
    listing
        .sessions
        .iter()
        .filter(|session| {
            session
                .cli_session
                .as_ref()
                .is_some_and(|id| known.contains(id))
        })
        .count()
}

pub fn render(rt: &Runtime, theme: Theme) -> Result<String> {
    let listings = listings(&rt.layout)?;
    let known = known_sessions(rt)?;
    let book = identity::book(rt)?;
    Ok(render_at(
        theme,
        &listings,
        &book,
        &known,
        ui::table_width(),
    ))
}

pub fn render_at(
    theme: Theme,
    listings: &[Listing],
    book: &Book,
    known: &HashSet<String>,
    width: Option<usize>,
) -> String {
    let heading = ui::section_line(theme, "Claude desktop app accounts");
    if listings.is_empty() {
        return format!(
            "{heading}\n{}\n",
            ui::info_line(
                theme,
                "The Claude desktop app lists no chats on this machine."
            )
        );
    }
    let mut sheet = Sheet::new(
        theme,
        &[
            ("account", Align::Left),
            ("name", Align::Left),
            ("organization", Align::Left),
            ("chats", Align::Right),
            ("on disk", Align::Right),
            ("state", Align::Left),
        ],
    )
    .optional(&[2])
    .at(width);
    for listing in listings {
        let state = if listing.current {
            theme.cell("signed in", Some(Color::Green), &[Attribute::Bold])
        } else {
            theme.cell("", None, &[])
        };
        sheet.row(vec![
            theme.cell(&listing.account, Some(Color::Magenta), &[]),
            theme.cell(
                book.name(&listing.account)
                    .or_else(|| book.email(&listing.account))
                    .unwrap_or_default(),
                None,
                &[],
            ),
            theme.cell(&listing.organization, None, &[Attribute::Dim]),
            theme.count_cell(listing.sessions.len(), None),
            theme.count_cell(on_disk(listing, known), Some(Color::Cyan)),
            state,
        ]);
    }
    let unnamed = listings.iter().any(|listing| {
        book.name(&listing.account).is_none() && book.email(&listing.account).is_none()
    });
    let mut out = format!(
        "{heading}\n{sheet}\n{}\n",
        ui::hint_line(
            theme,
            "on disk: chats whose transcript is still on this machine, the ones accounts cp can copy"
        )
    );
    if unnamed {
        out.push_str(&ui::hint_line(
            theme,
            "Name an account to use that name instead of its id: chatkeep claude accounts name",
        ));
        out.push('\n');
    }
    out
}

/// One picker row per listing, each on one terminal row of `total` columns: who the account
/// is, its organization, how many chats it lists, when one was last active, and the title of
/// that chat, so an account is recognisable without its id.
pub fn picker_rows(book: &Book, listings: &[&Listing], total: usize) -> Vec<String> {
    let now = SystemTime::now();
    let labels: Vec<String> = listings
        .iter()
        .map(|listing| {
            let mut label = book.label(&listing.account);
            if listing.current {
                label.push_str(" (signed in)");
            }
            label
        })
        .collect();
    let several_orgs = |account: &str| {
        listings
            .iter()
            .filter(|listing| listing.account == account)
            .count()
            > 1
    };
    let width = labels.iter().map(String::len).max().unwrap_or(0).min(40);
    listings
        .iter()
        .zip(&labels)
        .map(|(listing, label)| {
            let mut lead = format!("{label:<width$}  ");
            if several_orgs(&listing.account) {
                lead.push_str(&format!("org {}  ", identity::short(&listing.organization)));
            }
            lead.push_str(&format!(
                "{:>9}",
                ui::plural(listing.sessions.len(), "chat", "chats")
            ));
            let latest = listing.latest();
            if let Some(at) = latest.and_then(|session| session.modified) {
                lead.push_str(&format!("  {:>8}", ago(at, now)));
            }
            let title = latest
                .and_then(|session| session.title.as_deref())
                .unwrap_or_default();
            fit_row(&lead, title, total)
        })
        .collect()
}

fn ago(at: SystemTime, now: SystemTime) -> String {
    let millis = |time: SystemTime| {
        time.duration_since(UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    };
    ui::ago(millis(at), millis(now))
}

/// `lead` and as much of `title` as fits `total` columns, counted in bytes the way the picker
/// counts them, cut on a character boundary.
fn fit_row(lead: &str, title: &str, total: usize) -> String {
    let budget = total.saturating_sub(6);
    if title.is_empty() || lead.len() + 2 >= budget {
        return lead
            .chars()
            .scan(0, |used, ch| {
                *used += ch.len_utf8();
                (*used <= budget.max(1)).then_some(ch)
            })
            .collect::<String>()
            .trim_end()
            .to_string();
    }
    let room = budget - lead.len() - 2;
    let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut = String::new();
    for ch in title.chars() {
        if cut.len() + ch.len_utf8() > room {
            cut.truncate(cut.trim_end().len());
            if cut.len() + 3 <= room {
                cut.push_str("...");
            }
            break;
        }
        cut.push(ch);
    }
    format!("{lead}  {cut}")
}

/// The one listing `spec` names, which may be a name or e-mail address the book knows.
pub(super) fn single<'a>(listings: &'a [Listing], book: &Book, spec: &str) -> Result<&'a Listing> {
    let found = matching(listings, &book.expand(spec))?;
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
    Ok(listing)
}

/// The listings `spec` names: `ACCOUNT` or `ACCOUNT/ORGANIZATION`, each an id or the start of
/// one.
pub(super) fn matching<'a>(listings: &'a [Listing], spec: &str) -> Result<Vec<&'a Listing>> {
    let (account, organization) = match spec.split_once('/') {
        Some((account, organization)) => (account, Some(organization)),
        None => (spec, None),
    };
    if account.is_empty() {
        bail!("an account id is required");
    }
    let accounts: HashSet<&str> = listings
        .iter()
        .filter(|listing| listing.account.starts_with(account))
        .map(|listing| listing.account.as_str())
        .collect();
    if accounts.len() > 1 {
        let mut names: Vec<&str> = accounts.into_iter().collect();
        names.sort();
        bail!("{account} matches several accounts: {}", names.join(", "));
    }
    let found: Vec<&Listing> = listings
        .iter()
        .filter(|listing| {
            listing.account.starts_with(account)
                && organization.is_none_or(|wanted| listing.organization.starts_with(wanted))
        })
        .collect();
    if found.is_empty() {
        return Err(ui::hinted(
            format!("the desktop app has no account {spec} on this machine"),
            "Run chatkeep claude accounts ls to see the accounts it has.",
        ));
    }
    Ok(found)
}

#[derive(Debug, Clone)]
pub struct CopyPlan {
    pub from: Vec<Listing>,
    pub to: Listing,
    /// Entries to copy, with the file each becomes.
    items: Vec<(DesktopSession, PathBuf)>,
}

pub fn plan_copy(rt: &Runtime, from: &str, to: Option<&str>) -> Result<(CopyPlan, Report)> {
    let listings = listings(&rt.layout)?;
    let book = identity::book(rt)?;
    let from = book.expand(from);
    let from = from.as_str();
    let to_spec = match to {
        Some(to) => book.expand(to),
        None => current(&rt.layout).ok_or_else(|| {
            ui::hinted(
                "chatkeep cannot tell which account the desktop app is signed in with",
                "Pass the account that should list the chats as the second argument.",
            )
        })?,
    };
    let targets = matching(&listings, &to_spec).map_err(|err| {
        if to.is_some() {
            err
        } else {
            ui::hinted(
                format!("the account signed in now ({to_spec}) has no chat list yet"),
                "Open the Code tab of the desktop app once with that account, then run this again.",
            )
        }
    })?;
    let [target] = targets.as_slice() else {
        bail!(
            "account {to_spec} has several organizations; pass one of: {}",
            targets
                .iter()
                .map(|listing| listing.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
    };
    let sources: Vec<Listing> = matching(&listings, from)?
        .into_iter()
        .filter(|listing| listing.dir != target.dir)
        .cloned()
        .collect();
    if sources.is_empty() {
        bail!("{from} is the account the chats would be copied to");
    }
    let known = known_sessions(rt)?;
    let mut listed: HashSet<String> = target
        .sessions
        .iter()
        .filter_map(|session| session.cli_session.clone())
        .collect();
    let mut report = Report::default();
    let mut items = Vec::new();
    let mut without_transcript = 0usize;
    let mut already = 0usize;
    for source in &sources {
        for session in &source.sessions {
            let Some(id) = &session.cli_session else {
                without_transcript += 1;
                continue;
            };
            if !known.contains(id) {
                without_transcript += 1;
                continue;
            }
            let name = session
                .file
                .file_name()
                .context("desktop session file has no name")?;
            let dest = target.dir.join(name);
            if exists(&dest) || !listed.insert(id.clone()) {
                already += 1;
                continue;
            }
            items.push((session.clone(), dest));
        }
    }
    if without_transcript > 0 {
        report.warnings.push(format!(
            "skipped {}: no transcript on this machine, so there is nothing to open",
            ui::plural(without_transcript, "chat", "chats")
        ));
    }
    if already > 0 {
        report.warnings.push(format!(
            "skipped {}: account {} already lists them",
            ui::plural(already, "chat", "chats"),
            target.account
        ));
    }
    Ok((
        CopyPlan {
            from: sources,
            to: (*target).clone(),
            items,
        },
        report,
    ))
}

impl CopyPlan {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// One row per chat: its title and the folder it runs in.
    pub fn rows(&self) -> Vec<Vec<String>> {
        self.items
            .iter()
            .map(|(session, _)| {
                vec![
                    session.title.clone().unwrap_or_default(),
                    session
                        .cwd
                        .as_deref()
                        .map(ui::home_relative)
                        .unwrap_or_default(),
                ]
            })
            .collect()
    }
}

pub fn execute_copy(rt: &Runtime, plan: &CopyPlan, mut report: Report) -> Result<Report> {
    if plan.items.is_empty() {
        return Ok(report);
    }
    let accounts: Vec<&str> = {
        let mut names: Vec<&str> = plan
            .from
            .iter()
            .map(|listing| listing.account.as_str())
            .collect();
        names.dedup();
        names
    };
    report.applied.push(format!(
        "copy {} {} -> {}",
        ui::plural(plan.items.len(), "chat", "chats"),
        accounts.join(", "),
        plan.to.account
    ));
    if rt.dry_run {
        return Ok(report);
    }
    in_journal(rt, "claude-accounts-cp", |journal| {
        ensure_dir(journal, &plan.to.dir)?;
        for (session, dest) in &plan.items {
            let raw = fs::read_to_string(&session.file)
                .with_context(|| format!("failed to read {}", session.file.display()))?;
            let mut json: Value = serde_json::from_str(&raw)
                .with_context(|| format!("{} is not valid JSON", session.file.display()))?;
            if let Some(object) = json.as_object_mut() {
                for key in ACCOUNT_BOUND {
                    object.shift_remove(key);
                }
            }
            journal.created(dest)?;
            write_atomic(dest, serde_json::to_string(&json)?.as_bytes())?;
            if let Ok(meta) = fs::metadata(&session.file) {
                fs::set_permissions(dest, meta.permissions())?;
            }
        }
        Ok(())
    })?;
    let processes = rt.processes.processes()?;
    if !live::desktop_apps(&processes).is_empty() {
        report
            .warnings
            .push("restart the Claude desktop app to see the copied chats".to_string());
    }
    Ok(report)
}
