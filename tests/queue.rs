mod common;

use std::fs;
use std::path::{MAIN_SEPARATOR, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use chatkeep::cli::{self, Cli, Command};
use chatkeep::cursor::uri::normalize_path_in;
use chatkeep::cursor::workspace::compute_workspace_hash;
use chatkeep::engine::{self, CursorRunning, CursorRunningHint, Instance, Probe, queue};
use chatkeep::queue_cmd;
use clap::{CommandFactory, Parser, ValueHint};

use common::*;

static CWD_LOCK: Mutex<()> = Mutex::new(());

struct CwdRestore(PathBuf);

impl Drop for CwdRestore {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.0);
    }
}

fn lock_cwd() -> (MutexGuard<'static, ()>, CwdRestore) {
    let guard = CWD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    (guard, CwdRestore(std::env::current_dir().unwrap()))
}

fn stable_cwd(home: &Home) -> (MutexGuard<'static, ()>, CwdRestore) {
    let pair = lock_cwd();
    std::env::set_current_dir(&home.root).unwrap();
    pair
}

fn enter(dir: &Path) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    std::env::set_current_dir(dir).unwrap();
    std::env::current_dir().unwrap()
}

fn mkdir(path: PathBuf) -> PathBuf {
    fs::create_dir_all(&path).unwrap();
    path
}

fn run(rt: &engine::Runtime, args: &[&str]) -> anyhow::Result<()> {
    let argv = std::iter::once("chatkeep").chain(args.iter().copied());
    cli::dispatch(Cli::try_parse_from(argv).unwrap(), Some(rt.clone()))
}

fn anchored(base: &Path, relative: &str) -> String {
    format!("{}{MAIN_SEPARATOR}{relative}", base.display())
}

fn resolved(base: &Path, argv: &[&str]) -> Command {
    let argv: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    let mut command = queue_cmd::parse_command(&argv).unwrap();
    command.resolve_paths(base);
    command
}

fn path_args(command: Command) -> cli::PathArgs {
    match command {
        Command::Mv(args) | Command::Cp(args) => args,
        other => panic!("expected mv or cp, got {other:?}"),
    }
}

fn unguarded_sample(leaf: &[&str]) -> Option<Vec<String>> {
    let sample: &[&str] = match leaf {
        ["ls"] => &["ls"],
        ["history"] => &["history"],
        ["stats"] => &["stats"],
        ["cursor", "cache", "clear"] => &["cursor", "cache", "clear"],
        ["cursor", "cache", "scan"] => &["cursor", "cache", "scan"],
        ["cursor", "cache", "stats"] => &["cursor", "cache", "stats"],
        ["claude", "accounts", "ls"] => &["claude", "accounts", "ls"],
        ["claude", "accounts", "cp"] => &["claude", "accounts", "cp", "old"],
        ["claude", "cache", "clear"] => &["claude", "cache", "clear"],
        ["claude", "cache", "stats"] => &["claude", "cache", "stats"],
        ["queue", "add"] => &["queue", "add", "--", "ls"],
        ["queue", "list"] => &["queue", "list"],
        ["queue", "rm"] => &["queue", "rm", "deadbeef"],
        ["queue", "clear"] => &["queue", "clear"],
        ["queue", "retry"] => &["queue", "retry"],
        ["queue", "execute"] => &["queue", "execute", "-n"],
        ["update"] => &["update"],
        ["github"] => &["github"],
        ["uninstall"] => &["uninstall", "-n"],
        ["help"] => &["help"],
        _ => return None,
    };
    Some(sample.iter().map(|arg| arg.to_string()).collect())
}

fn guarded_sample(leaf: &[&str], from: &str, to: &str, archive: &str) -> Option<Vec<String>> {
    let sample: Vec<&str> = match leaf {
        ["mv"] => vec!["mv", from, to],
        ["cp"] => vec!["cp", from, to],
        ["split"] => vec!["split", from, to],
        ["combine"] => vec!["combine", to, from],
        ["rm"] => vec!["rm", "hash-from"],
        ["import"] => vec!["import", archive],
        ["export"] => vec!["export", "hash-from", archive],
        ["cursor", "save"] => vec!["cursor", "save", "id", to],
        ["cursor", "rx"] => vec!["cursor", "rx", "hash-from"],
        _ => return None,
    };
    Some(sample.into_iter().map(str::to_string).collect())
}

#[test]
fn resolve_keeps_a_spec_that_equals_an_earlier_flag_value() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    fs::create_dir_all(base.join("default")).unwrap();
    fs::create_dir_all(base.join("b")).unwrap();
    for argv in [
        ["mv", "--profile", "default", "default", "b"].as_slice(),
        &["mv", "--profile=default", "default", "b"],
        &["mv", "--profile", "default", "--", "default", "b"],
        &["mv", "default", "b", "--profile", "default"],
    ] {
        let args = path_args(resolved(base, argv));
        assert_eq!(args.common.profile.as_deref(), Some("default"), "{argv:?}");
        assert_eq!(args.from.as_deref(), Some("default"), "{argv:?}");
        assert_eq!(args.to, Some(anchored(base, "b")), "{argv:?}");
    }
}

#[test]
fn resolve_anchors_values_after_double_dash_that_look_like_flags() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let args = path_args(resolved(
        base,
        &["cp", "-y", "--", "--from-like-flag", "--to-like-flag"],
    ));
    assert!(args.common.yes);
    assert_eq!(args.from.as_deref(), Some("--from-like-flag"));
    assert_eq!(args.to, Some(anchored(base, "--to-like-flag")));
}

#[test]
fn resolve_leaves_repeated_replace_values_alone() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let args = path_args(resolved(
        base,
        &[
            "mv",
            "--replace",
            "b",
            "b",
            "--replace",
            "b",
            "b",
            "default",
            "b",
        ],
    ));
    assert_eq!(args.common.replace, ["b", "b", "b", "b"]);
    assert_eq!(args.from.as_deref(), Some("default"));
    assert_eq!(args.to, Some(anchored(base, "b")));

    let args = path_args(resolved(
        base,
        &[
            "mv",
            "--replace",
            "older/",
            "upgraded/",
            "--profile",
            "workshop",
        ],
    ));
    assert_eq!(args.common.replace, ["older/", "upgraded/"]);
    assert_eq!(args.from, None);
    assert_eq!(args.to, None);
}

#[test]
fn resolve_anchors_every_split_target_and_keeps_combine_specs() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let Command::Split(args) = resolved(
        base,
        &[
            "split",
            "--profile",
            "t1",
            "src",
            "t1",
            "--move",
            "t2",
            "t1",
        ],
    ) else {
        panic!("expected split");
    };
    assert_eq!(args.common.profile.as_deref(), Some("t1"));
    assert_eq!(args.source.as_deref(), Some("src"));
    assert_eq!(
        args.targets,
        [
            anchored(base, "t1"),
            anchored(base, "t2"),
            anchored(base, "t1")
        ]
    );
    assert!(args.move_chats);

    let Command::Combine(args) = resolved(base, &["combine", "target", "s1", "--move", "s2"])
    else {
        panic!("expected combine");
    };
    assert_eq!(args.target.as_deref(), Some("target"));
    assert_eq!(args.sources, ["s1", "s2"]);
    assert!(args.move_chats);
}

#[test]
fn resolve_leaves_absolute_values_byte_identical() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let absolute = base.join("x").join("..").join("b").display().to_string();
    let archive = base.join("in.chatkeep").display().to_string();
    let args = path_args(resolved(base, &["mv", "default", &absolute]));
    assert_eq!(args.to.as_deref(), Some(absolute.as_str()));
    let Command::Import(args) = resolved(base, &["import", &archive, &absolute]) else {
        panic!("expected import");
    };
    assert_eq!(args.file, Some(archive));
    assert_eq!(args.to, Some(absolute));
}

#[test]
fn resolve_treats_tilde_like_a_direct_run() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let elsewhere = tempfile::tempdir().unwrap();
    let Command::Export(args) = resolved(base, &["export", "~/app", "~/app.chatkeep"]) else {
        panic!("expected export");
    };
    assert_eq!(args.target.as_deref(), Some("~/app"));
    let file = args.file.unwrap();
    assert_eq!(file, anchored(base, "~/app.chatkeep"));
    assert_eq!(
        normalize_path_in(Path::new(&file), elsewhere.path()),
        normalize_path_in(Path::new("~/app.chatkeep"), base)
    );
}

#[cfg(windows)]
#[test]
fn resolve_matches_direct_windows_drive_and_unc_handling() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let elsewhere = tempfile::tempdir().unwrap();
    for absolute in [
        r"C:\x",
        "C:/x",
        "C:",
        r"\\server\share\y",
        "//server/share/y",
    ] {
        let args = path_args(resolved(base, &["mv", "src", absolute]));
        assert_eq!(args.to.as_deref(), Some(absolute), "{absolute}");
    }
    for relative in [r"\rooted", r"sub\dir", "C:drive-relative"] {
        let to = path_args(resolved(base, &["mv", "src", relative]))
            .to
            .unwrap();
        assert_eq!(to, anchored(base, relative), "{relative}");
        assert_eq!(
            normalize_path_in(Path::new(&to), elsewhere.path()),
            normalize_path_in(Path::new(relative), base),
            "{relative}"
        );
    }
}

#[cfg(unix)]
#[test]
fn resolve_treats_windows_looking_values_as_relative_on_unix() {
    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let elsewhere = tempfile::tempdir().unwrap();
    for value in [r"C:\x", r"\\server\share\y"] {
        let to = path_args(resolved(base, &["mv", "src", value])).to.unwrap();
        assert_eq!(to, anchored(base, value), "{value}");
        assert_eq!(
            normalize_path_in(Path::new(&to), elsewhere.path()),
            normalize_path_in(Path::new(value), base),
            "{value}"
        );
    }
}

#[test]
fn every_path_arg_of_every_leaf_resolves_against_the_stored_cwd() {
    fn leaves(
        cmd: &clap::Command,
        prefix: Vec<String>,
        out: &mut Vec<(Vec<String>, clap::Command)>,
    ) {
        let subs: Vec<&clap::Command> = cmd.get_subcommands().collect();
        if subs.is_empty() {
            out.push((prefix, cmd.clone()));
            return;
        }
        for sub in subs {
            let mut next = prefix.clone();
            next.push(sub.get_name().to_string());
            leaves(sub, next, out);
        }
    }

    let base = tempfile::tempdir().unwrap();
    let base = base.path();
    let mut root = Cli::command();
    root.build();
    let mut found = Vec::new();
    leaves(&root, Vec::new(), &mut found);
    let (mut anchored_args, mut verbatim_args) = (0, 0);
    for (leaf, cmd) in &found {
        for arg in cmd.get_arguments() {
            let hint = arg.get_value_hint();
            let filesystem = matches!(
                hint,
                ValueHint::FilePath | ValueHint::DirPath | ValueHint::ExecutablePath
            );
            if !filesystem && hint != ValueHint::AnyPath {
                continue;
            }
            let sentinel = format!("sentinel-{}-{}", leaf.join("-"), arg.get_id());
            let mut argv = leaf.clone();
            if arg.is_positional() {
                for earlier in cmd
                    .get_positionals()
                    .take_while(|earlier| earlier.get_id() != arg.get_id())
                {
                    argv.push(format!("filler-{}", earlier.get_id()));
                }
            } else {
                argv.push(format!("--{}", arg.get_long().unwrap()));
            }
            argv.push(sentinel.clone());
            for other in cmd.get_arguments().filter(|other| {
                other.is_required_set()
                    && other.get_id() != arg.get_id()
                    && (!other.is_positional()
                        || !arg.is_positional()
                        || other.get_index() > arg.get_index())
            }) {
                if !other.is_positional() {
                    argv.push(format!("--{}", other.get_long().unwrap()));
                }
                argv.push(format!("filler-{}", other.get_id()));
            }
            let mut command = queue_cmd::parse_command(&argv)
                .unwrap_or_else(|err| panic!("{argv:?} does not parse: {err:#}"));
            command.resolve_paths(base);
            let debug = format!("{command:?}");
            let verbatim = format!("{sentinel:?}");
            let absolute = format!("{:?}", anchored(base, &sentinel));
            if filesystem {
                assert!(
                    debug.contains(&absolute) && !debug.replace(&absolute, "").contains(&sentinel),
                    "`{}` on {leaf:?} is a {hint:?} arg; anchor it in Command::resolve_paths: {debug}",
                    arg.get_id()
                );
                anchored_args += 1;
            } else {
                assert!(
                    debug.contains(&verbatim) && !debug.contains(&absolute),
                    "`{}` on {leaf:?} is an AnyPath workspace spec (id or path) and must stay verbatim: {debug}",
                    arg.get_id()
                );
                verbatim_args += 1;
            }
        }
    }
    assert_eq!((anchored_args, verbatim_args), (7, 9));
}

#[test]
fn typed_cursor_running_always_exits_75_for_any_argv() {
    let err = anyhow::Error::new(CursorRunning::for_command(
        "Cursor is running: default (pid 1).",
    ));
    let argv = vec!["ls".into()];
    let report =
        queue_cmd::cursor_blocked_report(err.downcast_ref::<CursorRunning>().unwrap(), &argv);
    assert_eq!(queue_cmd::report_failure_with(&err, &argv), 75);
    assert_eq!(
        report.stable,
        "chatkeep: cursor-running; queue with: chatkeep queue add -- ls"
    );
}

#[test]
fn blocked_write_prints_hint_and_exits_75() {
    let err = anyhow::Error::new(CursorRunning::for_command(
        "Cursor is running: default (pid 42).",
    ));
    let argv = vec![
        "mv".into(),
        "--replace".into(),
        "/Machines/older/".into(),
        "/Machines/upgraded/".into(),
        "--profile".into(),
        "workshop".into(),
    ];
    let code = queue_cmd::report_failure_with(&err, &argv);
    assert_eq!(code, 75);
    let report =
        queue_cmd::cursor_blocked_report(err.downcast_ref::<CursorRunning>().unwrap(), &argv);
    assert_eq!(report.summary, "Cursor is running: default (pid 42).");
    assert_eq!(
        report.hint,
        "Queue it with: chatkeep queue add -- mv --replace /Machines/older/ /Machines/upgraded/ --profile workshop"
    );
    assert_eq!(
        report.stable,
        "chatkeep: cursor-running; queue with: chatkeep queue add -- mv --replace /Machines/older/ /Machines/upgraded/ --profile workshop"
    );
}

#[test]
fn every_clap_guarded_subcommand_exits_75_with_stable_line() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();
    let archive = home.root.join("a.chatkeep").display().to_string();
    let mut guarded = 0;

    fn walk_leaves(cmd: &clap::Command, prefix: Vec<String>, visit: &mut dyn FnMut(&[String])) {
        let subs: Vec<_> = cmd
            .get_subcommands()
            .filter(|sub| !sub.is_hide_set())
            .collect();
        if subs.is_empty() {
            visit(&prefix);
            return;
        }
        for sub in subs {
            let mut next = prefix.clone();
            next.push(sub.get_name().to_string());
            walk_leaves(sub, next, visit);
        }
    }

    walk_leaves(&Cli::command(), Vec::new(), &mut |leaf| {
        let refs: Vec<&str> = leaf.iter().map(String::as_str).collect();
        if let Some(sample) = guarded_sample(&refs, &from_s, &to_s, &archive) {
            let dry = runtime(&home.layout, false, true);
            assert!(
                queue_cmd::dry_run_reaches_guard(&dry, &sample).unwrap(),
                "{leaf:?} should reach the Cursor guard on dry-run preflight"
            );
            let rt = runtime(&home.layout, true, false);
            let argv_refs: Vec<&str> = sample.iter().map(|s| s.as_str()).collect();
            let result = run(&rt, &argv_refs);
            let Err(err) = result else {
                panic!("{leaf:?}: expected CursorRunning, got Ok");
            };
            let running = err
                .downcast_ref::<CursorRunning>()
                .unwrap_or_else(|| panic!("{leaf:?}: expected CursorRunning, got {err:#}"));
            assert!(matches!(running.hint, CursorRunningHint::QueueAdd));
            let report = queue_cmd::cursor_blocked_report(running, &sample);
            assert_eq!(
                queue_cmd::report_failure_with(&err, &sample),
                75,
                "{leaf:?}"
            );
            assert_eq!(
                report.stable,
                format!(
                    "chatkeep: cursor-running; queue with: {}",
                    queue::queue_add_command(&sample)
                ),
                "{leaf:?}"
            );
            guarded += 1;
            return;
        }
        let sample = unguarded_sample(&refs).unwrap_or_else(|| {
            panic!("leaf `{leaf:?}` has neither a guarded nor an unguarded sample")
        });
        let rt = runtime(&home.layout, false, true);
        assert!(
            !queue_cmd::dry_run_reaches_guard(&rt, &sample).unwrap(),
            "{leaf:?} is listed as unguarded but dry-run preflight reached the Cursor guard"
        );
    });
    assert_eq!(guarded, 9);
}

#[test]
fn every_value_taking_arg_has_an_explicit_value_hint() {
    fn walk(cmd: &clap::Command) {
        for arg in cmd.get_arguments() {
            if arg.get_num_args().map(|range| range.max_values() == 0) == Some(true) {
                continue;
            }
            if arg.is_positional() && arg.get_id() == "command" && cmd.get_name() == "chatkeep" {
                continue;
            }
            let takes_value = arg
                .get_num_args()
                .map(|range| range.max_values() > 0)
                .unwrap_or(false)
                || arg.get_action().takes_values();
            if !takes_value {
                continue;
            }
            assert_ne!(
                arg.get_value_hint(),
                clap::ValueHint::Unknown,
                "arg `{}` on `{}` has ValueHint::Unknown; mark it as a path hint (AnyPath/FilePath/DirPath/ExecutablePath) or ValueHint::Other",
                arg.get_id(),
                cmd.get_name()
            );
        }
        for sub in cmd.get_subcommands() {
            walk(sub);
        }
    }
    walk(&Cli::command());
}

#[test]
fn queue_add_command_round_trips_argv() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let rt = runtime(&home.layout, true, false);
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();
    run(
        &rt,
        &[
            "queue",
            "add",
            "--",
            "mv",
            &from_s,
            &to_s,
            "--profile",
            "default",
        ],
    )
    .unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].argv,
        vec![
            "mv".to_string(),
            from_s,
            to_s,
            "--profile".to_string(),
            "default".to_string()
        ]
    );
    assert_eq!(entries[0].profile.as_deref(), Some("default"));
    assert_eq!(entries[0].status, queue::Status::Pending);
}

#[test]
fn queue_add_rejects_unguarded_command() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    let err = run(&rt, &["queue", "add", "--", "ls"]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "ls does not need Cursor closed and can run directly. Run the command without queue add."
    );
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
}

#[test]
fn queue_add_refuses_missing_args_with_real_error() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    let err = run(&rt, &["queue", "add", "--", "mv"]).unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "refusing to queue: mv needs FROM and TO, or --replace FROM TO. Fix the arguments and retry queue add."
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 1);
}

#[test]
fn queue_add_refuses_unknown_profile_with_real_error() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    let err = run(
        &rt,
        &[
            "queue",
            "add",
            "--",
            "mv",
            "/tmp/a",
            "/tmp/b",
            "--profile",
            "definitely-missing-profile-xyz",
        ],
    )
    .unwrap_err();
    assert_eq!(
        format!("{err:#}"),
        "refusing to queue: no Cursor profile named definitely-missing-profile-xyz. Known profiles: default. Fix the arguments and retry queue add."
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 1);
}

#[test]
fn queue_add_clap_parse_error_exits_2() {
    let err = match queue_cmd::parse_command(&["mv".into(), "--not-a-real-flag-xyz".into()]) {
        Ok(_) => panic!("expected clap parse error"),
        Err(err) => err,
    };
    assert!(
        err.downcast_ref::<queue_cmd::ClapParseError>().is_some(),
        "{err:#}"
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 2);
}

#[test]
fn queue_add_rejects_nested_queue() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    for nested in [
        ["queue", "add", "--", "queue", "list"].as_slice(),
        &["queue", "add", "--", "--color", "never", "queue", "list"],
    ] {
        let err = run(&rt, nested).unwrap_err();
        assert_eq!(err.to_string(), "cannot nest queue commands", "{nested:?}");
    }
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
}

#[test]
fn queue_execute_refuses_while_cursor_is_running() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, true, false);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "missing".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    let running = err
        .downcast_ref::<CursorRunning>()
        .unwrap_or_else(|| panic!("{err:#}"));
    assert!(matches!(
        running.hint,
        CursorRunningHint::QueueExecute { pending: 1 }
    ));
    let report = queue_cmd::cursor_blocked_report(running, &["queue".into(), "execute".into()]);
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 75);
    assert_eq!(
        report.stable,
        "chatkeep: cursor-running; queued: 1 pending; run later: chatkeep queue execute"
    );
    assert_eq!(
        report.hint,
        "Close Cursor, then run: chatkeep queue execute"
    );
}

#[test]
fn queue_execute_runs_in_order_stops_and_continues() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let mid = mkdir(home.root.join("mid"));
    let other = mkdir(home.root.join("other"));
    let dest = mkdir(home.root.join("dest"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    write_folder_workspace(&home.layout, "hash-other", &other);
    let rt = runtime(&home.layout, false, false);
    let from_s = from.display().to_string();
    let mid_s = mid.display().to_string();
    let other_s = other.display().to_string();
    let dest_s = dest.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &mid_s]).unwrap();
    run(&rt, &["queue", "add", "--", "rm", "hash-from"]).unwrap();
    run(&rt, &["queue", "add", "--", "mv", &other_s, &dest_s]).unwrap();
    let ids: Vec<String> = queue::load(&home.layout.chatkeep_home)
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();

    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    assert_eq!(err.to_string(), "queue execute stopped after a failure");
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, ids[1]);
    assert_eq!(entries[0].status, queue::Status::Failed);
    assert_eq!(
        entries[0].last_error.as_deref(),
        Some("no workspace or chat matches hash-from")
    );
    assert_eq!(entries[1].id, ids[2]);
    assert_eq!(entries[1].status, queue::Status::Pending);
    assert!(
        home.layout
            .workspace_storage()
            .join(compute_workspace_hash(&mid).unwrap())
            .exists()
    );

    run(&rt, &["queue", "execute", "-y", "--continue-on-error"]).unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, ids[1]);
    assert_eq!(entries[0].status, queue::Status::Failed);
    assert!(
        home.layout
            .workspace_storage()
            .join(compute_workspace_hash(&dest).unwrap())
            .exists()
    );
}

struct OpensAfterFirstEntry {
    home: PathBuf,
}

impl Probe for OpensAfterFirstEntry {
    fn instances(&self) -> anyhow::Result<Vec<Instance>> {
        Ok(if queue::load(&self.home)?.len() == 1 {
            vec![Instance {
                pid: 99,
                name: "Cursor".into(),
                user_data_dir: None,
                main: true,
            }]
        } else {
            Vec::new()
        })
    }
}

#[test]
fn queue_execute_stops_mid_run_when_cursor_appears() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let mid = mkdir(home.root.join("mid"));
    let other = mkdir(home.root.join("other"));
    let dest = mkdir(home.root.join("dest"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    write_folder_workspace(&home.layout, "hash-other", &other);
    let probe = Arc::new(OpensAfterFirstEntry {
        home: home.layout.chatkeep_home.clone(),
    });
    let rt = runtime_with(&home.layout, probe, false);
    let from_s = from.display().to_string();
    let mid_s = mid.display().to_string();
    let other_s = other.display().to_string();
    let dest_s = dest.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &mid_s]).unwrap();
    run(&rt, &["queue", "add", "--", "mv", &other_s, &dest_s]).unwrap();
    let ids: Vec<String> = queue::load(&home.layout.chatkeep_home)
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();

    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    let running = err
        .downcast_ref::<CursorRunning>()
        .unwrap_or_else(|| panic!("{err:#}"));
    assert!(matches!(
        running.hint,
        CursorRunningHint::QueueExecute { pending: 1 }
    ));
    let report = queue_cmd::cursor_blocked_report(running, &[]);
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 75);
    assert_eq!(
        report.summary,
        "Cursor is running: default (Cursor, pid 99)."
    );
    assert_eq!(
        report.stable,
        "chatkeep: cursor-running; queued: 1 pending; run later: chatkeep queue execute"
    );
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, ids[1]);
    assert_eq!(entries[0].status, queue::Status::Pending);
    assert_eq!(entries[0].last_error, None);
    let storage = home.layout.workspace_storage();
    assert!(storage.join(compute_workspace_hash(&mid).unwrap()).exists());
    assert!(storage.join("hash-other").exists());
    assert!(
        !storage
            .join(compute_workspace_hash(&dest).unwrap())
            .exists()
    );
}

#[test]
fn queue_execute_dry_run_writes_nothing() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let rt = runtime(&home.layout, false, true);
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &to_s]).unwrap();
    let before = snapshot(&home);
    run(&rt, &["queue", "execute", "-y", "-n"]).unwrap();
    assert_same(&before, &snapshot(&home), "queue execute -n");
    assert_eq!(
        queue::load(&home.layout.chatkeep_home).unwrap()[0].status,
        queue::Status::Pending
    );
}

#[test]
fn queue_execute_dry_run_allows_cursor_running_and_keeps_queue_bytes() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let rt = runtime(&home.layout, true, true);
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &to_s]).unwrap();
    let path = queue::queue_path(&home.layout.chatkeep_home);
    let before = fs::read(&path).unwrap();
    run(&rt, &["queue", "execute", "-y", "-n"]).unwrap();
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn queue_rm_clear_retry_and_lock() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    let (first, _) = queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "a".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let (second, _) = queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "b".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    queue::update(
        &home.layout.chatkeep_home,
        &first.id,
        queue::Status::Failed,
        Some("boom".into()),
    )
    .unwrap();
    run(&rt, &["queue", "rm", "-y", &first.id]).unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, second.id);

    queue::update(
        &home.layout.chatkeep_home,
        &second.id,
        queue::Status::Skipped,
        Some("gone".into()),
    )
    .unwrap();
    run(&rt, &["queue", "retry", "-y", &second.id]).unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries[0].status, queue::Status::Pending);
    assert_eq!(entries[0].last_error, None);

    queue::update(
        &home.layout.chatkeep_home,
        &second.id,
        queue::Status::Failed,
        Some("again".into()),
    )
    .unwrap();
    run(&rt, &["queue", "retry", "-y"]).unwrap();
    assert_eq!(
        queue::load(&home.layout.chatkeep_home).unwrap()[0].status,
        queue::Status::Pending
    );

    run(&rt, &["queue", "clear", "-y"]).unwrap();
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());

    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "c".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let lock = queue::Lock::acquire(&home.layout.chatkeep_home)
        .unwrap()
        .unwrap();
    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "another chatkeep queue command is running (pid {}). Wait for it to finish and retry.",
            std::process::id()
        )
    );
    drop(lock);
}

#[test]
fn queue_execute_uses_stored_cwd_for_relative_paths() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    let work = enter(&home.root.join("work"));
    let from = mkdir(work.join("from"));
    let to = mkdir(work.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let rt = runtime(&home.layout, false, false);
    run(&rt, &["queue", "add", "--", "mv", "from", "to"]).unwrap();
    std::env::set_current_dir(&other).unwrap();
    let entry = &queue::load(&home.layout.chatkeep_home).unwrap()[0];
    assert_eq!(entry.cwd, work);
    assert_eq!(
        entry.argv,
        vec!["mv".to_string(), "from".into(), "to".into()]
    );
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert_eq!(std::env::current_dir().unwrap(), other);
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
    assert!(
        home.layout
            .workspace_storage()
            .join(compute_workspace_hash(&to).unwrap())
            .exists()
    );
}

#[test]
fn queue_execute_resolves_the_profile_collision_against_the_stored_cwd() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    let decoy = mkdir(other.join("default"));
    mkdir(other.join("b"));
    let work = enter(&home.root.join("work"));
    let source = mkdir(work.join("default"));
    let dest = mkdir(work.join("b"));
    write_folder_workspace(&home.layout, "hash-source", &source);
    write_folder_workspace(&home.layout, "hash-decoy", &decoy);
    let rt = runtime(&home.layout, false, false);
    let argv = ["mv", "--profile", "default", "default", "b"];
    run(&rt, &[["queue", "add", "--"].as_slice(), &argv].concat()).unwrap();
    assert_eq!(
        queue::load(&home.layout.chatkeep_home).unwrap()[0].argv,
        argv
    );
    std::env::set_current_dir(&other).unwrap();
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
    let storage = home.layout.workspace_storage();
    assert!(
        storage
            .join(compute_workspace_hash(&dest).unwrap())
            .exists()
    );
    assert!(!storage.join("hash-source").exists());
    assert!(storage.join("hash-decoy").exists());
}

#[test]
fn queue_execute_finds_a_renamed_folder_by_its_old_relative_path() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    let decoy = mkdir(other.join("old"));
    mkdir(other.join("new"));
    let work = enter(&home.root.join("work"));
    let new = mkdir(work.join("new"));
    write_folder_workspace(&home.layout, "hash-old", &work.join("old"));
    write_folder_workspace(&home.layout, "hash-decoy", &decoy);
    let rt = runtime(&home.layout, false, false);
    run(&rt, &["queue", "add", "--", "mv", "old", "new"]).unwrap();
    std::env::set_current_dir(&other).unwrap();
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
    let storage = home.layout.workspace_storage();
    assert!(storage.join(compute_workspace_hash(&new).unwrap()).exists());
    assert!(!storage.join("hash-old").exists());
    assert!(storage.join("hash-decoy").exists());
}

#[test]
fn queue_execute_combines_into_a_relative_target_under_the_stored_cwd() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    let decoy = mkdir(other.join("target"));
    mkdir(other.join("source"));
    let work = enter(&home.root.join("work"));
    let source = mkdir(work.join("source"));
    let target = mkdir(work.join("target"));
    write_folder_workspace(&home.layout, "hash-source", &source);
    chat(
        &home.layout,
        A,
        "hash-source",
        &folder_identity("hash-source", &source),
    );
    let rt = runtime(&home.layout, false, false);
    run(
        &rt,
        &[
            "queue", "add", "--", "combine", "target", "source", "--move",
        ],
    )
    .unwrap();
    std::env::set_current_dir(&other).unwrap();
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
    assert_eq!(
        header_ids(&home.layout, &compute_workspace_hash(&target).unwrap()),
        [A]
    );
    assert!(header_ids(&home.layout, "hash-source").is_empty());
    assert!(
        !home
            .layout
            .workspace_storage()
            .join(compute_workspace_hash(&decoy).unwrap())
            .exists()
    );
}

#[test]
fn queue_execute_keeps_workspace_ids_verbatim() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    let work = enter(&home.root.join("work"));
    let folder = mkdir(work.join("x"));
    write_folder_workspace(&home.layout, "hash-x", &folder);
    let rt = runtime(&home.layout, false, false);
    run(&rt, &["queue", "add", "--", "rm", "hash-x"]).unwrap();
    std::env::set_current_dir(&other).unwrap();
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert!(queue::load(&home.layout.chatkeep_home).unwrap().is_empty());
    assert!(!home.layout.workspace_storage().join("hash-x").exists());
}

#[test]
fn queue_execute_plans_in_parallel_but_keeps_each_result_with_its_entry() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let rt = runtime(&home.layout, false, false);
    let mut gone = Vec::new();
    let mut moved = Vec::new();
    for index in 0..12 {
        if index % 5 == 3 {
            let cwd = mkdir(home.root.join(format!("gone-{index}")));
            queue::add(
                &home.layout.chatkeep_home,
                vec!["rm".into(), "hash".into()],
                cwd.clone(),
                None,
                Vec::new(),
            )
            .unwrap();
            gone.push(cwd);
        } else {
            let from = mkdir(home.root.join(format!("from-{index}")));
            let to = mkdir(home.root.join(format!("to-{index}")));
            write_folder_workspace(&home.layout, &format!("hash-{index}"), &from);
            let from_s = from.display().to_string();
            let to_s = to.display().to_string();
            run(&rt, &["queue", "add", "--", "mv", &from_s, &to_s]).unwrap();
            moved.push(to);
        }
    }
    let ids: Vec<String> = queue::load(&home.layout.chatkeep_home)
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    for cwd in &gone {
        fs::remove_dir_all(cwd).unwrap();
    }

    let err = run(&rt, &["queue", "execute", "-y", "--continue-on-error"]).unwrap_err();
    assert_eq!(err.to_string(), "queue execute finished with failures");
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    let left: Vec<(&str, queue::Status, Option<&str>)> = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry.status, entry.last_error.as_deref()))
        .collect();
    let expected: Vec<String> = gone
        .iter()
        .map(|cwd| format!("queued cwd no longer exists: {}", cwd.display()))
        .collect();
    assert_eq!(
        left,
        [
            (
                ids[3].as_str(),
                queue::Status::Skipped,
                Some(expected[0].as_str())
            ),
            (
                ids[8].as_str(),
                queue::Status::Skipped,
                Some(expected[1].as_str())
            ),
        ]
    );
    for to in &moved {
        assert!(
            home.layout
                .workspace_storage()
                .join(compute_workspace_hash(to).unwrap())
                .exists(),
            "{}",
            to.display()
        );
    }
}

#[test]
fn queue_execute_marks_missing_cwd_as_skipped() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let gone = mkdir(home.root.join("gone-cwd"));
    let rt = runtime(&home.layout, false, false);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "hash".into()],
        gone.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    fs::remove_dir_all(&gone).unwrap();
    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    assert_eq!(err.to_string(), "queue execute finished with failures");
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, queue::Status::Skipped);
    let expected = format!("queued cwd no longer exists: {}", gone.display());
    assert_eq!(entries[0].last_error.as_deref(), Some(expected.as_str()));
}

#[test]
fn queue_jsonl_append_keeps_prior_bytes() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let path = queue::queue_path(&home.layout.chatkeep_home);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "one".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let before = fs::read(&path).unwrap();
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "two".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let after = fs::read(&path).unwrap();
    assert_eq!(&after[..before.len()], before.as_slice());
    assert!(after[before.len()..].ends_with(b"\n"));
}

#[test]
fn queue_jsonl_ignores_blank_lines() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let path = queue::queue_path(&home.layout.chatkeep_home);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "a".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let body = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("\n\n{body}\n\n")).unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].argv, vec!["rm".to_string(), "a".into()]);
}

#[test]
fn queue_jsonl_rejects_unknown_status() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let path = queue::queue_path(&home.layout.chatkeep_home);
    fs::create_dir_all(&home.layout.chatkeep_home).unwrap();
    fs::write(
        &path,
        concat!(
            r#"{"schema":1,"id":"deadbeef","argv":["rm","x"],"cwd":"/tmp","created_at":"2026-01-01T00:00:00Z","tool_version":"1.0.0","status":"done"}"#,
            "\n"
        ),
    )
    .unwrap();
    let err = queue::load(&home.layout.chatkeep_home)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        format!(
            "queue.jsonl line 1 is malformed in {}: unknown variant `done`, expected one of `pending`, `running`, `failed`, `skipped` at line 1 column 133",
            path.display()
        )
    );
}

#[test]
fn queue_jsonl_rejects_truncated_malformed_and_unknown_schema() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let path = queue::queue_path(&home.layout.chatkeep_home);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "a".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "b".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();

    let good = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = good.lines().collect();
    lines.insert(1, "{broken");
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    let err = queue::load(&home.layout.chatkeep_home)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        format!(
            "queue.jsonl line 2 is malformed in {}: key must be a string at line 1 column 2",
            path.display()
        )
    );

    fs::write(&path, "{\"schema\":1,\"id\":\"x\"").unwrap();
    let err = queue::load(&home.layout.chatkeep_home)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        format!(
            "queue.jsonl line 1 is truncated (incomplete write) in {}: EOF while parsing an object at line 1 column 20",
            path.display()
        )
    );

    fs::write(
        &path,
        concat!(
            r#"{"schema":99,"id":"deadbeef","argv":["rm","x"],"cwd":"/tmp","created_at":"2026-01-01T00:00:00Z","tool_version":"1.0.0","status":"pending"}"#,
            "\n"
        ),
    )
    .unwrap();
    let err = queue::load(&home.layout.chatkeep_home)
        .unwrap_err()
        .to_string();
    assert_eq!(
        err,
        format!(
            "unsupported queue schema 99 on line 1 of {} (want 1)",
            path.display()
        )
    );
}

#[test]
fn queue_execute_skips_when_live_replan_fails() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    let workspace = write_folder_workspace(&home.layout, "hash-from", &from);
    let rt = runtime(&home.layout, false, false);
    let from_s = from.display().to_string();
    let to_s = to.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &to_s]).unwrap();
    fs::remove_dir_all(&from).unwrap();
    fs::remove_dir_all(&workspace).unwrap();
    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    assert_eq!(err.to_string(), "queue execute finished with failures");
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, queue::Status::Skipped);
    let expected = format!("no workspace matches {from_s}");
    assert_eq!(entries[0].last_error.as_deref(), Some(expected.as_str()));
}

#[test]
fn suggested_queue_command_is_shell_safe() {
    let cmd = queue::queue_add_command(&[
        "mv".into(),
        "--replace".into(),
        "/Machines/older/".into(),
        "/Machines/upgraded/".into(),
        "--profile".into(),
        "workshop".into(),
    ]);
    assert_eq!(
        cmd,
        "chatkeep queue add -- mv --replace /Machines/older/ /Machines/upgraded/ --profile workshop"
    );
}

#[test]
fn dry_run_mv_succeeds_while_cursor_is_running() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let to = mkdir(home.root.join("to"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    let before = snapshot(&home);
    let rt = runtime(&home.layout, true, true);
    let report = engine::move_paths(
        &rt,
        &[("hash-from".into(), to.display().to_string())],
        None,
        false,
        false,
    )
    .unwrap();
    // The plan shows the destination the way Cursor spells it (lowercase drive on Windows).
    let shown = normalize_path_in(&to, &home.root);
    assert_eq!(
        report.applied,
        [format!(
            "dry-run hash-from -> {} (0 chats)",
            shown.display()
        )]
    );
    assert_same(&before, &snapshot(&home), "dry-run with cursor running");
}

#[test]
fn queue_add_refuses_version_flag() {
    let err = match queue_cmd::parse_command(&["-V".into()]) {
        Ok(command) => panic!("expected refusal, got {command:?}"),
        Err(err) => err,
    };
    assert_eq!(err.to_string(), "queue add needs a write command after --");
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 1);
}

#[test]
fn queue_add_refuses_ambiguous_export_with_the_direct_error() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let other = install_layout(&home.root, "workshop", home.root.join("Cursor-workshop"));
    let folder = mkdir(home.root.join("shared-app"));
    write_folder_workspace(&home.layout, "hash-default", &folder);
    write_folder_workspace(&other, "hash-workshop", &folder);
    let mut rt = runtime(&home.layout, false, false);
    rt.installs = vec![home.layout.clone(), other];
    let folder_s = folder.display().to_string();
    let archive = home.root.join("out.chatkeep").display().to_string();
    let direct = run(&rt, &["export", "-n", &folder_s, &archive]).unwrap_err();
    let queued = run(&rt, &["queue", "add", "--", "export", &folder_s, &archive]).unwrap_err();
    assert_eq!(
        format!("{queued:#}"),
        format!("refusing to queue: {direct:#} Fix the arguments and retry queue add.")
    );
    run(
        &rt,
        &[
            "queue",
            "add",
            "--",
            "export",
            &folder_s,
            &archive,
            "--profile",
            "default",
        ],
    )
    .unwrap();
    assert_eq!(queue::load(&home.layout.chatkeep_home).unwrap().len(), 1);
}

#[test]
fn queue_execute_resolves_relative_replace_against_the_stored_cwd() {
    let home = cursor_home();
    let (_lock, _restore) = lock_cwd();
    let other = enter(&home.root.join("other"));
    mkdir(other.join("new"));
    let work = enter(&home.root.join("work"));
    let source = mkdir(work.join("old").join("proj"));
    let dest = mkdir(work.join("new").join("proj"));
    write_folder_workspace(&home.layout, "hash-old", &source);
    let rt = runtime(&home.layout, false, false);
    let from_prefix = source.parent().unwrap().display().to_string();
    run(
        &rt,
        &["queue", "add", "--", "mv", "--replace", &from_prefix, "new"],
    )
    .unwrap();
    std::env::set_current_dir(&other).unwrap();
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    assert!(
        home.layout
            .workspace_storage()
            .join(compute_workspace_hash(&dest).unwrap())
            .exists()
    );
    assert!(!home.layout.workspace_storage().join("hash-old").exists());
}

#[test]
fn queue_execute_probe_failure_exits_1() {
    struct Broken;
    impl Probe for Broken {
        fn instances(&self) -> anyhow::Result<Vec<Instance>> {
            anyhow::bail!("permission denied")
        }
    }
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "missing".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    let rt = runtime_with(&home.layout, Arc::new(Broken), false);
    let err = run(&rt, &["queue", "execute", "-y"]).unwrap_err();
    assert!(err.downcast_ref::<CursorRunning>().is_none(), "{err:#}");
    assert!(
        format!("{err:#}").contains("Cannot tell whether Cursor is running: permission denied"),
        "{err:#}"
    );
    assert_eq!(queue_cmd::report_failure_with(&err, &[]), 1);
}

#[test]
fn queue_execute_continue_on_error_runs_the_rest_in_the_same_run() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let from = mkdir(home.root.join("from"));
    let mid = mkdir(home.root.join("mid"));
    let other = mkdir(home.root.join("other"));
    let dest = mkdir(home.root.join("dest"));
    write_folder_workspace(&home.layout, "hash-from", &from);
    write_folder_workspace(&home.layout, "hash-other", &other);
    let rt = runtime(&home.layout, false, false);
    let from_s = from.display().to_string();
    let mid_s = mid.display().to_string();
    let other_s = other.display().to_string();
    let dest_s = dest.display().to_string();
    run(&rt, &["queue", "add", "--", "mv", &from_s, &mid_s]).unwrap();
    run(&rt, &["queue", "add", "--", "rm", "hash-from"]).unwrap();
    run(&rt, &["queue", "add", "--", "mv", &other_s, &dest_s]).unwrap();
    let ids: Vec<String> = queue::load(&home.layout.chatkeep_home)
        .unwrap()
        .into_iter()
        .map(|entry| entry.id)
        .collect();
    let err = run(&rt, &["queue", "execute", "-y", "--continue-on-error"]).unwrap_err();
    assert_eq!(err.to_string(), "queue execute finished with failures");
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, ids[1]);
    assert_eq!(entries[0].status, queue::Status::Failed);
    assert_eq!(
        entries[0].last_error.as_deref(),
        Some("no workspace or chat matches hash-from")
    );
    let storage = home.layout.workspace_storage();
    assert!(storage.join(compute_workspace_hash(&mid).unwrap()).exists());
    assert!(
        storage
            .join(compute_workspace_hash(&dest).unwrap())
            .exists()
    );
}

#[test]
fn queue_execute_marks_a_crashed_running_entry_failed() {
    let home = cursor_home();
    let (_lock, _restore) = stable_cwd(&home);
    let (entry, _) = queue::add(
        &home.layout.chatkeep_home,
        vec!["rm".into(), "hash".into()],
        home.root.clone(),
        None,
        Vec::new(),
    )
    .unwrap();
    queue::update(
        &home.layout.chatkeep_home,
        &entry.id,
        queue::Status::Running,
        None,
    )
    .unwrap();
    let rt = runtime(&home.layout, false, false);
    run(&rt, &["queue", "execute", "-y"]).unwrap();
    let entries = queue::load(&home.layout.chatkeep_home).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, queue::Status::Failed);
    assert_eq!(
        entries[0].last_error.as_deref(),
        Some("interrupted during a previous run; check chatkeep history, then retry or rm")
    );
}

#[test]
fn queue_clear_rejects_write_flags() {
    for argv in [
        ["chatkeep", "queue", "clear", "--replace", "a", "b"].as_slice(),
        &["chatkeep", "queue", "execute", "--profile", "default"],
    ] {
        let err = Cli::try_parse_from(argv).unwrap_err();
        assert_eq!(
            err.kind(),
            clap::error::ErrorKind::UnknownArgument,
            "{argv:?}"
        );
    }
}

#[test]
fn cursor_running_keeps_the_rollback_note() {
    let err = anyhow::Error::new(CursorRunning::for_command(
        "Cursor is running: default (pid 1).",
    ))
    .context("Rollback incomplete. Backup kept at /backups/x");
    assert!(format!("{err:#}").contains("/backups/x"));
    assert_eq!(
        queue_cmd::report_failure_with(&err, &["mv".into(), "a".into(), "b".into()]),
        75
    );
}
