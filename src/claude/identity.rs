//! Who the desktop app's accounts are. The app keeps an account only as an id, so chatkeep
//! learns an account's e-mail address where Claude Code writes it down on this machine, and
//! lets the user give each account a name. Commands then show and accept those instead of ids.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{Layout, Runtime, config};
use crate::engine::fsops::write_atomic;

/// One account as Claude Code wrote it down somewhere: its id, and what else was there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    pub account: String,
    pub organization: Option<String>,
    pub organization_name: Option<String>,
    pub email: Option<String>,
}

/// The account a Claude Code session runs with, from the variables the desktop app starts it
/// with. Only set when chatkeep itself runs inside such a session.
pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Option<Seen> {
    let filled = |key: &str| var(key).filter(|value| !value.is_empty());
    Some(Seen {
        account: filled("CLAUDE_CODE_ACCOUNT_UUID")?,
        organization: filled("CLAUDE_CODE_ORGANIZATION_UUID"),
        organization_name: None,
        email: filled("CLAUDE_CODE_USER_EMAIL"),
    })
}

/// The last login Claude Code saved in `.claude.json`. Any Claude Code writes it, the copy the
/// desktop app runs included, so it is not necessarily the account the app uses now.
pub fn last_login(layout: &Layout) -> Result<Option<Seen>> {
    let Some(json) = config::read(&layout.config_file)? else {
        return Ok(None);
    };
    let Some(login) = json.get("oauthAccount") else {
        return Ok(None);
    };
    let text = |key: &str| {
        login
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    Ok(text("accountUuid").map(|account| Seen {
        account,
        organization: text("organizationUuid"),
        organization_name: text("organizationName"),
        email: text("emailAddress"),
    }))
}

/// What chatkeep remembers about accounts, in `accounts.json` in its home.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Book {
    /// The names the user gave, by account id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, String>,
    /// E-mail addresses seen on this machine, by account id. Kept after the login that showed
    /// them is gone, so an account the app no longer uses still has a face.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub emails: BTreeMap<String, String>,
}

pub fn book_path(home: &Path) -> PathBuf {
    home.join("accounts.json")
}

pub fn load(home: &Path) -> Result<Book> {
    let path = book_path(home);
    if !path.is_file() {
        return Ok(Book::default());
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("{} is not valid JSON", path.display()))
}

fn save(home: &Path, book: &Book) -> Result<()> {
    fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
    write_atomic(
        &book_path(home),
        serde_json::to_string_pretty(book)?.as_bytes(),
    )
}

/// Every account Claude Code wrote down on this machine with its e-mail address.
pub fn sightings(rt: &Runtime) -> Result<Vec<Seen>> {
    let mut found: Vec<Seen> = rt.session_account.iter().cloned().collect();
    found.extend(last_login(&rt.layout)?);
    Ok(found)
}

/// The book, with every e-mail address seen now added. The additions are saved, except in a
/// dry run; a book that cannot be saved still serves this run.
pub fn book(rt: &Runtime) -> Result<Book> {
    let home = &rt.layout.chatkeep_home;
    let mut book = load(home)?;
    let mut learned = false;
    for seen in sightings(rt)? {
        let Some(email) = seen.email else {
            continue;
        };
        if book.emails.get(&seen.account) != Some(&email) {
            book.emails.insert(seen.account, email);
            learned = true;
        }
    }
    if learned && !rt.dry_run {
        save(home, &book)?;
    }
    Ok(book)
}

/// Give `account` the name `name`, or take its name away with `None`.
pub fn set_name(rt: &Runtime, account: &str, name: Option<&str>) -> Result<()> {
    let home = &rt.layout.chatkeep_home;
    let mut book = book(rt)?;
    match name.map(str::trim) {
        Some(name) => {
            valid_name(name)?;
            if let Some((other, _)) = book
                .names
                .iter()
                .find(|(id, taken)| id.as_str() != account && taken.eq_ignore_ascii_case(name))
            {
                bail!("account {other} is already named {name}");
            }
            book.names.insert(account.to_string(), name.to_string());
        }
        None => {
            book.names.remove(account);
        }
    }
    if !rt.dry_run {
        save(home, &book)?;
    }
    Ok(())
}

/// A name stands in for an id on the command line, so it may not look like a path or a pair.
fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("an account name cannot be empty");
    }
    if name.contains('/') || name.chars().any(char::is_control) {
        bail!("an account name cannot hold / or control characters: {name:?}");
    }
    Ok(())
}

impl Book {
    pub fn name(&self, account: &str) -> Option<&str> {
        self.names.get(account).map(String::as_str)
    }

    pub fn email(&self, account: &str) -> Option<&str> {
        self.emails.get(account).map(String::as_str)
    }

    /// How to show `account`: its name, else its e-mail address, else the start of its id.
    pub fn label(&self, account: &str) -> String {
        self.name(account)
            .or_else(|| self.email(account))
            .map(str::to_string)
            .unwrap_or_else(|| short(account).to_string())
    }

    /// The account a name or e-mail address stands for, case-insensitively. `None` when `spec`
    /// is neither, so the caller treats it as an id.
    pub fn resolve(&self, spec: &str) -> Option<String> {
        let wanted = spec.trim();
        self.names
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(wanted))
            .or_else(|| {
                self.emails
                    .iter()
                    .find(|(_, email)| email.eq_ignore_ascii_case(wanted))
            })
            .map(|(account, _)| account.clone())
    }

    /// `spec` with a name or e-mail address before its `/` replaced by the account id.
    pub fn expand(&self, spec: &str) -> String {
        let (account, organization) = match spec.split_once('/') {
            Some((account, organization)) => (account, Some(organization)),
            None => (spec, None),
        };
        match (self.resolve(account), organization) {
            (Some(id), Some(organization)) => format!("{id}/{organization}"),
            (Some(id), None) => id,
            (None, _) => spec.to_string(),
        }
    }
}

/// The first block of an id, enough to tell accounts apart on one machine.
pub fn short(id: &str) -> &str {
    id.split('-').next().unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book() -> Book {
        Book {
            names: BTreeMap::from([("aaaa-1".to_string(), "Work".to_string())]),
            emails: BTreeMap::from([
                ("aaaa-1".to_string(), "me@work.test".to_string()),
                ("bbbb-2".to_string(), "Me@Home.test".to_string()),
            ]),
        }
    }

    #[test]
    fn a_label_prefers_the_name_then_the_address_then_the_id() {
        let book = book();
        assert_eq!(book.label("aaaa-1"), "Work");
        assert_eq!(book.label("bbbb-2"), "Me@Home.test");
        assert_eq!(book.label("cccc-3"), "cccc");
    }

    #[test]
    fn names_and_addresses_resolve_to_ids_in_any_case() {
        let book = book();
        assert_eq!(book.resolve("work").as_deref(), Some("aaaa-1"));
        assert_eq!(book.resolve("me@home.test").as_deref(), Some("bbbb-2"));
        assert_eq!(book.resolve("aaaa"), None);
        assert_eq!(book.expand("Work/org-1"), "aaaa-1/org-1");
        assert_eq!(book.expand("aaaa/org-1"), "aaaa/org-1");
    }

    #[test]
    fn the_environment_names_the_session_account() {
        let env = |key: &str| match key {
            "CLAUDE_CODE_ACCOUNT_UUID" => Some("aaaa-1".to_string()),
            "CLAUDE_CODE_USER_EMAIL" => Some("me@work.test".to_string()),
            "CLAUDE_CODE_ORGANIZATION_UUID" => Some(String::new()),
            _ => None,
        };
        let seen = from_env(env).unwrap();
        assert_eq!(seen.account, "aaaa-1");
        assert_eq!(seen.email.as_deref(), Some("me@work.test"));
        assert_eq!(seen.organization, None);
        assert_eq!(from_env(|_| None), None);
    }
}
