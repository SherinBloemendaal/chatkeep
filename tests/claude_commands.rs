//! `cp`, `split`, `combine`, `export`, `import`, `stats`, the queue, and the desktop app
//! accounts for Claude Code, on a fake `~/.claude`.

mod common;

use chatkeep::claude::{Busy, accounts, archive, slug::project_slug, stats, store, transfer};
use chatkeep::cli::{Cli, Command, Route, Tools, dispatch_with};
use chatkeep::engine::queue;
use chatkeep::queue_cmd;
use clap::Parser;
use common::claude_home::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const THIRD: &str = "33333333-3333-4333-8333-333333333333";
const CLAUDE_CLI: &str = "/Users/me/.local/share/claude/versions/2.1.0";
const DESKTOP_APP: &str = "/Applications/Claude.app/Contents/MacOS/Claude";

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn cli(tools: &Tools, args: &[&str]) -> anyhow::Result<()> {
    let argv = std::iter::once("chatkeep").chain(args.iter().copied());
    dispatch_with(Cli::parse_from(argv), tools)
}

fn claude_only(home: &Home) -> Tools {
    Tools::new(None, Some(home.rt.clone()))
}

fn cp(
    home: &Home,
    from: &Path,
    to: &Path,
    folder: bool,
) -> anyhow::Result<chatkeep::engine::Report> {
    let pairs = [(text(from), text(to))];
    let (plans, report) = transfer::plan_copy(&home.rt, &pairs, None, false, folder)?;
    transfer::execute_copy(&home.rt, plans, report)
}

fn project(home: &Home, path: &Path) -> store::Project {
    let projects = store::discover(&home.rt).unwrap();
    store::find(&projects, &home.rt, &text(path))
        .unwrap_or_else(|| panic!("no project for {}", path.display()))
        .clone()
}

#[test]
fn cp_gives_the_new_folder_its_own_copy_of_every_chat() {
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    let app_str = text(&app);
    let copy_str = text(&copy);
    let slug = project_slug(&app_str);
    home.transcript(
        &app,
        SESSION,
        &[
            json!({"type": "user", "cwd": app_str, "sessionId": SESSION,
                "message": {"content": format!("edit {app_str}/src/main.rs")}}),
            json!({"type": "bridge-session", "sessionId": SESSION, "bridgeSessionId": "cse_1"}),
            json!({"type": "user", "cwd": app_str, "sessionId": SESSION,
                "message": {"content": format!("saved to ~/.claude/projects/{slug}/{SESSION}/tool-results/a.txt")}}),
        ],
    );
    let extra = home.project_dir(&app).join(SESSION);
    fs::create_dir_all(extra.join("tool-results")).unwrap();
    fs::write(
        extra.join("tool-results").join("a.txt"),
        format!("{app_str}/x"),
    )
    .unwrap();
    fs::write(extra.join("blob.bin"), [0xff, 0xfe, 0x00]).unwrap();
    home.memory(&app, "MEMORY.md", "- [Note](note.md)\n");
    home.memory(&app, "note.md", &format!("lives in {app_str}"));
    let checkpoints = home.rt.layout.config_dir.join("file-history");
    fs::create_dir_all(checkpoints.join(SESSION)).unwrap();
    fs::write(checkpoints.join(SESSION).join("abc@v1"), "before").unwrap();
    home.config(json!({"projects": {app_str.clone(): {"hasTrustDialogAccepted": true}}}));
    home.desktop_entry(ACCOUNT, ORG, "a", SESSION, &app);
    let before = fs::read(home.project_dir(&app).join(format!("{SESSION}.jsonl"))).unwrap();

    let report = cp(&home, &app, &copy, false).unwrap();
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);

    // The original is untouched.
    assert_eq!(
        fs::read(home.project_dir(&app).join(format!("{SESSION}.jsonl"))).unwrap(),
        before
    );
    let ids = home.session_ids(&copy);
    assert_eq!(ids.len(), 1);
    let new = &ids[0];
    assert_ne!(new, SESSION);
    let dest = home.project_dir(&copy);
    let transcript = read(&dest.join(format!("{new}.jsonl")));
    assert!(transcript.contains(&format!("\"cwd\":\"{copy_str}\"")));
    assert!(transcript.contains(&format!("edit {copy_str}/src/main.rs")));
    assert!(transcript.contains(&format!("\"sessionId\":\"{new}\"")));
    assert!(!transcript.contains(SESSION), "{transcript}");
    assert!(!transcript.contains(&app_str), "{transcript}");
    assert!(
        transcript.contains(&format!(
            "~/.claude/projects/{}/{new}/tool-results/a.txt",
            project_slug(&copy_str)
        )),
        "{transcript}"
    );
    // A copy does not claim the remote session of its original.
    assert!(!transcript.contains("bridge-session"));
    assert_eq!(transcript.lines().count(), 2);
    assert_eq!(
        read(&dest.join(new).join("tool-results").join("a.txt")),
        format!("{copy_str}/x")
    );
    assert_eq!(
        fs::read(dest.join(new).join("blob.bin")).unwrap(),
        [0xff, 0xfe, 0x00]
    );
    assert_eq!(
        read(&dest.join("memory").join("note.md")),
        format!("lives in {copy_str}")
    );
    assert_eq!(read(&checkpoints.join(new).join("abc@v1")), "before");
    assert!(checkpoints.join(SESSION).is_dir());

    let config = home.read_config();
    assert_eq!(config["projects"][&copy_str], config["projects"][&app_str]);

    let entries = home.desktop_entries(ACCOUNT, ORG);
    assert_eq!(entries.len(), 2);
    let added = entries
        .iter()
        .find(|entry| entry["cliSessionId"] == json!(new))
        .expect("the copy has its own desktop app entry");
    assert_eq!(added["cwd"], json!(copy_str));
    assert_ne!(added["sessionId"], json!("local_a"));
    assert!(added.get("bridgeSessionIds").is_none());
    assert_eq!(project(&home, &copy).sessions.len(), 1);
}

#[test]
fn cp_twice_merges_memory_and_keeps_both_sets_of_chats() {
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    home.memory(&app, "MEMORY.md", "- one\n");
    home.memory(&copy, "MEMORY.md", "- two\n");
    cp(&home, &app, &copy, false).unwrap();
    cp(&home, &app, &copy, false).unwrap();
    assert_eq!(home.session_ids(&copy).len(), 2);
    assert_eq!(
        read(&home.project_dir(&copy).join("memory").join("MEMORY.md")),
        "- two\n- one\n"
    );
}

#[test]
fn cp_stops_before_it_overwrites_a_different_memory_file() {
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    home.memory(&app, "note.md", "ours");
    home.memory(&copy, "note.md", "theirs");
    let before = home.snapshot();
    let err = cp(&home, &app, &copy, false).unwrap_err();
    assert!(
        format!("{err:#}").contains("different memory files"),
        "{err:#}"
    );
    assert!(home.snapshot() == before, "the run changed files");
}

#[test]
fn cp_with_project_also_copies_the_real_folder() {
    let home = Home::new();
    let app = home.folder("app");
    fs::write(app.join("main.rs"), "fn main() {}").unwrap();
    home.session(&app, SESSION);
    let copy = home.root.join("copy");
    cp(&home, &app, &copy, true).unwrap();
    assert_eq!(read(&copy.join("main.rs")), "fn main() {}");
    assert!(app.join("main.rs").is_file());
    assert_eq!(home.session_ids(&copy).len(), 1);
}

#[test]
fn cp_skips_a_missing_destination_and_refuses_the_source_itself() {
    let home = Home::new();
    let app = home.folder("app");
    home.session(&app, SESSION);
    let report = cp(&home, &app, &home.root.join("nowhere"), false).unwrap();
    assert_eq!(report.skipped.len(), 1);
    assert!(report.warnings[0].contains("destination missing"));
    let err = cp(&home, &app, &app, false).unwrap_err();
    assert!(format!("{err:#}").contains("is the source project itself"));
}

#[test]
fn a_dry_run_of_cp_changes_nothing() {
    let mut home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    home.rt.dry_run = true;
    let before = home.snapshot();
    let report = cp(&home, &app, &copy, false).unwrap();
    assert!(
        report.applied[0].starts_with("copy project"),
        "{:?}",
        report.applied
    );
    assert!(home.snapshot() == before, "the run changed files");
}

#[test]
fn cp_waits_for_a_session_that_still_runs_in_the_project() {
    let mut home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    home.live(4242, &app, CLAUDE_CLI);
    let before = home.snapshot();
    let err = cp(&home, &app, &copy, false).unwrap_err();
    let busy = err.downcast_ref::<Busy>().expect("a busy error");
    assert!(busy.summary.contains("Busy (pid 4242)"), "{}", busy.summary);
    let argv: Vec<String> = ["cp", "/a", "/b"].map(String::from).to_vec();
    let report = queue_cmd::claude_blocked_report(busy, &argv);
    assert_eq!(
        report.stable,
        "chatkeep: claude-busy; queue with: chatkeep queue add -- cp /a /b"
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &argv), 75);
    assert!(home.snapshot() == before, "the run changed files");
}

#[cfg(unix)]
#[test]
fn a_cp_that_fails_halfway_leaves_nothing_behind() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    home.session(&app, OTHER);
    let pairs = [(text(&app), text(&copy))];
    let (plans, report) = transfer::plan_copy(&home.rt, &pairs, None, false, false).unwrap();
    // The second transcript becomes unreadable after planning, so the copy fails midway.
    let locked = home.project_dir(&app).join(format!("{SESSION}.jsonl"));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let err = transfer::execute_copy(&home.rt, plans, report).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        format!("{err:#}").contains("All changes were rolled back"),
        "{err:#}"
    );
    assert!(!home.project_dir(&copy).exists());
    assert_eq!(home.session_ids(&app).len(), 2);
}

/// A monorepo with one chat per sub-project, one that touched both, and one that touched
/// neither.
fn monorepo(home: &Home) -> (PathBuf, PathBuf, PathBuf) {
    let mono = home.folder("mono");
    let api = home.folder("mono/api");
    let web = home.folder("mono/web");
    let mono_str = text(&mono);
    let edit = |file: &str| {
        json!({"type": "assistant", "cwd": mono_str, "message": {"content": [
            {"type": "tool_use", "name": "Edit", "input": {"file_path": file}}
        ]}})
    };
    home.transcript(
        &mono,
        SESSION,
        &[
            json!({"type": "user", "cwd": mono_str, "sessionId": SESSION}),
            edit(&format!("{}/src/a.rs", text(&api))),
        ],
    );
    home.transcript(
        &mono,
        OTHER,
        &[
            json!({"type": "user", "cwd": mono_str, "sessionId": OTHER}),
            edit(&format!("{}/a.ts", text(&web))),
            edit(&format!("{}/b.rs", text(&api))),
        ],
    );
    home.transcript(
        &mono,
        THIRD,
        &[
            json!({"type": "user", "cwd": mono_str, "sessionId": THIRD}),
            edit(&format!("{mono_str}/README.md")),
        ],
    );
    (mono, api, web)
}

#[test]
fn split_suggests_targets_from_the_files_a_chat_touched() {
    let home = Home::new();
    let (mono, api, web) = monorepo(&home);
    let source = project(&home, &mono);
    let found = transfer::suggest_split(&home.rt, &source, &[api.clone(), web.clone()]).unwrap();
    assert_eq!(found.assigned[OTHER], [web, api.clone()]);
    assert_eq!(found.assigned[SESSION], [api]);
    assert_eq!(found.unassigned, [THIRD]);
}

#[test]
fn split_copies_chats_and_leaves_the_paths_they_mention_alone() {
    let home = Home::new();
    let (mono, api, web) = monorepo(&home);
    let source = project(&home, &mono);
    let assigned = BTreeMap::from([
        (SESSION.to_string(), vec![api.clone()]),
        (OTHER.to_string(), vec![web.clone(), api.clone()]),
    ]);
    let report = transfer::split(
        &home.rt,
        &source,
        &[api.clone(), web.clone()],
        &assigned,
        false,
    )
    .unwrap();
    assert_eq!(report.applied.len(), 2, "{:?}", report.applied);

    // Everything is still in the source.
    assert_eq!(home.session_ids(&mono).len(), 3);
    assert_eq!(home.session_ids(&api).len(), 2);
    assert_eq!(home.session_ids(&web).len(), 1);
    let copied = home.session_ids(&web).remove(0);
    let transcript = read(&home.project_dir(&web).join(format!("{copied}.jsonl")));
    // The chat now runs in the target, but the files it worked on did not move.
    assert!(transcript.contains(&format!("\"cwd\":\"{}\"", text(&web))));
    assert!(transcript.contains(&format!("{}/b.rs", text(&api))));
    assert!(transcript.contains(&format!("\"sessionId\":\"{copied}\"")));
    assert_eq!(project(&home, &web).path.as_deref(), Some(web.as_path()));
}

#[test]
fn split_with_move_takes_each_chat_out_of_the_source() {
    let home = Home::new();
    let (mono, api, web) = monorepo(&home);
    let mono_str = text(&mono);
    home.prompts(&[
        json!({"display": "a", "project": mono_str, "sessionId": SESSION}),
        json!({"display": "b", "project": mono_str, "sessionId": THIRD}),
    ]);
    let desktop = home.desktop_session("a", SESSION, &mono);
    let source = project(&home, &mono);
    let assigned = BTreeMap::from([
        (SESSION.to_string(), vec![api.clone()]),
        (OTHER.to_string(), vec![web.clone(), api.clone()]),
    ]);
    transfer::split(
        &home.rt,
        &source,
        &[api.clone(), web.clone()],
        &assigned,
        true,
    )
    .unwrap();

    assert_eq!(home.session_ids(&mono), [THIRD]);
    // The first target gets the chat itself, every other target a copy.
    assert_eq!(home.session_ids(&web), [OTHER]);
    let in_api = home.session_ids(&api);
    assert_eq!(in_api.len(), 2);
    assert!(in_api.contains(&SESSION.to_string()));
    assert!(!in_api.contains(&OTHER.to_string()));

    let desk: Value = serde_json::from_str(&read(&desktop)).unwrap();
    assert_eq!(desk["cwd"], json!(text(&api)));
    let prompts = read(&home.rt.layout.prompt_history());
    assert!(prompts.contains(&format!("\"project\":\"{}\"", text(&api))));
    assert!(prompts.contains(&format!("\"project\":\"{mono_str}\"")));
}

#[test]
fn split_refuses_targets_it_cannot_use() {
    let home = Home::new();
    let (mono, api, _) = monorepo(&home);
    let source = project(&home, &mono);
    let none = BTreeMap::new();
    for (targets, message) in [
        (vec![home.root.join("nope")], "target does not exist"),
        (vec![mono.clone()], "is the source project"),
        (vec![api.clone(), api.clone()], "is listed twice"),
    ] {
        let err = transfer::split(&home.rt, &source, &targets, &none, false).unwrap_err();
        assert!(format!("{err:#}").contains(message), "{err:#}");
    }
    let stray = BTreeMap::from([("not-a-chat".to_string(), vec![api.clone()])]);
    let err = transfer::split(&home.rt, &source, &[api], &stray, false).unwrap_err();
    assert!(format!("{err:#}").contains("does not belong to"), "{err:#}");
}

#[test]
fn split_from_the_command_line_needs_a_target_for_every_chat() {
    let home = Home::new();
    let (mono, api, web) = monorepo(&home);
    let tools = claude_only(&home);
    let err = cli(&tools, &["split", &text(&mono), &text(&api), &text(&web)]).unwrap_err();
    assert!(
        format!("{err:#}").contains("1 chat match no target folder"),
        "{err:#}"
    );
    assert_eq!(home.session_ids(&api).len(), 0);
}

#[test]
fn combine_brings_projects_and_single_chats_into_one_project() {
    let home = Home::new();
    let one = home.folder("one");
    let two = home.folder("two");
    let all = home.folder("all");
    home.session(&one, SESSION);
    home.session(&two, OTHER);
    home.session(&two, THIRD);
    home.session(&all, "44444444-4444-4444-8444-444444444444");
    let sources = [text(&one), THIRD.to_string(), text(&all)];
    let report = transfer::combine(&home.rt, &text(&all), &sources, false).unwrap();
    assert_eq!(report.skipped, [text(&all)]);
    assert!(report.warnings[0].contains("it is the combine target"));
    // Two copies joined the one chat the target had; the sources keep theirs.
    assert_eq!(home.session_ids(&all).len(), 3);
    assert_eq!(home.session_ids(&one), [SESSION]);
    assert_eq!(home.session_ids(&two).len(), 2);
}

#[test]
fn combine_with_move_empties_the_sources() {
    let home = Home::new();
    let one = home.folder("one");
    let all = home.folder("all");
    home.session(&one, SESSION);
    transfer::combine(&home.rt, &text(&all), &[text(&one)], true).unwrap();
    assert_eq!(home.session_ids(&all), [SESSION]);
    assert!(home.session_ids(&one).is_empty());
    let moved = read(&home.project_dir(&all).join(format!("{SESSION}.jsonl")));
    assert!(moved.contains(&format!("\"cwd\":\"{}\"", text(&all))));
    // The subagent transcripts travel with their session.
    assert!(
        home.project_dir(&all)
            .join(SESSION)
            .join("subagents")
            .join("agent-a1.jsonl")
            .is_file()
    );
}

#[test]
fn combine_names_a_source_it_cannot_find() {
    let home = Home::new();
    let all = home.folder("all");
    let err = transfer::combine(&home.rt, &text(&all), &["nope".to_string()], false).unwrap_err();
    assert!(format!("{err:#}").contains("no Claude Code project or session matches nope"));
    let err = transfer::combine(&home.rt, "/no/such/folder", &[], false).unwrap_err();
    assert!(format!("{err:#}").contains("target does not exist"));
}

/// A project with everything an archive carries.
fn full_project(home: &Home) -> PathBuf {
    let app = home.folder("app");
    let app_str = text(&app);
    home.session(&app, SESSION);
    home.session(&app, OTHER);
    home.memory(&app, "MEMORY.md", "- [Note](note.md)\n");
    home.memory(&app, "note.md", &format!("lives in {app_str}"));
    let checkpoints = home.rt.layout.config_dir.join("file-history").join(SESSION);
    fs::create_dir_all(&checkpoints).unwrap();
    fs::write(checkpoints.join("abc@v1"), "before").unwrap();
    home.config(json!({"projects": {
        "/elsewhere": {"x": 1},
        app_str.clone(): {"hasTrustDialogAccepted": true}
    }}));
    home.prompts(&[
        json!({"display": "hi", "project": app_str, "sessionId": SESSION}),
        json!({"display": "other", "project": "/elsewhere"}),
    ]);
    home.desktop_session("a", SESSION, &app);
    app
}

#[test]
fn export_then_import_puts_a_removed_project_back_exactly() {
    let home = Home::new();
    let app = full_project(&home);
    let file = home.root.join("app.chatkeep");
    let before = home.snapshot();
    let report = archive::export(&home.rt, &project(&home, &app), &file).unwrap();
    assert!(report.applied[0].starts_with("export project"));
    assert_eq!(chatkeep::engine::archive_tool(&file).unwrap(), "claude");

    let target = chatkeep::claude::ops::find_target(&home.rt, &text(&app))
        .unwrap()
        .unwrap();
    chatkeep::claude::ops::remove(&home.rt, &[target]).unwrap();
    assert!(!home.project_dir(&app).exists());

    archive::import(&home.rt, &file, None, false).unwrap();
    let mut after = home.snapshot();
    after.retain(|(path, _)| path != &file);
    assert_eq!(changed_files(&home, &before, &after), Vec::<PathBuf>::new());
}

/// Files that differ between two snapshots. Settings are compared as JSON and the prompt
/// history as a set of lines: both are rewritten as a whole, which may reorder them.
fn changed_files(
    home: &Home,
    before: &[(PathBuf, Vec<u8>)],
    after: &[(PathBuf, Vec<u8>)],
) -> Vec<PathBuf> {
    let normal = |snapshot: &[(PathBuf, Vec<u8>)]| -> BTreeMap<PathBuf, Vec<u8>> {
        snapshot
            .iter()
            .filter(|(path, _)| path.is_file())
            .map(|(path, bytes)| {
                let bytes = if path == &home.rt.layout.config_file {
                    let json: Value = serde_json::from_slice(bytes).unwrap();
                    let mut keys: Vec<String> = json["projects"]
                        .as_object()
                        .unwrap()
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect();
                    keys.sort();
                    keys.join("\n").into_bytes()
                } else if path == &home.rt.layout.prompt_history() {
                    let text = String::from_utf8_lossy(bytes).into_owned();
                    let mut lines: Vec<&str> = text.lines().collect();
                    lines.sort();
                    lines.join("\n").into_bytes()
                } else {
                    bytes.clone()
                };
                (path.clone(), bytes)
            })
            .collect()
    };
    let (before, after) = (normal(before), normal(after));
    let mut changed: Vec<PathBuf> = before
        .iter()
        .filter(|(path, bytes)| after.get(*path) != Some(bytes))
        .map(|(path, _)| path.clone())
        .collect();
    changed.extend(
        after
            .keys()
            .filter(|path| !before.contains_key(*path))
            .cloned(),
    );
    changed
}

#[test]
fn import_into_another_folder_rewrites_the_paths() {
    let home = Home::new();
    let app = full_project(&home);
    let moved = home.folder("moved");
    let file = home.root.join("app.chatkeep");
    archive::export(&home.rt, &project(&home, &app), &file).unwrap();

    // The chats are still here, so nothing is imported until --overwrite says so.
    let report = archive::import(&home.rt, &file, Some(&moved), false).unwrap();
    assert_eq!(report.skipped.len(), 2);
    assert!(home.session_ids(&moved).is_empty());

    let report = archive::import(&home.rt, &file, Some(&moved), true).unwrap();
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("replacing existing session"))
    );
    assert_eq!(home.session_ids(&moved), [SESSION, OTHER]);
    assert!(home.session_ids(&app).is_empty());
    let transcript = read(&home.project_dir(&moved).join(format!("{SESSION}.jsonl")));
    assert!(transcript.contains(&format!("\"cwd\":\"{}\"", text(&moved))));
    assert!(!transcript.contains(&format!("{}/", text(&app))));
    assert_eq!(
        read(&home.project_dir(&moved).join("memory").join("note.md")),
        format!("lives in {}", text(&moved))
    );
    let config = home.read_config();
    assert_eq!(
        config["projects"][text(&moved)]["hasTrustDialogAccepted"],
        true
    );
    let prompts = read(&home.rt.layout.prompt_history());
    assert!(prompts.contains(&format!("\"project\":\"{}\"", text(&moved))));
}

#[test]
fn import_refuses_an_archive_that_was_changed() {
    let home = Home::new();
    let app = full_project(&home);
    let file = home.root.join("app.chatkeep");
    archive::export(&home.rt, &project(&home, &app), &file).unwrap();
    let err = archive::export(&home.rt, &project(&home, &app), &file).unwrap_err();
    assert!(format!("{err:#}").contains("refusing to overwrite"));

    // Repack with one transcript altered.
    let unpacked = home.root.join("unpacked");
    fs::create_dir_all(&unpacked).unwrap();
    let mut archive =
        tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(&file).unwrap()));
    archive.unpack(&unpacked).unwrap();
    fs::write(
        unpacked.join("project").join(format!("{SESSION}.jsonl")),
        "tampered\n",
    )
    .unwrap();
    let forged = home.root.join("forged.chatkeep");
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        fs::File::create(&forged).unwrap(),
        flate2::Compression::default(),
    ));
    builder.append_dir_all(".", &unpacked).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    let before = home.snapshot();
    let err = archive::import(&home.rt, &forged, None, true).unwrap_err();
    assert!(format!("{err:#}").contains("checksum mismatch"), "{err:#}");
    assert!(home.snapshot() == before, "the run changed files");
}

#[test]
fn import_finds_the_tool_from_the_archive() {
    let home = Home::new();
    let app = full_project(&home);
    let file = home.root.join("app.chatkeep");
    let tools = claude_only(&home);
    cli(&tools, &["export", &text(&app), &text(&file)]).unwrap();
    cli(&tools, &["rm", "-y", &text(&app)]).unwrap();

    let cursor = common::cursor_home();
    let mut cursor_rt = common::runtime(&cursor.layout, false, false);
    cursor_rt.cwd = home.root.clone();
    let both = Tools::new(Some(cursor_rt), Some(home.rt.clone()));
    let err = cli(&both, &["import", "--tool", "cursor", &text(&file)]).unwrap_err();
    assert!(
        format!("{err:#}").contains("holds Claude Code chats, not Cursor chats"),
        "{err:#}"
    );
    cli(&both, &["import", &text(&file)]).unwrap();
    assert_eq!(home.session_ids(&app), [SESSION, OTHER]);
}

#[test]
fn stats_count_every_answer_once() {
    let home = Home::new();
    let app = home.folder("app");
    let answer = |id: &str, model: &str, out: u64| {
        json!({"type": "assistant", "timestamp": "2026-03-04T10:00:00Z", "message": {
            "id": id, "model": model,
            "usage": {"input_tokens": 10, "output_tokens": out,
                "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000}
        }})
    };
    let file = home.transcript(
        &app,
        SESSION,
        &[
            json!({"type": "user", "timestamp": "2026-02-01T09:00:00Z", "cwd": text(&app)}),
            // One answer written as two lines, one per part: counted once.
            answer("msg_1", "claude-opus", 5),
            answer("msg_1", "claude-opus", 5),
            answer("msg_2", "claude-opus", 7),
            answer("msg_3", "claude-haiku", 1),
            answer("msg_4", "<synthetic>", 0),
        ],
    );
    let usage = stats::read_usage(&file).unwrap();
    assert_eq!(usage.tokens.input, 40);
    assert_eq!(usage.tokens.output, 13);
    assert_eq!(usage.tokens.cache_write, 400);
    assert_eq!(usage.tokens.cache_read, 4000);
    assert_eq!(usage.model(), Some("claude-opus"));
    assert_eq!(usage.month.as_deref(), Some("2026-02"));

    let found = stats::collect(&home.rt).unwrap();
    assert_eq!(found.projects, 1);
    assert_eq!(found.sessions, 1);
    assert_eq!(found.models["claude-opus"], 1);
    assert_eq!(found.per_month["2026-02"], 1);
    let shown = stats::render_at(chatkeep::ui::Theme::plain(), &found, Some(120));
    for needle in [
        "Claude Code overview",
        "40 in, 13 out",
        "claude-opus",
        "2026-02",
    ] {
        assert!(shown.contains(needle), "{needle} not in:\n{shown}");
    }
    cli(&claude_only(&home), &["stats", "--tool", "claude"]).unwrap();
}

const OLD_ACCOUNT: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const NEW_ACCOUNT: &str = "bbbbbbbb-2222-4222-8222-222222222222";

/// Two desktop app accounts: the old one lists three chats, the new one is signed in.
fn two_accounts(home: &Home) -> PathBuf {
    let app = home.folder("app");
    home.session(&app, SESSION);
    home.session(&app, OTHER);
    home.desktop_entry(OLD_ACCOUNT, "org-old", "one", SESSION, &app);
    home.desktop_entry(OLD_ACCOUNT, "org-old", "two", OTHER, &app);
    home.desktop_entry(OLD_ACCOUNT, "org-old", "gone", THIRD, &app);
    home.desktop_entry(NEW_ACCOUNT, "org-new", "two", OTHER, &app);
    home.signed_in(NEW_ACCOUNT);
    app
}

#[test]
fn accounts_ls_shows_which_chats_can_still_be_opened() {
    let home = Home::new();
    two_accounts(&home);
    let shown = accounts::render(&home.rt, chatkeep::ui::Theme::plain()).unwrap();
    let row = |account: &str| {
        shown
            .lines()
            .find(|line| line.contains(account))
            .unwrap_or_else(|| panic!("{account} not in:\n{shown}"))
            .to_string()
    };
    let old: Vec<String> = row(OLD_ACCOUNT)
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert!(
        old.contains(&"3".to_string()) && old.contains(&"2".to_string()),
        "{old:?}"
    );
    assert!(row(NEW_ACCOUNT).contains("signed in"));
    assert!(!row(OLD_ACCOUNT).contains("signed in"));
}

#[test]
fn accounts_cp_lists_old_chats_under_the_account_signed_in_now() {
    let home = Home::new();
    two_accounts(&home);
    // The start of an account id is enough.
    let (plan, report) = accounts::plan_copy(&home.rt, "aaaa", None).unwrap();
    assert_eq!(plan.to.account, NEW_ACCOUNT);
    assert_eq!(plan.rows().len(), 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("skipped 1 chat: no transcript"))
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|line| line.contains("already lists them"))
    );
    let report = accounts::execute_copy(&home.rt, &plan, report).unwrap();
    assert_eq!(
        report.applied,
        [format!("copy 1 chat {OLD_ACCOUNT} -> {NEW_ACCOUNT}")]
    );

    let entries = home.desktop_entries(NEW_ACCOUNT, "org-new");
    assert_eq!(entries.len(), 2);
    let copied = entries
        .iter()
        .find(|entry| entry["cliSessionId"] == json!(SESSION))
        .unwrap();
    assert_eq!(copied["sessionId"], json!("local_one"));
    assert_eq!(copied["title"], json!("Chat one"));
    // What belongs to the old account stays with it.
    assert!(copied.get("bridgeSessionIds").is_none());
    assert!(copied.get("remoteMcpServersConfig").is_none());
    // The old account keeps its own list untouched.
    let kept = home.desktop_entries(OLD_ACCOUNT, "org-old");
    assert_eq!(kept.len(), 3);
    assert!(
        kept.iter()
            .all(|entry| entry.get("bridgeSessionIds").is_some())
    );

    // A second run has nothing left to copy.
    let (plan, _) = accounts::plan_copy(&home.rt, OLD_ACCOUNT, None).unwrap();
    assert!(plan.is_empty());
}

#[test]
fn accounts_cp_says_what_is_wrong_with_the_accounts_it_got() {
    let home = Home::new();
    two_accounts(&home);
    let message = |from: &str, to: Option<&str>| {
        format!("{:#}", accounts::plan_copy(&home.rt, from, to).unwrap_err())
    };
    assert!(message("cccc", None).contains("has no account cccc"));
    assert!(message(NEW_ACCOUNT, None).contains("is the account the chats would be copied to"));
    assert!(message(OLD_ACCOUNT, Some("dddd")).contains("has no account dddd"));

    // Without a signed-in account, the target has to be named.
    fs::remove_file(
        home.rt
            .layout
            .desktop_dir
            .clone()
            .unwrap()
            .join("config.json"),
    )
    .unwrap();
    assert!(message(OLD_ACCOUNT, None).contains("cannot tell which account"));
    assert!(accounts::plan_copy(&home.rt, OLD_ACCOUNT, Some("bbbb")).is_ok());
}

#[test]
fn accounts_cp_from_the_command_line_picks_the_only_other_account() {
    let mut home = Home::new();
    two_accounts(&home);
    home.running(vec![process(7, DESKTOP_APP)]);
    let tools = claude_only(&home);
    cli(&tools, &["claude", "accounts", "ls"]).unwrap();
    cli(&tools, &["claude", "accounts", "cp", "-n"]).unwrap();
    assert_eq!(home.desktop_entries(NEW_ACCOUNT, "org-new").len(), 1);
    // New entries never clash with a running app; it only has to be restarted to show them.
    cli(&tools, &["claude", "accounts", "cp", "-y"]).unwrap();
    assert_eq!(home.desktop_entries(NEW_ACCOUNT, "org-new").len(), 2);
    let history = read(&home.rt.layout.history_file());
    assert!(
        history.contains("\"command\":\"claude accounts\""),
        "{history}"
    );
}

#[test]
fn the_queue_holds_a_claude_code_command_until_its_project_is_free() {
    let mut home = Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, SESSION);
    home.live(4242, &old, CLAUDE_CLI);
    let tools = claude_only(&home);
    let old_str = text(&old);
    let new_str = text(&new);

    // Run directly, the command is refused and says how to queue it.
    let err = cli(&tools, &["mv", "-y", &old_str, &new_str]).unwrap_err();
    assert!(err.downcast_ref::<Busy>().is_some(), "{err:#}");

    cli(&tools, &["queue", "add", "--", "mv", &old_str, &new_str]).unwrap();
    let state = home.rt.layout.chatkeep_home.clone();
    assert_eq!(queue::load(&state).unwrap().len(), 1);

    // Still busy: the entry stays and the queue reports how many wait.
    let err = cli(&tools, &["queue", "execute", "-y"]).unwrap_err();
    let busy = err.downcast_ref::<Busy>().expect("a busy error");
    assert_eq!(busy.pending, Some(1));
    assert_eq!(
        queue_cmd::claude_blocked_report(busy, &[]).stable,
        "chatkeep: claude-busy; queued: 1 pending; run later: chatkeep queue execute"
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 75);
    let entries = queue::load(&state).unwrap();
    assert_eq!(entries[0].status, queue::Status::Pending);
    assert!(home.project_dir(&old).exists());

    // The session ended: the queue runs the move.
    home.running(Vec::new());
    let tools = claude_only(&home);
    cli(&tools, &["queue", "execute", "-y"]).unwrap();
    assert!(queue::load(&state).unwrap().is_empty());
    assert!(!home.project_dir(&old).exists());
    assert_eq!(home.session_ids(&new), [SESSION]);
}

#[test]
fn the_queue_refuses_commands_that_can_run_right_away() {
    let home = Home::new();
    let tools = claude_only(&home);
    let err = cli(&tools, &["queue", "add", "--", "ls"]).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("ls does not need Claude Code closed and can run directly"),
        "{err}"
    );
    let err = cli(&tools, &["queue", "add", "--", "mv", "/no/such", "/other"]).unwrap_err();
    assert!(format!("{err:#}").contains("refusing to queue"), "{err:#}");
}

/// A Cursor workspace for `folder` next to the Claude Code home.
fn with_cursor(home: &Home, folder: &Path, running: bool) -> (common::Home, Tools) {
    let cursor = common::cursor_home();
    common::write_folder_workspace(&cursor.layout, "hash-one", folder);
    let mut rt = common::runtime(&cursor.layout, running, false);
    rt.cwd = home.root.clone();
    let tools = Tools::new(Some(rt), Some(home.rt.clone()));
    (cursor, tools)
}

#[test]
fn a_queued_command_for_both_tools_leaves_only_cursors_half_when_cursor_is_open() {
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    let (cursor, tools) = with_cursor(&home, &app, true);
    let state = cursor.layout.chatkeep_home.clone();

    cli(
        &tools,
        &["queue", "add", "--", "cp", &text(&app), &text(&copy)],
    )
    .unwrap();
    let err = cli(&tools, &["queue", "execute", "-y"]).unwrap_err();
    assert!(
        err.downcast_ref::<chatkeep::engine::CursorRunning>()
            .is_some(),
        "{err:#}"
    );
    // Nothing ran: an entry that needs Cursor waits as a whole.
    assert!(home.session_ids(&copy).is_empty());
    assert_eq!(queue::load(&state).unwrap()[0].argv.len(), 3);
}

#[test]
fn flags_and_namespaces_decide_the_tool() {
    let home = Home::new();
    let app = home.folder("app");
    home.session(&app, SESSION);
    let (_cursor, tools) = with_cursor(&home, &app, false);
    let message = |args: &[&str]| format!("{:#}", cli(&tools, args).unwrap_err());

    assert!(
        message(&["ls", "--profile", "default", "--tool", "claude"])
            .contains("--profile only exists for Cursor")
    );
    assert!(
        message(&["cursor", "rx", "hash-one", "--tool", "claude"])
            .contains("chatkeep cursor commands only work on Cursor")
    );
    assert!(
        message(&["export", &text(&app), &text(&home.root.join("a.chatkeep"))])
            .contains("has chats in both Cursor and Claude Code")
    );
    assert!(
        message(&["combine", &text(&app), "nope"])
            .contains("no Cursor workspace, Claude Code project, or chat matches nope")
    );

    // `--profile` alone means Cursor, so Claude Code's project is not listed or touched.
    cli(&tools, &["ls", "--profile", "default"]).unwrap();
    let only_claude = claude_only(&home);
    assert!(
        format!(
            "{:#}",
            cli(&only_claude, &["cursor", "rx", "x"]).unwrap_err()
        )
        .contains("no Cursor installation was found")
    );
    let only_cursor = Tools::new(
        Some(common::runtime(&common::cursor_home().layout, false, false)),
        None,
    );
    assert!(
        format!(
            "{:#}",
            cli(&only_cursor, &["claude", "accounts", "ls"]).unwrap_err()
        )
        .contains("no Claude Code data was found")
    );
}

#[test]
fn combine_gives_each_tool_the_sources_it_knows() {
    let home = Home::new();
    let app = home.folder("app");
    let extra = home.folder("extra");
    let all = home.folder("all");
    home.session(&app, SESSION);
    home.session(&extra, OTHER);
    // Cursor knows `app` only; Claude Code knows `app` and `extra`.
    let (_cursor, tools) = with_cursor(&home, &app, false);
    let args = ["combine", "-y", &text(&all), &text(&app), &text(&extra)];
    let parsed = Cli::parse_from(std::iter::once("chatkeep").chain(args.iter().copied()));
    let sources = |command: &Command| match command {
        Command::Combine(args) => args.sources.clone(),
        other => panic!("expected combine, got {other:?}"),
    };
    match chatkeep::cli::route(&tools, parsed.command.unwrap()).unwrap() {
        Route::Both {
            cursor_command,
            claude_command,
            ..
        } => {
            assert_eq!(sources(&cursor_command), [text(&app)]);
            assert_eq!(sources(&claude_command), [text(&app), text(&extra)]);
        }
        _ => panic!("combine should run for both tools"),
    }
    cli(&tools, &args).unwrap();
    assert_eq!(home.session_ids(&all).len(), 2);
}

#[test]
fn import_with_overwrite_replaces_a_chat_in_place() {
    let home = Home::new();
    let app = full_project(&home);
    let file = home.root.join("app.chatkeep");
    archive::export(&home.rt, &project(&home, &app), &file).unwrap();
    let transcript = home.project_dir(&app).join(format!("{SESSION}.jsonl"));
    let archived = read(&transcript);
    fs::write(&transcript, "changed since the export\n").unwrap();

    archive::import(&home.rt, &file, None, false).unwrap();
    assert_eq!(read(&transcript), "changed since the export\n");
    archive::import(&home.rt, &file, None, true).unwrap();
    assert_eq!(read(&transcript), archived);
    assert_eq!(home.session_ids(&app), [SESSION, OTHER]);
    assert_eq!(home.desktop_entries(ACCOUNT, ORG).len(), 1);
}

#[test]
fn cp_runs_for_both_tools_when_both_know_the_folder() {
    let home = Home::new();
    let app = home.folder("app");
    let copy = home.folder("copy");
    home.session(&app, SESSION);
    let (cursor, tools) = with_cursor(&home, &app, false);
    cli(&tools, &["cp", "-y", &text(&app), &text(&copy)]).unwrap();
    assert_eq!(home.session_ids(&copy).len(), 1);
    let copied = chatkeep::engine::discover(&common::runtime(&cursor.layout, false, false))
        .unwrap()
        .into_iter()
        .filter(|workspace| workspace.path.as_deref() == Some(copy.as_path()))
        .count();
    assert_eq!(copied, 1);
}

#[test]
fn ls_and_stats_build_the_index_and_keep_it_in_step_with_the_files() {
    let home = Home::new();
    let app = home.folder("app");
    home.session(&app, SESSION);
    home.session(&app, OTHER);
    let db = chatkeep::claude::index::db_path(&home.rt.layout.chatkeep_home);
    let tools = claude_only(&home);

    cli(&tools, &["ls"]).unwrap();
    let rows = || {
        chatkeep::claude::index::Cache::open_existing(&home.rt.layout.chatkeep_home)
            .unwrap()
            .expect("the index exists")
            .counts()
            .unwrap()
            .transcripts
    };
    assert!(db.is_file());
    assert_eq!(rows(), 2);

    // `ls <id>` and `stats` take the title and the tokens from the index.
    let found = project(&home, &app);
    let session = found.session(SESSION).unwrap();
    assert!(session.facts.is_some());
    assert_eq!(store::title(&found, session).as_deref(), Some("Work"));
    cli(&tools, &["ls", &text(&app)]).unwrap();
    cli(&tools, &["stats", "--fresh"]).unwrap();
    cli(&tools, &["claude", "cache", "stats"]).unwrap();

    // A removed session leaves the index on the next read.
    cli(&tools, &["rm", "-y", OTHER]).unwrap();
    assert_eq!(rows(), 2);
    cli(&tools, &["ls"]).unwrap();
    assert_eq!(rows(), 1);

    cli(&tools, &["claude", "cache", "clear", "-n"]).unwrap();
    assert!(db.is_file());
    cli(&tools, &["claude", "cache", "clear", "-y"]).unwrap();
    assert!(!db.is_file());
    cli(&tools, &["claude", "cache", "clear", "-y"]).unwrap();
}

#[test]
fn the_index_stays_off_when_asked() {
    let mut home = Home::new();
    let app = home.folder("app");
    home.session(&app, SESSION);
    home.rt.cache = false;
    let tools = claude_only(&home);
    cli(&tools, &["ls"]).unwrap();
    cli(&tools, &["stats"]).unwrap();
    cli(&tools, &["claude", "cache", "stats"]).unwrap();
    assert!(!chatkeep::claude::index::db_path(&home.rt.layout.chatkeep_home).exists());
    let found = project(&home, &app);
    assert!(found.sessions[0].facts.is_none());
    assert_eq!(found.sessions[0].cwd.as_deref(), Some(text(&app).as_str()));
    assert_eq!(
        store::title(&found, &found.sessions[0]).as_deref(),
        Some("Work")
    );
}
