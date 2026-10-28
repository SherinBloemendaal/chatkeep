//! `--json`: every command answers with one JSON document, on fake Cursor and Claude Code homes.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use chatkeep::cli::Tools;
use chatkeep::engine::index::{Config, Spawner};
use chatkeep::json;
use serde_json::{Value, json};

use common::claude_home;
use common::*;

fn respond(tools: &Tools, args: &[&str]) -> (Value, i32) {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let response = json::respond(&args, || Ok(tools.clone()));
    // What the binary prints is exactly this document, so it must survive a round trip.
    let text = response.body.to_string();
    let parsed: Value = serde_json::from_str(&text).unwrap();
    (parsed, response.code)
}

fn ok(tools: &Tools, args: &[&str]) -> Value {
    let (body, code) = respond(tools, args);
    assert_eq!(code, 0, "{body:#}");
    assert_eq!(body["ok"], true, "{body:#}");
    body
}

fn failed(tools: &Tools, args: &[&str]) -> (Value, i32) {
    let (body, code) = respond(tools, args);
    assert_ne!(code, 0, "{body:#}");
    assert_eq!(body["ok"], false, "{body:#}");
    (body, code)
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn cursor_only(home: &Home, running: bool) -> Tools {
    Tools::new(Some(runtime(&home.layout, running, false)), None)
}

/// Two folder workspaces: `app` with chats A and B, `gone` (deleted folder) with chat C.
fn two_workspaces(home: &Home) -> (std::path::PathBuf, std::path::PathBuf) {
    let app = home.root.join("app");
    let gone = home.root.join("gone");
    fs::create_dir_all(&app).unwrap();
    write_folder_workspace(&home.layout, "hash-app", &app);
    write_folder_workspace(&home.layout, "hash-gone", &gone);
    chat(
        &home.layout,
        A,
        "hash-app",
        &folder_identity("hash-app", &app),
    );
    chat(
        &home.layout,
        B,
        "hash-app",
        &folder_identity("hash-app", &app),
    );
    chat(
        &home.layout,
        C,
        "hash-gone",
        &folder_identity("hash-gone", &gone),
    );
    (app, gone)
}

#[test]
fn ls_lists_every_workspace_with_its_fields() {
    let home = cursor_home();
    let (app, gone) = two_workspaces(&home);
    let body = ok(&cursor_only(&home, false), &["ls", "--json"]);
    assert_eq!(body["command"], "ls");
    assert_eq!(body["dry_run"], false);
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["tool"], "cursor");
    let data = &results[0]["data"];
    assert_eq!(data["index_as_of"], Value::Null);
    let projects = data["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 2);
    // Missing destinations come first, as in the table.
    assert_eq!(projects[0]["id"], "hash-gone");
    assert_eq!(projects[0]["destination"], "missing");
    assert_eq!(projects[0]["destination_missing"], true);
    assert_eq!(projects[0]["path"], text(&gone));
    let row = &projects[1];
    assert_eq!(row["tool"], "cursor");
    assert_eq!(row["id"], "hash-app");
    assert_eq!(row["kind"], "folder");
    assert_eq!(row["path"], text(&app));
    assert_eq!(row["uri"], uri(&app));
    assert_eq!(row["install"], "default");
    assert_eq!(row["profile_label"], "default");
    assert_eq!(row["chats"], 2);
    assert_eq!(row["subagents"], 0);
    assert_eq!(row["archived"], 0);
    assert!(row["size_bytes"].is_u64());
    assert_eq!(row["destination"], "present");
    assert_eq!(row["last_active"], "2023-11-14T22:15:00Z");
    assert!(
        Path::new(row["storage_dir"].as_str().unwrap()).is_absolute(),
        "{row}"
    );
}

/// A background refresh that never starts: the test reads what the index holds.
struct NoRefresh;

impl Spawner for NoRefresh {
    fn spawn(&self, _home: &Path) -> anyhow::Result<()> {
        Ok(())
    }
}

/// When each project of `ls --json` was last active, by id.
fn last_active(body: &Value) -> Vec<(String, Value)> {
    body["results"][0]["data"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["id"].as_str().unwrap().to_string(),
                row["last_active"].clone(),
            )
        })
        .collect()
}

#[test]
fn ls_from_the_index_knows_when_each_workspace_was_last_active() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let mut rt = runtime(&home.layout, false, false);
    rt.index = Some(Config::new(Arc::new(NoRefresh)));
    let tools = Tools::new(Some(rt), None);
    let expected = vec![
        ("hash-gone".to_string(), json!("2023-11-14T22:15:00Z")),
        ("hash-app".to_string(), json!("2023-11-14T22:15:00Z")),
    ];
    // The first run builds the index, the second reads from it.
    assert_eq!(last_active(&ok(&tools, &["ls", "--json"])), expected);
    let body = ok(&tools, &["ls", "--json"]);
    assert!(
        body["results"][0]["data"]["index_as_of"].is_string(),
        "{body}"
    );
    assert_eq!(last_active(&body), expected);

    // A chat that changes shows after the index is refreshed (in the background, or here by
    // --fresh), and the refreshed index then has it too.
    global(&home.layout)
        .execute(
            "UPDATE composerHeaders SET lastUpdatedAt = 1800000000000 WHERE composerId = ?1",
            [B],
        )
        .unwrap();
    let changed = vec![
        ("hash-gone".to_string(), json!("2023-11-14T22:15:00Z")),
        ("hash-app".to_string(), json!("2027-01-15T08:00:00Z")),
    ];
    let body = ok(&tools, &["ls", "--fresh", "--json"]);
    assert_eq!(body["results"][0]["data"]["index_as_of"], Value::Null);
    assert_eq!(last_active(&body), changed);
    let body = ok(&tools, &["ls", "--json"]);
    assert!(
        body["results"][0]["data"]["index_as_of"].is_string(),
        "{body}"
    );
    assert_eq!(last_active(&body), changed);
    let body = ok(&tools, &["ls", "--json", &text(&app)]);
    let detail = &body["results"][0]["data"]["details"][0];
    assert_eq!(detail["project"]["last_active"], "2027-01-15T08:00:00Z");
    assert_eq!(detail["last_active"], "2027-01-15T08:00:00Z");

    // A chat Cursor recorded no update for counts from when it was created.
    global(&home.layout)
        .execute(
            "UPDATE composerHeaders SET lastUpdatedAt = NULL WHERE composerId = ?1",
            [C],
        )
        .unwrap();
    let body = ok(&tools, &["ls", "--fresh", "--json"]);
    assert_eq!(
        last_active(&body)[0],
        ("hash-gone".to_string(), json!("2023-11-14T22:13:20Z"))
    );
}

#[test]
fn ls_of_one_workspace_lists_its_chats() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let body = ok(&cursor_only(&home, false), &["ls", &text(&app), "--json"]);
    let details = body["results"][0]["data"]["details"].as_array().unwrap();
    assert_eq!(details.len(), 1);
    let detail = &details[0];
    assert_eq!(detail["project"]["id"], "hash-app");
    assert_eq!(detail["project"]["chats"], 2);
    assert_eq!(detail["last_active"], "2023-11-14T22:15:00Z");
    let chats = detail["chats"].as_array().unwrap();
    let mut ids: Vec<&str> = chats
        .iter()
        .map(|chat| chat["id"].as_str().unwrap())
        .collect();
    ids.sort();
    assert_eq!(ids, [A, B]);
    let first = &chats[0];
    assert_eq!(first["created_at"], "2023-11-14T22:13:20Z");
    assert_eq!(first["updated_at"], "2023-11-14T22:15:00Z");
    assert_eq!(first["archived"], false);
    assert_eq!(first["subagent"], false);
    assert_eq!(first["tokens"]["input"], 0);
}

#[test]
fn stats_returns_the_counts() {
    let home = cursor_home();
    two_workspaces(&home);
    let body = ok(&cursor_only(&home, false), &["stats", "--json"]);
    let data = &body["results"][0]["data"];
    assert_eq!(data["installations"], 1);
    assert_eq!(data["workspaces"], 2);
    assert_eq!(data["chats"], 3);
    assert_eq!(
        data["by_kind"],
        json!([{"kind": "folder", "workspaces": 2}])
    );
    assert_eq!(data["per_month"], json!([{"month": "2023-11", "chats": 3}]));
    assert_eq!(data["usage"]["conversations"], 3);
    assert_eq!(data["profiles"][0]["root"], text(&home.layout.cursor_root));
    assert!(data["global_db_bytes"].is_u64());
}

#[test]
fn history_returns_each_logged_command() {
    let home = cursor_home();
    let log = home.layout.history_file();
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(
        &log,
        concat!(
            r#"{"at":"2026-09-24T12:00:00+02:00","command":"mv","args":["/a","/b"],"outcome":"ok","duration_ms":12}"#,
            "\n",
            "not json\n",
        ),
    )
    .unwrap();
    let body = ok(&cursor_only(&home, false), &["history", "--json"]);
    let data = &body["results"][0];
    assert_eq!(data["tool"], Value::Null);
    assert_eq!(data["data"]["unreadable"], 1);
    assert_eq!(
        data["data"]["entries"],
        json!([{
            "at": "2026-09-24T10:00:00Z",
            "command": "mv",
            "args": ["/a", "/b"],
            "outcome": "ok",
            "duration_ms": 12,
        }])
    );
}

#[test]
fn mv_dry_run_returns_the_plan_and_changes_nothing() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let dest = home.root.join("moved");
    fs::create_dir_all(&dest).unwrap();
    let before = snapshot(&home);
    let body = ok(
        &cursor_only(&home, false),
        &["mv", "-n", "--json", &text(&app), &text(&dest)],
    );
    assert_eq!(body["command"], "mv");
    assert_eq!(body["dry_run"], true);
    let data = &body["results"][0]["data"];
    assert_eq!(data["title"], "Move");
    assert!(!data["applied"].as_array().unwrap().is_empty(), "{data}");
    assert!(data["warnings"].is_array());
    assert!(data["skipped"].is_array());
    assert!(data["rewritten_keys"].is_array());
    assert_same(&before, &snapshot(&home), "a dry run");
}

#[test]
fn a_write_needs_yes_or_dry_run() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let before = snapshot(&home);
    let (body, code) = failed(
        &cursor_only(&home, false),
        &["mv", "--json", &text(&app), "/elsewhere"],
    );
    assert_eq!(code, 1);
    assert_eq!(body["command"], "mv");
    assert_eq!(body["error"]["kind"], "needs_confirmation");
    assert_same(&before, &snapshot(&home), "a refused run");
}

#[test]
fn a_missing_argument_is_named_instead_of_asked() {
    let home = cursor_home();
    let (body, _) = failed(&cursor_only(&home, false), &["mv", "--json"]);
    assert_eq!(body["error"]["kind"], "missing_argument");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("FROM and TO"),
        "{body}"
    );
    let (body, _) = failed(&cursor_only(&home, false), &["queue", "rm", "--json"]);
    assert_eq!(body["error"]["kind"], "missing_argument");
}

#[test]
fn parse_errors_and_terminal_only_commands_answer_in_json() {
    let home = cursor_home();
    let tools = cursor_only(&home, false);
    let (body, code) = failed(&tools, &["--json", "mv", "--bogus"]);
    assert_eq!(code, 2);
    assert_eq!(body["command"], Value::Null);
    assert_eq!(body["error"]["kind"], "usage");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("--bogus")
    );
    for args in [
        &["update", "--json"][..],
        &["claude", "sync", "watch", "--json"],
        &["--json", "mv", "-h"],
    ] {
        let (body, _) = failed(&tools, args);
        assert_eq!(body["error"]["kind"], "unsupported_in_json", "{args:?}");
    }
    let body = ok(&tools, &["--json", "--version"]);
    assert_eq!(
        body["results"][0]["data"]["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn an_open_cursor_answers_with_the_arguments_to_queue() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let dest = home.root.join("moved");
    fs::create_dir_all(&dest).unwrap();
    let (body, code) = failed(
        &cursor_only(&home, true),
        &["mv", "-y", "--json", &text(&app), &text(&dest)],
    );
    assert_eq!(code, 75);
    assert_eq!(body["error"]["kind"], "tool_running");
    assert_eq!(
        body["error"]["queue_args"],
        json!(["mv", "-y", text(&app), text(&dest)])
    );
}

#[test]
fn queue_add_stores_the_entry_and_list_shows_it() {
    let home = cursor_home();
    let (app, _) = two_workspaces(&home);
    let dest = home.root.join("moved");
    fs::create_dir_all(&dest).unwrap();
    let tools = cursor_only(&home, true);
    let body = ok(
        &tools,
        &[
            "--json",
            "queue",
            "add",
            "--",
            "mv",
            "-y",
            &text(&app),
            &text(&dest),
        ],
    );
    assert_eq!(body["command"], "queue add");
    let entry = &body["results"][0]["data"]["entry"];
    assert_eq!(entry["position"], 1);
    assert_eq!(entry["status"], "pending");
    assert_eq!(entry["argv"], json!(["mv", "-y", text(&app), text(&dest)]));
    assert_eq!(entry["tool"], Value::Null);
    assert!(entry["created_at"].as_str().unwrap().ends_with('Z'));
    let id = entry["id"].as_str().unwrap().to_string();

    let body = ok(&tools, &["queue", "list", "--json"]);
    let entries = body["results"][0]["data"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], id.as_str());
    assert_eq!(entries[0]["interrupted"], false);

    let body = ok(&tools, &["queue", "rm", "-n", "--json", &id]);
    assert_eq!(body["dry_run"], true);
    assert_eq!(body["results"][0]["data"]["title"], "Queue remove");
    let body = ok(&tools, &["queue", "list", "--json"]);
    assert_eq!(
        body["results"][0]["data"]["entries"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// Workspace `source-hash` with chat A that touched `api`, B that touched `web`, and C that
/// touched neither.
fn monorepo(home: &Home) -> (std::path::PathBuf, std::path::PathBuf) {
    let source = home.root.join("source");
    let api = home.root.join("api");
    let web = home.root.join("web");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(api.join("src")).unwrap();
    fs::create_dir_all(web.join("src")).unwrap();
    write_folder_workspace(&home.layout, "source-hash", &source);
    let identity = folder_identity("source-hash", &source);
    let api_file = native(&api, "src/main.rs");
    fs::write(&api_file, "fn main() {}\n").unwrap();
    let web_file = native(&web, "src/new.ts");
    for id in [A, B, C] {
        insert_header(
            &home.layout,
            id,
            "source-hash",
            false,
            &json!({"name": id, "workspaceIdentifier": identity}),
        );
    }
    kv_text(
        &home.layout,
        &format!("ofsContent:{A}:{}", uri(&api_file)),
        "body",
    );
    kv_text(
        &home.layout,
        &format!("composerData:{B}"),
        &json!({"workspaceIdentifier": identity, "newlyCreatedFiles": [web_file.display().to_string()]})
            .to_string(),
    );
    for id in [A, C] {
        kv_text(
            &home.layout,
            &format!("composerData:{id}"),
            &json!({"workspaceIdentifier": identity}).to_string(),
        );
    }
    (api, web)
}

fn split_chats(body: &Value) -> Vec<(String, Vec<String>, Vec<String>)> {
    body["results"][0]["data"]["chats"]
        .as_array()
        .unwrap()
        .iter()
        .map(|chat| {
            let paths = |key: &str| -> Vec<String> {
                chat[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|path| path.as_str().unwrap().to_string())
                    .collect()
            };
            (
                chat["id"].as_str().unwrap().to_string(),
                paths("suggested"),
                paths("targets"),
            )
        })
        .collect()
}

#[test]
fn split_dry_run_shows_the_suggestion_and_assign_changes_it() {
    let home = cursor_home();
    let (api, web) = monorepo(&home);
    let tools = cursor_only(&home, false);
    let (api, web) = (text(&api), text(&web));
    let before = snapshot(&home);

    let body = ok(
        &tools,
        &["split", "-n", "--json", "source-hash", &api, &web],
    );
    assert_eq!(body["command"], "split");
    assert_eq!(
        split_chats(&body),
        [
            (A.to_string(), vec![api.clone()], vec![api.clone()]),
            (B.to_string(), vec![web.clone()], vec![web.clone()]),
            (C.to_string(), vec![], vec![]),
        ]
    );
    let warnings = body["results"][0]["data"]["warnings"].as_array().unwrap();
    assert!(
        warnings[0].as_str().unwrap().contains("--assign"),
        "{warnings:?}"
    );

    let assign_c = format!("{C}={web}");
    let assign_a = format!("{A}=");
    let body = ok(
        &tools,
        &[
            "split",
            "-n",
            "--json",
            "source-hash",
            &api,
            &web,
            "--assign",
            &assign_c,
            "--assign",
            &assign_a,
        ],
    );
    assert_eq!(
        split_chats(&body),
        [
            (A.to_string(), vec![api.clone()], vec![]),
            (B.to_string(), vec![web.clone()], vec![web.clone()]),
            (C.to_string(), vec![], vec![web.clone()]),
        ]
    );
    assert!(
        body["results"][0]["data"]["warnings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_same(&before, &snapshot(&home), "split dry runs");

    // A real run cannot leave a chat without a target.
    let (body, _) = failed(
        &tools,
        &["split", "-y", "--json", "source-hash", &api, &web],
    );
    assert_eq!(body["error"]["kind"], "missing_argument");
    let (body, _) = failed(
        &tools,
        &[
            "split",
            "-n",
            "--json",
            "source-hash",
            &api,
            &web,
            "--assign",
            &format!("{C}=/not/a/target"),
        ],
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not one of the split targets")
    );
}

const ACCOUNT_A: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const ACCOUNT_B: &str = "bbbbbbbb-2222-4222-8222-222222222222";

/// Accounts A and B of the desktop app, each listing the one chat, B signed in, both in
/// profile `work`.
fn two_accounts(home: &claude_home::Home) {
    let app = home.folder("app");
    home.session(&app, claude_home::SESSION);
    home.desktop_entry(ACCOUNT_A, "org-a", "one", claude_home::SESSION, &app);
    home.desktop_entry(ACCOUNT_B, "org-b", "two", claude_home::SESSION, &app);
    home.signed_in(ACCOUNT_B);
    chatkeep::claude::sync::set_profile(
        &home.rt,
        "work",
        &["aaaa".to_string(), "bbbb".to_string()],
    )
    .unwrap();
}

#[test]
fn claude_accounts_and_sync_profiles_are_listed() {
    let home = claude_home::Home::new();
    two_accounts(&home);
    let tools = Tools::new(None, Some(home.rt.clone()));

    let body = ok(&tools, &["claude", "accounts", "ls", "--json"]);
    assert_eq!(body["command"], "claude accounts ls");
    assert_eq!(body["results"][0]["tool"], "claude");
    let accounts = body["results"][0]["data"]["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    let b = accounts
        .iter()
        .find(|account| account["account"] == ACCOUNT_B)
        .unwrap();
    assert_eq!(b["organization"], "org-b");
    assert_eq!(b["member"], format!("{ACCOUNT_B}/org-b"));
    assert_eq!(b["chats"], 1);
    assert_eq!(b["on_disk"], 1);
    assert_eq!(b["signed_in"], true);
    assert_eq!(b["label"], "bbbbbbbb");

    let body = ok(&tools, &["claude", "sync", "profiles", "--json"]);
    let profiles = body["results"][0]["data"]["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0]["name"], "work");
    let members: Vec<&str> = profiles[0]["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|member| member["member"].as_str().unwrap())
        .collect();
    assert_eq!(
        members,
        [format!("{ACCOUNT_A}/org-a"), format!("{ACCOUNT_B}/org-b")]
    );
    assert!(body["results"][0]["data"]["auto"]["state"].is_string());

    let body = ok(&tools, &["claude", "sync", "log", "--last", "5", "--json"]);
    assert!(body["results"][0]["data"]["entries"].is_array());
}

#[test]
fn claude_ls_of_one_project_lists_its_sessions() {
    let home = claude_home::Home::new();
    let app = home.folder("app");
    home.session(&app, claude_home::SESSION);
    let tools = Tools::new(None, Some(home.rt.clone()));
    let body = ok(&tools, &["ls", "--json", &text(&app)]);
    let detail = &body["results"][0]["data"]["details"][0];
    let project = &detail["project"];
    assert_eq!(project["tool"], "claude");
    assert_eq!(project["kind"], "project");
    assert_eq!(project["path"], text(&app));
    assert_eq!(project["chats"], 1);
    assert_eq!(project["subagents"], 1);
    assert_eq!(project["last_active"], detail["last_active"]);
    assert!(project["last_active"].as_str().unwrap().ends_with('Z'));
    let session = &detail["chats"][0];
    assert_eq!(session["id"], claude_home::SESSION);
    assert_eq!(session["title"], "Work");
    assert_eq!(session["subagents"], 1);
    assert!(session["updated_at"].as_str().unwrap().ends_with('Z'));
    assert!(session["tokens"]["input"].is_u64());

    let body = ok(&tools, &["ls", "--json"]);
    let row = &body["results"][0]["data"]["projects"][0];
    assert_eq!(row["path"], text(&app));
    assert_eq!(row["last_active"], session["updated_at"]);
}

/// A Claude Code project for `folder` and a Cursor workspace for the same folder, with Cursor
/// open or closed as `probe` says.
fn both_tools(
    home: &claude_home::Home,
    folder: &Path,
    probe: Arc<dyn chatkeep::engine::Probe>,
) -> (Home, Tools) {
    let cursor = cursor_home();
    write_folder_workspace(&cursor.layout, "hash-one", folder);
    let mut rt = runtime(&cursor.layout, false, false);
    rt.cwd = home.root.clone();
    rt.probe = probe;
    let tools = Tools::new(Some(rt), Some(home.rt.clone()));
    (cursor, tools)
}

/// After Claude Code's part, an open Cursor leaves `queue_args` that queue only Cursor's part:
/// `--tool cursor` goes before the `--`, so the positionals stay what they were.
#[test]
fn queue_args_of_a_half_done_command_keep_its_positionals() {
    let home = claude_home::Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, claude_home::SESSION);
    let (_cursor, tools) = both_tools(&home, &old, Arc::new(FixedProbe(true)));

    let (body, code) = failed(
        &tools,
        &["--json", "mv", "-y", "--", &text(&old), &text(&new)],
    );
    assert_eq!(code, 75);
    assert_eq!(body["error"]["kind"], "tool_running");
    assert_eq!(body["results"][0]["tool"], "claude");
    let queued = json!(["mv", "-y", "--tool", "cursor", "--", text(&old), text(&new)]);
    assert_eq!(body["error"]["queue_args"], queued);

    // A fresh pair of homes: the mv above already moved Claude Code's project.
    let home = claude_home::Home::new();
    let old = home.folder("app");
    home.session(&old, claude_home::SESSION);
    let (_cursor, tools) = both_tools(&home, &old, Arc::new(FixedProbe(true)));
    let all = home.folder("all");
    let (body, _) = failed(
        &tools,
        &["--json", "combine", "-y", "--", &text(&all), &text(&old)],
    );
    assert_eq!(body["results"][0]["tool"], "claude");
    assert_eq!(body["error"]["kind"], "tool_running");
    let queue_args: Vec<String> =
        serde_json::from_value(body["error"]["queue_args"].clone()).unwrap();
    assert_eq!(
        queue_args,
        [
            "combine",
            "-y",
            "--tool",
            "cursor",
            "--",
            &text(&all),
            &text(&old)
        ]
    );
    match chatkeep::queue_cmd::parse_command(&queue_args).unwrap() {
        chatkeep::cli::Command::Combine(args) => {
            assert_eq!(args.common.tool, Some(chatkeep::cli::Tool::Cursor));
            assert_eq!(args.sources, [text(&old)]);
        }
        other => panic!("expected combine, got {other:?}"),
    }
}

#[test]
fn queue_execute_stores_only_cursors_part_before_the_double_dash() {
    let home = claude_home::Home::new();
    let old = home.root.join("old");
    let new = home.folder("new");
    home.session(&old, claude_home::SESSION);
    let (cursor, closed) = both_tools(&home, &old, Arc::new(FixedProbe(false)));
    ok(
        &closed,
        &[
            "--json",
            "queue",
            "add",
            "--",
            "mv",
            "-y",
            "--",
            &text(&old),
            &text(&new),
        ],
    );

    // Closed when the queue and the entry are checked, open when Cursor's part runs.
    let (_, opening) = {
        let mut rt = runtime(&cursor.layout, false, false);
        rt.cwd = home.root.clone();
        rt.probe = Arc::new(OpensAfter::new(2));
        ((), Tools::new(Some(rt), Some(home.rt.clone())))
    };
    let (body, code) = failed(&opening, &["queue", "execute", "-y", "--json"]);
    assert_eq!(code, 75);
    assert_eq!(body["error"]["queue_args"], Value::Null);
    let entry = &body["results"][0]["data"]["entries"][0];
    assert_eq!(entry["status"], "blocked");
    assert_eq!(entry["results"][0]["title"], "Move (Claude Code)");

    let body = ok(&closed, &["queue", "list", "--json"]);
    assert_eq!(
        body["results"][0]["data"]["entries"][0]["argv"],
        json!(["mv", "-y", "--tool", "cursor", "--", text(&old), text(&new)])
    );
    assert_eq!(body["results"][0]["data"]["entries"][0]["tool"], "cursor");
}

/// What the command log holds, as `history --json` shows it: command and arguments.
fn logged(tools: &Tools) -> Vec<(String, Vec<String>)> {
    let body = ok(tools, &["history", "--json"]);
    body["results"][0]["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["command"].as_str().unwrap().to_string(),
                serde_json::from_value(entry["args"].clone()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn json_runs_log_the_writes_a_text_run_logs() {
    let home = cursor_home();
    two_workspaces(&home);
    let mut rt = runtime(&home.layout, false, false);
    rt.index = Some(Config::new(Arc::new(NoRefresh)));
    let tools = Tools::new(Some(rt), None);

    ok(&tools, &["cursor", "cache", "scan", "-n", "--json"]);
    ok(&tools, &["ls", "--json"]);
    assert!(logged(&tools).is_empty());
    ok(&tools, &["cursor", "cache", "scan", "--json"]);
    assert_eq!(
        logged(&tools),
        [("cursor cache".to_string(), vec!["scan".to_string()])]
    );
}
