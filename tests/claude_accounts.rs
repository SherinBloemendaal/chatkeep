//! Names for desktop app accounts, changing sync profiles one account at a time, the account
//! doctor, and pruning the caches of accounts not in use, on a fake home.

mod common;

use chatkeep::claude::identity::{self, Seen};
use chatkeep::claude::{accounts, doctor, sync};
use chatkeep::cli::{Cli, Tools, dispatch_with};
use chatkeep::ui::Theme;
use clap::Parser;
use common::claude_home::*;
use serde_json::json;
use std::fs;

const A: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const B: &str = "bbbbbbbb-2222-4222-8222-222222222222";
const C: &str = "cccccccc-3333-4333-8333-333333333333";

fn cli(home: &Home, args: &[&str]) -> anyhow::Result<()> {
    let tools = Tools::new(None, Some(home.rt.clone()));
    let argv = std::iter::once("chatkeep").chain(args.iter().copied());
    dispatch_with(Cli::parse_from(argv), &tools)
}

fn error(result: anyhow::Result<()>) -> String {
    format!("{:#}", result.unwrap_err())
}

/// Accounts A, B and C, one organization each, each listing one chat.
fn three_accounts(home: &Home) {
    let app = home.folder("app");
    home.session(&app, SESSION);
    home.desktop_entry(A, "org-a", "one", SESSION, &app);
    home.desktop_entry(B, "org-b", "two", SESSION, &app);
    home.desktop_entry(C, "org-c", "three", SESSION, &app);
    home.signed_in(B);
}

fn profile_accounts(home: &Home, name: &str) -> Vec<String> {
    sync::load(&home.rt.layout.chatkeep_home).unwrap().profiles[name]
        .accounts
        .clone()
}

#[test]
fn accounts_go_by_their_name_or_e_mail_address() {
    let home = Home::new();
    three_accounts(&home);
    home.config(json!({"oauthAccount": {"accountUuid": B, "emailAddress": "me@home.test"}}));
    cli(&home, &["claude", "accounts", "name", "aaaa", "Work"]).unwrap();

    let shown = accounts::render(&home.rt, Theme::plain()).unwrap();
    assert!(shown.contains("Work"), "{shown}");
    assert!(shown.contains("me@home.test"), "{shown}");

    // The address stays known once the login that showed it is gone.
    home.config(json!({}));
    cli(
        &home,
        &["claude", "sync", "set", "pair", "work", "ME@home.test"],
    )
    .unwrap();
    assert_eq!(
        profile_accounts(&home, "pair"),
        vec![format!("{A}/org-a"), format!("{B}/org-b")]
    );

    // A name is unique and can be taken away again.
    assert!(
        error(cli(&home, &["claude", "accounts", "name", "cccc", "work"]))
            .contains("already named")
    );
    cli(&home, &["claude", "accounts", "name", "Work", "--clear"]).unwrap();
    let book = identity::load(&home.rt.layout.chatkeep_home).unwrap();
    assert_eq!(book.name(A), None);
    assert_eq!(book.email(B), Some("me@home.test"));
    assert!(
        error(cli(&home, &["claude", "accounts", "name", "zzzz", "x"])).contains("no account zzzz")
    );
}

#[test]
fn a_profile_takes_and_gives_up_one_account_at_a_time() {
    let home = Home::new();
    three_accounts(&home);
    cli(&home, &["claude", "sync", "set", "work", "aaaa", "bbbb"]).unwrap();
    let mut config = sync::load(&home.rt.layout.chatkeep_home).unwrap();
    let history = std::collections::BTreeSet::from(["chat".to_string()]);
    let profile = config.profiles.get_mut("work").unwrap();
    for label in profile.accounts.clone() {
        profile.known.insert(label, history.clone());
    }
    sync::save(&home.rt.layout.chatkeep_home, &config).unwrap();

    cli(&home, &["claude", "sync", "add", "work", "cccc"]).unwrap();
    assert_eq!(profile_accounts(&home, "work").len(), 3);
    assert!(
        error(cli(&home, &["claude", "sync", "add", "work", "cccc"]))
            .contains("already has every account")
    );
    assert!(
        error(cli(&home, &["claude", "sync", "add", "nope", "cccc"]))
            .contains("no sync profile named nope")
    );

    cli(&home, &["claude", "sync", "rm", "work", "bbbb"]).unwrap();
    assert_eq!(
        profile_accounts(&home, "work"),
        vec![format!("{A}/org-a"), format!("{C}/org-c")]
    );
    // What the profile remembers about an account that stays is kept.
    let config = sync::load(&home.rt.layout.chatkeep_home).unwrap();
    assert_eq!(
        config.profiles["work"].known.get(&format!("{A}/org-a")),
        Some(&history)
    );
    assert!(
        !config.profiles["work"]
            .known
            .contains_key(&format!("{B}/org-b"))
    );

    assert!(
        error(cli(&home, &["claude", "sync", "rm", "work", "bbbb"]))
            .contains("has no account bbbb")
    );
    assert!(
        error(cli(&home, &["claude", "sync", "rm", "work", "aaaa"]))
            .contains("fewer than two accounts")
    );

    // An account that left this machine can still be taken out by its label.
    cli(&home, &["claude", "sync", "add", "work", "bbbb"]).unwrap();
    fs::remove_dir_all(home.rt.layout.desktop_sessions_dir().unwrap().join(C)).unwrap();
    cli(&home, &["claude", "sync", "rm", "work", "cccc"]).unwrap();
    assert_eq!(profile_accounts(&home, "work").len(), 2);

    // Without accounts, rm still removes the whole profile.
    cli(&home, &["claude", "sync", "rm", "work"]).unwrap();
    assert!(
        sync::load(&home.rt.layout.chatkeep_home)
            .unwrap()
            .profiles
            .is_empty()
    );
}

#[test]
fn without_accounts_and_without_a_terminal_set_and_add_explain_themselves() {
    let home = Home::new();
    three_accounts(&home);
    let err = error(cli(&home, &["claude", "sync", "set", "work"]));
    assert!(err.contains("needs the accounts"), "{err}");
    cli(&home, &["claude", "sync", "set", "work", "aaaa", "bbbb"]).unwrap();
    let err = error(cli(&home, &["claude", "sync", "add", "work"]));
    assert!(err.contains("needs the accounts"), "{err}");
    let err = error(cli(&home, &["claude", "accounts", "name"]));
    assert!(err.contains("needs the account to name"), "{err}");
}

#[test]
fn picker_rows_say_who_an_account_is_on_one_row() {
    let home = Home::new();
    three_accounts(&home);
    identity::set_name(&home.rt, A, Some("Work")).unwrap();
    let listings = accounts::listings(&home.rt.layout).unwrap();
    let refs: Vec<&accounts::Listing> = listings.iter().collect();
    let book = identity::load(&home.rt.layout.chatkeep_home).unwrap();
    let rows = accounts::picker_rows(&book, &refs, 80);
    assert!(rows[0].starts_with("Work"), "{rows:?}");
    assert!(
        rows[0].contains("1 chat") && rows[0].contains("Chat one"),
        "{rows:?}"
    );
    assert!(rows[1].starts_with("bbbbbbbb (signed in)"), "{rows:?}");
    for width in [80, 50, 20] {
        for row in accounts::picker_rows(&book, &refs, width) {
            assert!(row.len() <= width - 6, "{width}: {row}");
        }
    }
}

#[test]
fn the_doctor_tells_apart_the_app_the_session_and_the_last_login() {
    let mut home = Home::new();
    three_accounts(&home);
    home.config(json!({
        "oauthAccount": {"accountUuid": A, "emailAddress": "old@work.test", "organizationUuid": "org-a", "organizationName": "Old"},
        "groveConfigCache": {A: {}, C: {}},
    }));
    home.rt.session_account = Some(Seen {
        account: C.to_string(),
        organization: Some("org-c".to_string()),
        organization_name: None,
        email: Some("me@new.test".to_string()),
    });
    let checkup = doctor::checkup(&home.rt, None).unwrap();
    assert_eq!(checkup.desktop.as_deref(), Some(B));
    assert_eq!(checkup.desktop_organizations, vec!["org-b".to_string()]);
    assert_eq!(checkup.stale, 0, "A, B and C are all in use");
    let book = identity::book(&home.rt).unwrap();
    let shown = doctor::render(Theme::plain(), &checkup, &book);
    assert!(
        shown.contains("This session runs as me@new.test"),
        "{shown}"
    );
    assert!(shown.contains("Start a new session"), "{shown}");
    assert!(shown.contains("old@work.test"), "{shown}");
    assert!(
        shown.contains("does not decide which account they use"),
        "{shown}"
    );
    assert!(shown.contains("No claude command on PATH"), "{shown}");

    // Once the session and the app agree, only the old login is left to explain.
    home.rt.session_account = None;
    let checkup = doctor::checkup(&home.rt, Some(home.root.join("bin/claude"))).unwrap();
    assert_eq!(checkup.stale, 1);
    let shown = doctor::render(Theme::plain(), &checkup, &book);
    assert!(!shown.contains("Start a new session"), "{shown}");
    assert!(
        shown.contains("Desktop sessions use the account"),
        "{shown}"
    );
    assert!(shown.contains("chatkeep claude accounts prune"), "{shown}");
}

#[test]
fn prune_drops_only_the_caches_of_accounts_not_in_use() {
    let home = Home::new();
    three_accounts(&home);
    let before = json!({
        "oauthAccount": {"accountUuid": A, "organizationUuid": "org-a"},
        "groveConfigCache": {A: {"on": true}, C: {"on": true}},
        "passesEligibilityCache": {"org-a": {}, "org-b": {}, "org-c": {}},
        "metricsStatusCacheByPrincipal": {format!("acct:{B}/org-b"): {}, format!("acct:{C}/org-c"): {}},
        "clientDataCacheSlots": {"one": {"org": "org-c"}, "two": {"org": "org-b"}},
        "projects": {"/somewhere": {"allowedTools": []}},
    });
    home.config(before.clone());

    cli(&home, &["claude", "accounts", "prune", "-n"]).unwrap();
    assert_eq!(home.read_config(), before);

    cli(&home, &["claude", "accounts", "prune", "-y"]).unwrap();
    assert_eq!(
        home.read_config(),
        json!({
            "oauthAccount": {"accountUuid": A, "organizationUuid": "org-a"},
            "groveConfigCache": {A: {"on": true}},
            "passesEligibilityCache": {"org-a": {}, "org-b": {}},
            "metricsStatusCacheByPrincipal": {format!("acct:{B}/org-b"): {}},
            "clientDataCacheSlots": {"two": {"org": "org-b"}},
            "projects": {"/somewhere": {"allowedTools": []}},
        })
    );
    let backups: Vec<_> = fs::read_dir(home.rt.layout.backup_root())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("claude-json-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(backups[0].path()).unwrap()).unwrap();
    assert_eq!(saved, before);

    // A second run finds nothing left.
    cli(&home, &["claude", "accounts", "prune", "-y"]).unwrap();
    assert!(doctor::plan_prune(&home.rt).unwrap().stale.is_empty());
}

#[test]
fn prune_keeps_everything_when_no_account_is_known_to_be_in_use() {
    let home = Home::new();
    home.config(json!({"groveConfigCache": {A: {}}}));
    let err = error(cli(&home, &["claude", "accounts", "prune", "-y"]));
    assert!(
        err.contains("cannot tell which Claude account is in use"),
        "{err}"
    );
    assert_eq!(home.read_config(), json!({"groveConfigCache": {A: {}}}));
}
