//! Claude Code support on a fake `~/.claude`, `~/.claude.json`, and desktop app folder.

mod common;

use chatkeep::claude::{ops, slug::project_slug, store};
use chatkeep::cli::{Cli, Tools, dispatch_with};
use clap::Parser;
use common::claude_home::*;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[test]
fn ls_finds_the_folder_from_the_settings_and_the_transcripts() {
    let home = Home::new();
    let app = home.folder("app");
    let gone = home.root.join("gone");
    home.session(&app, SESSION);
    home.session(&gone, OTHER);
    home.memory(&app, "MEMORY.md", "- one\n");
    home.config(json!({"projects": {app.to_string_lossy(): {}}}));

    let projects = store::discover(&home.rt).unwrap();
    assert_eq!(projects.len(), 2);
    let found = store::find(&projects, &home.rt, &app.to_string_lossy()).unwrap();
    assert_eq!(found.path.as_deref(), Some(app.as_path()));
    assert_eq!(found.sessions.len(), 1);
    assert_eq!(found.subagents(), 1);
    assert_eq!(found.memory_files, 1);
    assert!(!found.destination_missing());
    assert_eq!(
        store::title(found, &found.sessions[0]).as_deref(),
        Some("Work")
    );
    let missing = store::find(&projects, &home.rt, &gone.to_string_lossy()).unwrap();
    assert!(missing.destination_missing());
}

#[test]
fn mv_repoints_every_place_claude_code_keeps_the_path() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    let other = home.folder("other");
    home.session(&old, SESSION);
    home.memory(&old, "MEMORY.md", "- [Note](note.md)\n");
    let slug = project_slug(&old.to_string_lossy());
    home.memory(
        &old,
        "note.md",
        &format!(
            "Stored in ~/.claude/projects/{slug}/memory for {} only",
            old.display()
        ),
    );
    home.config(json!({
        "projects": {
            other.to_string_lossy(): {"a": 1},
            old.to_string_lossy(): {"hasTrustDialogAccepted": true}
        },
        "githubRepoPaths": {"me/app": [old.to_string_lossy()]}
    }));
    home.prompts(&[
        json!({"display": "hi", "project": old.to_string_lossy(), "sessionId": SESSION}),
        json!({"display": "elsewhere", "project": other.to_string_lossy()}),
    ]);
    let desktop = home.desktop_session("a", SESSION, &old);

    let report = home.mv(&old, &new).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    let dest = home.project_dir(&new);
    assert!(!home.project_dir(&old).exists());
    let transcript = read(&dest.join(format!("{SESSION}.jsonl")));
    let new_str = new.to_string_lossy();
    let old_str = old.to_string_lossy();
    assert!(transcript.contains(&format!("\"cwd\":\"{new_str}\"")));
    assert!(transcript.contains(&format!("edit {new_str}/src/main.rs")));
    assert!(transcript.contains(&format!("{new_str}/README")));
    // A longer name that merely starts with the old path is another folder.
    assert!(transcript.contains(&format!("{old_str}-old")));
    let subagent = read(&dest.join(SESSION).join("subagents").join("agent-a1.jsonl"));
    assert!(subagent.contains(&*new_str));
    let note = read(&dest.join("memory").join("note.md"));
    assert!(note.contains(&project_slug(&new_str)));
    assert!(note.contains(&*new_str));

    let config = home.read_config();
    let keys: Vec<&String> = config["projects"].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        [&other.to_string_lossy().into_owned(), &new_str.to_string()]
    );
    assert_eq!(config["githubRepoPaths"]["me/app"], json!([new_str]));

    let prompts = read(&home.rt.layout.prompt_history());
    assert!(prompts.contains(&format!("\"project\":\"{new_str}\"")));
    assert!(prompts.contains(&format!("\"project\":\"{}\"", other.to_string_lossy())));

    let desk: Value = serde_json::from_str(&read(&desktop)).unwrap();
    assert_eq!(desk["cwd"], json!(new_str));
    assert_eq!(desk["originCwd"], json!(new_str));
    assert_eq!(
        desk["writtenBranches"],
        json!([format!("{new_str}\u{0}main")])
    );
    assert!(
        !home
            .root
            .join("chatkeep/backups")
            .read_dir()
            .unwrap()
            .any(|_| true)
    );
}

#[test]
fn mv_keeps_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.config(json!({"projects": {old.to_string_lossy(): {}}}));
    fs::set_permissions(
        &home.rt.layout.config_file,
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let transcript = home.project_dir(&old).join(format!("{SESSION}.jsonl"));
    fs::set_permissions(&transcript, fs::Permissions::from_mode(0o644)).unwrap();

    home.mv(&old, &new).unwrap();

    let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&home.rt.layout.config_file), 0o600);
    assert_eq!(
        mode(&home.project_dir(&new).join(format!("{SESSION}.jsonl"))),
        0o644
    );
}

#[test]
fn dry_run_changes_nothing() {
    let mut home = Home::new();
    home.rt.dry_run = true;
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.config(json!({"projects": {old.to_string_lossy(): {}}}));
    let before = home.snapshot();

    let report = home.mv(&old, &new).unwrap();
    assert!(!report.applied.is_empty());
    assert_eq!(home.snapshot(), before);
}

#[test]
fn mv_skips_a_destination_that_does_not_exist() {
    let home = Home::new();
    let old = home.root.join("old");
    home.session(&old, SESSION);
    let before = home.snapshot();

    let report = home.mv(&old, &home.root.join("nowhere")).unwrap();
    assert_eq!(report.skipped.len(), 1);
    assert!(report.warnings[0].contains("destination missing"));
    assert_eq!(home.snapshot(), before);
}

#[test]
fn mv_refuses_while_a_session_runs_in_the_project() {
    let mut home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.live(4242, &old, "/x/claude/versions/2.1.0");
    let before = home.snapshot();

    let err = home.mv(&old, &new).unwrap_err();
    assert!(
        format!("{err:#}").contains("Claude Code is running in this project: Busy (pid 4242)"),
        "{err:#}"
    );
    assert_eq!(home.snapshot(), before);
}

#[test]
fn a_session_file_whose_process_is_gone_does_not_block() {
    let mut home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.live(4242, &old, "/x/claude/versions/2.1.0");
    home.running(Vec::new());

    home.mv(&old, &new).unwrap();
    assert!(home.project_dir(&new).exists());
}

#[test]
fn mv_refuses_while_the_desktop_app_lists_the_project() {
    let mut home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.desktop_session("a", SESSION, &old);
    home.running(vec![process(
        7,
        "/Applications/Claude.app/Contents/MacOS/Claude",
    )]);
    let before = home.snapshot();

    let err = home.mv(&old, &new).unwrap_err();
    assert!(
        format!("{err:#}").contains("desktop app is running"),
        "{err:#}"
    );
    assert_eq!(home.snapshot(), before);
}

#[test]
fn the_desktop_app_does_not_block_a_project_it_does_not_list() {
    let mut home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.running(vec![process(
        7,
        "/Applications/Claude.app/Contents/MacOS/Claude",
    )]);

    home.mv(&old, &new).unwrap();
    assert!(home.project_dir(&new).exists());
}

#[test]
fn mv_into_an_existing_project_merges_sessions_and_memory() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.session(&new, OTHER);
    home.memory(&old, "MEMORY.md", "- shared\n- from old\n");
    home.memory(&new, "MEMORY.md", "- shared\n- from new\n");
    home.memory(&old, "same.md", "identical");
    home.memory(&new, "same.md", "identical");
    home.config(json!({"projects": {
        old.to_string_lossy(): {"from": "old"},
        new.to_string_lossy(): {"from": "new"}
    }}));

    let report = home.mv(&old, &new).unwrap();
    assert!(report.warnings[0].contains("already has Claude Code settings"));

    let dest = home.project_dir(&new);
    assert!(dest.join(format!("{SESSION}.jsonl")).is_file());
    assert!(dest.join(format!("{OTHER}.jsonl")).is_file());
    assert_eq!(
        read(&dest.join("memory/MEMORY.md")),
        "- shared\n- from new\n- from old\n"
    );
    assert_eq!(read(&dest.join("memory/same.md")), "identical");
    assert!(!home.project_dir(&old).exists());
    let config = home.read_config();
    assert_eq!(config["projects"].as_object().unwrap().len(), 1);
    assert_eq!(
        config["projects"][new.to_string_lossy().as_ref()]["from"],
        "new"
    );
}

#[test]
fn a_merge_with_clashing_files_changes_nothing() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.session(&new, OTHER);
    home.memory(&old, "note.md", "old version");
    home.memory(&new, "note.md", "new version");
    let before = home.snapshot();

    let err = home.mv(&old, &new).unwrap_err();
    assert!(format!("{err:#}").contains("cannot merge"), "{err:#}");
    assert_eq!(home.snapshot(), before);
}

#[test]
fn a_failure_halfway_rolls_everything_back() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.memory(&old, "note.md", &format!("lives in {} now", old.display()));
    home.config(json!({"projects": {old.to_string_lossy(): {}}}));
    home.prompts(&[json!({"display": "hi", "project": old.to_string_lossy()})]);
    let desktop = home.desktop_session("a", SESSION, &old);
    let before = home.snapshot();
    let pairs = [(
        old.to_string_lossy().into_owned(),
        new.to_string_lossy().into_owned(),
    )];
    let (plans, report) = ops::plan_move(&home.rt, &pairs, None, false, false).unwrap();
    // Desktop sessions are rewritten after the project folder and settings moved.
    fs::set_permissions(&desktop, fs::Permissions::from_mode(0o000)).unwrap();

    let err = ops::execute_move(&home.rt, plans, report).unwrap_err();
    fs::set_permissions(&desktop, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        format!("{err:#}").contains("All changes were rolled back"),
        "{err:#}"
    );
    assert_eq!(home.snapshot(), before);
}

#[test]
fn replace_moves_every_project_under_a_prefix() {
    let home = Home::new();
    let a = home.root.join("older/a");
    let b = home.root.join("older/b");
    let keep = home.folder("elsewhere");
    home.folder("upgraded/a");
    home.folder("upgraded/b");
    home.session(&a, SESSION);
    home.session(&b, OTHER);
    home.session(&keep, "33333333-3333-4333-8333-333333333333");

    let (plans, report) =
        ops::plan_move(&home.rt, &[], Some(("/older/", "/upgraded/")), false, false).unwrap();
    assert_eq!(plans.len(), 2);
    ops::execute_move(&home.rt, plans, report).unwrap();

    assert!(home.project_dir(&home.root.join("upgraded/a")).exists());
    assert!(home.project_dir(&home.root.join("upgraded/b")).exists());
    assert!(home.project_dir(&keep).exists());
    assert!(!home.project_dir(&a).exists());
}

#[test]
fn project_flag_also_moves_the_real_folder() {
    let home = Home::new();
    let old = home.folder("old");
    fs::write(old.join("file.txt"), "content").unwrap();
    let new = home.root.join("new");
    home.session(&old, SESSION);

    let pairs = [(
        old.to_string_lossy().into_owned(),
        new.to_string_lossy().into_owned(),
    )];
    let (plans, report) = ops::plan_move(&home.rt, &pairs, None, false, true).unwrap();
    ops::execute_move(&home.rt, plans, report).unwrap();

    assert_eq!(read(&new.join("file.txt")), "content");
    assert!(!old.exists());
    assert!(home.project_dir(&new).exists());
}

#[test]
fn long_paths_land_in_the_hashed_folder_claude_code_expects() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder(&format!("{}project", "deep/".repeat(45)));
    home.session(&old, SESSION);

    home.mv(&old, &new).unwrap();
    let dest = home.project_dir(&new);
    assert!(dest.file_name().unwrap().len() > 200);
    assert!(dest.join(format!("{SESSION}.jsonl")).is_file());
}

#[test]
fn rm_removes_a_project_and_everything_keyed_to_its_sessions() {
    let home = Home::new();
    let app = home.folder("app");
    let other = home.folder("other");
    home.session(&app, SESSION);
    home.session(&other, OTHER);
    home.config(json!({"projects": {
        app.to_string_lossy(): {},
        other.to_string_lossy(): {}
    }}));
    home.prompts(&[
        json!({"display": "a", "project": app.to_string_lossy(), "sessionId": SESSION}),
        json!({"display": "b", "project": other.to_string_lossy(), "sessionId": OTHER}),
    ]);
    let desktop = home.desktop_session("a", SESSION, &app);
    let kept_desktop = home.desktop_session("b", OTHER, &other);
    let file_history = home.rt.layout.config_dir.join("file-history").join(SESSION);
    fs::create_dir_all(&file_history).unwrap();
    fs::write(file_history.join("x@v1"), "backup").unwrap();

    let target = ops::find_target(&home.rt, &app.to_string_lossy())
        .unwrap()
        .unwrap();
    ops::remove(&home.rt, &[target]).unwrap();

    assert!(!home.project_dir(&app).exists());
    assert!(home.project_dir(&other).exists());
    assert!(!desktop.exists());
    assert!(kept_desktop.exists());
    assert!(!file_history.exists());
    let config = home.read_config();
    assert_eq!(config["projects"].as_object().unwrap().len(), 1);
    let prompts = read(&home.rt.layout.prompt_history());
    assert!(!prompts.contains(&*app.to_string_lossy()));
    assert!(prompts.contains(&*other.to_string_lossy()));
}

#[test]
fn rm_removes_one_session_by_id() {
    let home = Home::new();
    let app = home.folder("app");
    home.session(&app, SESSION);
    home.session(&app, OTHER);

    let target = ops::find_target(&home.rt, SESSION).unwrap().unwrap();
    ops::remove(&home.rt, &[target]).unwrap();

    let dir = home.project_dir(&app);
    assert!(!dir.join(format!("{SESSION}.jsonl")).exists());
    assert!(!dir.join(SESSION).exists());
    assert!(dir.join(format!("{OTHER}.jsonl")).exists());
}

#[test]
fn cli_routes_to_claude_code_when_cursor_is_absent() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    let tools = Tools::new(None, Some(home.rt.clone()));

    let cli = Cli::parse_from([
        "chatkeep",
        "mv",
        "-y",
        &old.to_string_lossy(),
        &new.to_string_lossy(),
    ]);
    dispatch_with(cli, &tools).unwrap();
    assert!(home.project_dir(&new).exists());

    let history = read(&home.rt.layout.history_file());
    assert!(history.contains("\"command\":\"mv\""));
}

#[test]
fn cli_reports_when_no_tool_has_the_project() {
    let home = Home::new();
    let tools = Tools::new(None, Some(home.rt.clone()));
    let cli = Cli::parse_from(["chatkeep", "rm", "-y", "/no/such/project"]);
    let err = dispatch_with(cli, &tools).unwrap_err();
    assert!(
        format!("{err:#}").contains("no Claude Code project or session matches"),
        "{err:#}"
    );
}

/// A Claude Code project and a Cursor workspace for the same folder, with Cursor open.
fn both_tools(home: &Home, folder: &Path) -> (common::Home, chatkeep::engine::Runtime) {
    let cursor = common::cursor_home();
    common::write_folder_workspace(&cursor.layout, "hash-one", folder);
    let mut rt = common::runtime(&cursor.layout, true, false);
    rt.cwd = home.root.clone();
    (cursor, rt)
}

#[test]
fn an_open_cursor_queues_only_its_own_part() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    let (_cursor, cursor_rt) = both_tools(&home, &old);
    let tools = Tools::new(Some(cursor_rt), Some(home.rt.clone()));

    let cli = Cli::parse_from([
        "chatkeep",
        "mv",
        "-y",
        &old.to_string_lossy(),
        &new.to_string_lossy(),
    ]);
    let err = dispatch_with(cli, &tools).unwrap_err();

    // Claude Code's part ran; Cursor's waits for the queue.
    assert!(home.project_dir(&new).exists());
    let running = err
        .downcast_ref::<chatkeep::engine::CursorRunning>()
        .expect("Cursor is reported as running");
    assert_eq!(running.queue_args, ["--tool", "cursor"]);
    let argv: Vec<String> = ["mv", "-y", "/a", "/b"].map(String::from).to_vec();
    let report = chatkeep::queue_cmd::cursor_blocked_report(running, &argv);
    assert!(report.hint.contains("--tool cursor"), "{}", report.hint);
}

#[test]
fn a_project_cursor_does_not_know_ignores_an_open_cursor() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    let (_cursor, cursor_rt) = both_tools(&home, &home.root.join("unrelated"));
    let tools = Tools::new(Some(cursor_rt), Some(home.rt.clone()));

    let cli = Cli::parse_from([
        "chatkeep",
        "mv",
        "-y",
        &old.to_string_lossy(),
        &new.to_string_lossy(),
    ]);
    dispatch_with(cli, &tools).unwrap();
    assert!(home.project_dir(&new).exists());
}

#[test]
fn tool_cursor_leaves_claude_code_alone() {
    let home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    let (_cursor, mut cursor_rt) = both_tools(&home, &old);
    cursor_rt.probe = Arc::new(common::FixedProbe(false));
    let tools = Tools::new(Some(cursor_rt), Some(home.rt.clone()));

    let cli = Cli::parse_from([
        "chatkeep",
        "mv",
        "-y",
        "--tool",
        "cursor",
        &old.to_string_lossy(),
        &new.to_string_lossy(),
    ]);
    dispatch_with(cli, &tools).unwrap();
    assert!(home.project_dir(&old).exists());
    assert!(!home.project_dir(&new).exists());
}
