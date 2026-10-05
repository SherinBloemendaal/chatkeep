//! Two-way sync of the desktop app's chat lists between accounts, on a fake home.

mod common;

use chatkeep::claude::sync;
use chatkeep::cli::{Cli, Tools, dispatch_with, help_request};
use clap::Parser;
use common::claude_home::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const A: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const B: &str = "bbbbbbbb-2222-4222-8222-222222222222";
const C: &str = "cccccccc-3333-4333-8333-333333333333";
const THIRD: &str = "33333333-3333-4333-8333-333333333333";
const DESKTOP_APP: &str = "/Applications/Claude.app/Contents/MacOS/Claude";

fn cli(home: &Home, args: &[&str]) -> anyhow::Result<()> {
    let tools = Tools::new(None, Some(home.rt.clone()));
    let argv = std::iter::once("chatkeep").chain(args.iter().copied());
    dispatch_with(Cli::parse_from(argv), &tools)
}

/// Set the title of an entry and the time it was changed, in seconds since 1970.
fn retitle(file: &Path, title: &str, at: u64) {
    let mut json: Value = serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
    json["title"] = json!(title);
    fs::write(file, json.to_string()).unwrap();
    touch(file, at);
}

fn touch(file: &Path, at: u64) {
    fs::File::options()
        .write(true)
        .open(file)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(at))
        .unwrap();
}

fn titles(home: &Home, account: &str, org: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = home
        .desktop_entries(account, org)
        .iter()
        .map(|entry| {
            (
                entry["cliSessionId"].as_str().unwrap().to_string(),
                entry["title"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    found.sort();
    found
}

/// Accounts A and B in profile `work`: A lists two chats, B lists one of them and one more.
fn two_accounts(home: &Home) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let app = home.folder("app");
    for id in [SESSION, OTHER, THIRD] {
        home.session(&app, id);
    }
    let a_one = home.desktop_entry(A, "org-a", "one", SESSION, &app);
    let a_two = home.desktop_entry(A, "org-a", "two", OTHER, &app);
    let b_two = home.desktop_entry(B, "org-b", "two", OTHER, &app);
    let b_three = home.desktop_entry(B, "org-b", "three", THIRD, &app);
    for (file, at) in [(&a_one, 100), (&a_two, 100), (&b_two, 100), (&b_three, 100)] {
        touch(file, at);
    }
    home.signed_in(B);
    sync::set_profile(&home.rt, "work", &["aaaa".to_string(), "bbbb".to_string()]).unwrap();
    (a_one, a_two, b_two, b_three)
}

fn run(home: &Home, remove: bool) -> chatkeep::engine::Report {
    let config = sync::load(&home.rt.layout.chatkeep_home).unwrap();
    let profile = &config.profiles["work"];
    let plan = sync::plan(&home.rt, "work", profile).unwrap();
    sync::execute(&home.rt, &plan, remove, "manual").unwrap()
}

#[test]
fn sync_gives_every_account_every_chat() {
    let home = Home::new();
    two_accounts(&home);
    let report = run(&home, false);
    assert_eq!(report.applied.len(), 2, "{:?}", report.applied);
    let all = vec![
        (SESSION.to_string(), "Chat one".to_string()),
        (OTHER.to_string(), "Chat two".to_string()),
        (THIRD.to_string(), "Chat three".to_string()),
    ];
    let mut expected = all.clone();
    expected.sort();
    assert_eq!(titles(&home, A, "org-a"), expected);
    assert_eq!(titles(&home, B, "org-b"), expected);
    // What an account owns is not handed to the other one.
    let added = home
        .desktop_entries(B, "org-b")
        .into_iter()
        .find(|entry| entry["cliSessionId"] == json!(SESSION))
        .unwrap();
    assert!(added.get("bridgeSessionIds").is_none());
    assert!(added.get("remoteMcpServersConfig").is_none());

    // In step now: a second run has nothing to do.
    let again = run(&home, false);
    assert!(again.applied.is_empty(), "{:?}", again.applied);
}

#[test]
fn the_entry_that_changed_last_decides_the_title() {
    let home = Home::new();
    let (_, a_two, b_two, _) = two_accounts(&home);
    run(&home, false);
    retitle(&a_two, "Renamed in A", 200);
    retitle(&b_two, "Renamed in B, later", 300);
    run(&home, false);
    let title = |account: &str, org: &str| {
        titles(&home, account, org)
            .into_iter()
            .find(|(chat, _)| chat == OTHER)
            .unwrap()
            .1
    };
    assert_eq!(title(A, "org-a"), "Renamed in B, later");
    assert_eq!(title(B, "org-b"), "Renamed in B, later");
    // The updated entry keeps its own name and what its account owns.
    let updated: Value = serde_json::from_str(&fs::read_to_string(&a_two).unwrap()).unwrap();
    assert_eq!(updated["sessionId"], json!("local_two"));
    assert_eq!(updated["bridgeSessionIds"], json!(["session_remote"]));
    // And the time of the entry it came from, so it does not count as a newer change.
    assert_eq!(
        fs::metadata(&a_two).unwrap().modified().unwrap(),
        SystemTime::UNIX_EPOCH + Duration::from_secs(300)
    );
    assert!(run(&home, false).applied.is_empty());
}

#[test]
fn a_deleted_chat_is_never_added_back_and_only_removed_after_a_yes() {
    let home = Home::new();
    let (a_one, ..) = two_accounts(&home);
    run(&home, false);
    fs::remove_file(&a_one).unwrap();

    // Without a yes: A does not get it back, B keeps it, and the report says it waits.
    let report = run(&home, false);
    assert!(report.applied.is_empty(), "{:?}", report.applied);
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("1 chat was deleted in one account")),
        "{:?}",
        report.warnings
    );
    assert_eq!(titles(&home, A, "org-a").len(), 2);
    assert_eq!(titles(&home, B, "org-b").len(), 3);
    // Still waiting on the next run.
    assert!(run(&home, false).applied.is_empty());

    // With a yes it leaves B too, and stays gone.
    let report = run(&home, true);
    assert_eq!(report.applied.len(), 1, "{:?}", report.applied);
    assert!(report.applied[0].starts_with("remove 1 chat"));
    assert_eq!(titles(&home, B, "org-b").len(), 2);
    assert!(run(&home, false).applied.is_empty());
    assert_eq!(titles(&home, A, "org-a").len(), 2);
    // The transcript itself is untouched.
    assert!(
        home.session_ids(&home.root.join("app"))
            .contains(&SESSION.to_string())
    );
}

#[test]
fn changes_to_the_account_the_app_is_open_with_wait() {
    let mut home = Home::new();
    let (_, a_two, b_two, _) = two_accounts(&home);
    run(&home, false);
    home.running(vec![process(7, DESKTOP_APP)]);
    retitle(&a_two, "Renamed in A", 500);

    // B is signed in and the app is open: its entry is left alone for now.
    let report = run(&home, false);
    assert!(report.applied.is_empty(), "{:?}", report.applied);
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("1 change waits"))
    );
    let waiting: Value = serde_json::from_str(&fs::read_to_string(&b_two).unwrap()).unwrap();
    assert_eq!(waiting["title"], json!("Chat two"));

    // A new chat can still be added to it: the app only has to be restarted to show it.
    let app = home.root.join("app");
    home.session(&app, "44444444-4444-4444-8444-444444444444");
    home.desktop_entry(
        A,
        "org-a",
        "four",
        "44444444-4444-4444-8444-444444444444",
        &app,
    );
    let report = run(&home, false);
    assert_eq!(report.applied.len(), 1, "{:?}", report.applied);
    assert_eq!(titles(&home, B, "org-b").len(), 4);

    // Once the app is closed, the update lands.
    home.running(Vec::new());
    run(&home, false);
    let updated: Value = serde_json::from_str(&fs::read_to_string(&b_two).unwrap()).unwrap();
    assert_eq!(updated["title"], json!("Renamed in A"));
}

#[test]
fn chats_without_a_transcript_are_not_handed_on() {
    let home = Home::new();
    two_accounts(&home);
    home.desktop_entry(
        A,
        "org-a",
        "ghost",
        "99999999-9999-4999-8999-999999999999",
        Path::new("/gone"),
    );
    let report = run(&home, false);
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("skipped 1 chat: no transcript"))
    );
    assert_eq!(titles(&home, B, "org-b").len(), 3);
}

#[test]
fn a_dry_run_changes_nothing_and_remembers_nothing() {
    let mut home = Home::new();
    two_accounts(&home);
    home.rt.dry_run = true;
    let before = home.snapshot();
    let state = fs::read(sync::config_path(&home.rt.layout.chatkeep_home)).unwrap();
    let report = run(&home, false);
    assert_eq!(report.applied.len(), 2);
    assert!(home.snapshot() == before, "the dry run changed files");
    assert_eq!(
        fs::read(sync::config_path(&home.rt.layout.chatkeep_home)).unwrap(),
        state
    );
}

#[test]
fn profiles_keep_groups_of_accounts_apart() {
    let home = Home::new();
    two_accounts(&home);
    let app = home.root.join("app");
    home.desktop_entry(C, "org-c", "private", THIRD, &app);
    home.desktop_entry(C, "org-other", "x", THIRD, &app);
    let set = |name: &str, specs: &[&str]| {
        let specs: Vec<String> = specs.iter().map(|spec| spec.to_string()).collect();
        sync::set_profile(&home.rt, name, &specs).map_err(|err| format!("{err:#}"))
    };
    assert!(
        set("solo", &["aaaa"])
            .unwrap_err()
            .contains("at least two different accounts")
    );
    assert!(
        set("solo", &["aaaa", "aaaa"])
            .unwrap_err()
            .contains("at least two")
    );
    assert!(
        set("watch", &["aaaa", "bbbb"])
            .unwrap_err()
            .contains("cannot name a profile")
    );
    assert!(
        set("bad name", &["aaaa", "bbbb"])
            .unwrap_err()
            .contains("letters, digits")
    );
    assert!(
        set("x", &["aaaa", "zzzz"])
            .unwrap_err()
            .contains("has no account zzzz")
    );
    assert!(
        set("x", &["cccc", "aaaa"])
            .unwrap_err()
            .contains("several organizations")
    );
    // An account syncs in one profile only.
    assert!(
        set("private", &["aaaa", "cccc/org-c"])
            .unwrap_err()
            .contains("already syncs in profile work")
    );

    // Only the accounts of the profile are touched.
    run(&home, false);
    assert_eq!(home.desktop_entries(C, "org-c").len(), 1);

    assert!(sync::remove_profile(&home.rt, "work").unwrap());
    assert!(!sync::remove_profile(&home.rt, "work").unwrap());
    let config = sync::load(&home.rt.layout.chatkeep_home).unwrap();
    let err = sync::selected(&config, None).unwrap_err();
    assert!(format!("{err:#}").contains("no sync profile exists yet"));
}

#[test]
fn the_watcher_syncs_when_a_list_changes_and_only_then() {
    let home = Home::new();
    let (_, a_two, ..) = two_accounts(&home);
    let mut last = String::new();
    let first = sync::watch_round(&home.rt, None, &mut last)
        .unwrap()
        .unwrap();
    assert_eq!(first.applied.len(), 2);
    assert!(
        sync::watch_round(&home.rt, None, &mut last)
            .unwrap()
            .is_none()
    );

    retitle(&a_two, "Renamed", 900);
    let after = sync::watch_round(&home.rt, None, &mut last)
        .unwrap()
        .unwrap();
    assert_eq!(after.applied.len(), 1, "{:?}", after.applied);
    assert!(
        sync::watch_round(&home.rt, None, &mut last)
            .unwrap()
            .is_none()
    );

    // The watcher never removes: a deletion waits for a person.
    fs::remove_file(&a_two).unwrap();
    let pending = sync::watch_round(&home.rt, None, &mut last)
        .unwrap()
        .unwrap();
    assert!(pending.applied.is_empty());
    assert!(
        pending
            .warnings
            .iter()
            .any(|line| line.contains("deleted in one account"))
    );
    assert_eq!(titles(&home, B, "org-b").len(), 3);
}

#[test]
fn the_command_line_sets_up_and_runs_a_sync() {
    let home = Home::new();
    two_accounts(&home);
    sync::remove_profile(&home.rt, "work").unwrap();
    let err = cli(&home, &["claude", "sync"]).unwrap_err();
    assert!(format!("{err:#}").contains("no sync profile exists yet"));

    cli(&home, &["claude", "sync", "set", "work", "aaaa", "bbbb"]).unwrap();
    cli(&home, &["claude", "sync", "profiles"]).unwrap();
    cli(&home, &["claude", "sync", "-n"]).unwrap();
    assert_eq!(titles(&home, A, "org-a").len(), 2);
    cli(&home, &["claude", "sync", "work"]).unwrap();
    assert_eq!(titles(&home, A, "org-a").len(), 3);
    assert_eq!(titles(&home, B, "org-b").len(), 3);
    let err = cli(&home, &["claude", "sync", "nope"]).unwrap_err();
    assert!(format!("{err:#}").contains("no sync profile named nope"));
    cli(&home, &["claude", "sync", "rm", "work"]).unwrap();

    // `claude sync` runs on its own; only its action groups ask for an action.
    let args = |words: &[&str]| -> Vec<String> { words.iter().map(|w| w.to_string()).collect() };
    assert_eq!(help_request(&args(&["claude", "sync"])), None);
    assert_eq!(
        help_request(&args(&["claude", "sync", "auto"])),
        Some(args(&["claude", "sync", "auto"]))
    );
    assert_eq!(help_request(&args(&["claude"])), Some(args(&["claude"])));
}

#[test]
fn the_sync_log_says_which_chat_changed_how_and_folds_repeats() {
    let home = Home::new();
    let (_, a_two, ..) = two_accounts(&home);
    let state = home.rt.layout.chatkeep_home.clone();
    assert!(sync::read_log(&state).unwrap().is_empty());

    run(&home, false);
    let entries = sync::read_log(&state).unwrap();
    assert_eq!(entries.len(), 2);
    let added = entries.iter().find(|entry| entry.chat == SESSION).unwrap();
    assert_eq!(
        (
            added.action.as_str(),
            added.title.as_str(),
            added.source.as_str()
        ),
        ("add", "Chat one", "manual")
    );
    assert!(added.account.starts_with(B));
    assert_eq!(added.profile, "work");

    // The watcher renames the same chat three times in a row: one row, counted.
    let mut last = sync::fingerprint(&home.rt).unwrap();
    for (title, at) in [("First", 200), ("Second", 300), ("Third", 400)] {
        retitle(&a_two, title, at);
        sync::watch_round(&home.rt, None, &mut last)
            .unwrap()
            .unwrap();
    }
    sync::log_note(&state, "waiting: something").unwrap();
    sync::log_note(&state, "waiting: something").unwrap();
    let entries = sync::read_log(&state).unwrap();
    assert_eq!(entries.len(), 4, "{entries:?}");
    let renamed = &entries[2];
    assert_eq!(renamed.times, 3);
    assert_eq!(renamed.action, "update");
    assert_eq!(renamed.title, "Third");
    assert_eq!(renamed.source, "watch");
    assert_eq!(renamed.changed, ["title"]);
    assert_eq!((entries[3].action.as_str(), entries[3].times), ("note", 2));
    // Two different chats added in a row stay two lines.
    assert_eq!((entries[0].times, entries[1].times), (1, 1));

    // A half-written line is skipped.
    let before = fs::read_to_string(sync::log_path(&state)).unwrap();
    fs::write(sync::log_path(&state), format!("{{\"cut\n{before}")).unwrap();
    assert_eq!(sync::read_log(&state).unwrap().len(), 4);
    cli(&home, &["claude", "sync", "log"]).unwrap();
    cli(&home, &["claude", "sync", "log", "--last", "2"]).unwrap();
}
