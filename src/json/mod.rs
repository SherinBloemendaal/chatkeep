//! `--json`: every command prints exactly one JSON document on stdout, for apps that run
//! chatkeep as a subprocess. docs/json.md is the contract; keep it in step with this module.

mod claude;
mod cursor;
mod local;
mod shape;

use anyhow::Result;
use clap::Parser;
use clap::error::ErrorKind;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::claude::Busy;
use crate::cli::{
    self, AccountsAction, ClaudeCacheAction, ClaudeCommand, Cli, Command, CursorCommand,
    QueueAction, Route, SyncAction, SyncAutoAction, Tools,
};
use crate::engine::{self, CursorRunning, CursorRunningHint, append_history, record_history};
use crate::queue_cmd::{self, ClapParseError};
use crate::ui::{self, ColorMode, Hinted, Lacking};

/// Exit code of a command a tool that is still open stopped, as without `--json`.
const BLOCKED: i32 = 75;
/// Exit code of arguments clap could not parse.
const USAGE: i32 = 2;

/// Whether `args` (without the program name) ask for JSON: `--json` before any `--`.
pub fn requested(args: &[String]) -> bool {
    args.iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json")
}

/// Run the command of `args` on this machine and print its JSON document. Returns the exit code.
pub fn run(args: &[String]) -> i32 {
    ui::term::init(ColorMode::Never);
    let response = respond(args, Tools::system);
    println!("{}", response.body);
    response.code
}

/// The document a `--json` run prints, and its exit code.
pub struct Response {
    pub body: Value,
    pub code: i32,
}

/// One result of a command: the tool it is about, or `None` for chatkeep's own state.
struct Part {
    tool: Option<&'static str>,
    data: Value,
}

impl Part {
    fn of(tool: Option<&'static str>, data: Value) -> Self {
        Self { tool, data }
    }

    fn value(&self) -> Value {
        json!({"tool": self.tool, "data": self.data})
    }
}

/// Which targets each chat of a split goes to.
type Assigned = BTreeMap<String, Vec<PathBuf>>;

/// A command that failed after some of its parts finished.
struct Failed {
    error: anyhow::Error,
    done: Vec<Part>,
}

impl From<anyhow::Error> for Failed {
    fn from(error: anyhow::Error) -> Self {
        Self {
            error,
            done: Vec::new(),
        }
    }
}

/// A JSON error the run gives before it touches any data.
struct Refusal {
    kind: &'static str,
    message: String,
    hint: Option<String>,
}

impl Refusal {
    fn new(kind: &'static str, message: impl Into<String>, hint: Option<&str>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: hint.map(str::to_string),
        }
    }
}

/// Parse `args` and run the command with the tools `tools` builds, answering in JSON. `args`
/// leave out the program name; `--json` may or may not be among them.
pub fn respond(args: &[String], tools: impl FnOnce() -> Result<Tools>) -> Response {
    let stripped = without_json(args);
    if cli::help_request(&stripped).is_some() {
        return refuse(Some("help"), unsupported("help"));
    }
    let full = std::iter::once("chatkeep".to_string()).chain(args.iter().cloned());
    let parsed = match Cli::try_parse_from(full) {
        Ok(parsed) => parsed,
        Err(err) if err.kind() == ErrorKind::DisplayVersion => {
            return success(
                "version",
                false,
                vec![Part::of(
                    None,
                    json!({"version": env!("CARGO_PKG_VERSION")}),
                )],
            );
        }
        Err(err) if err.kind() == ErrorKind::DisplayHelp => {
            return refuse(Some("help"), unsupported("help"));
        }
        Err(err) => return refuse(None, usage_refusal(&err.render().to_string())),
    };
    let command = match parsed.command {
        _ if parsed.help => return refuse(Some("help"), unsupported("help")),
        Some(command) => command,
        None => {
            return refuse(
                None,
                Refusal::new(
                    "usage",
                    "no command given",
                    Some("Name a command, e.g. chatkeep ls --json."),
                ),
            );
        }
    };
    let name = label(&command);
    if let Some(refusal) = check(&command) {
        return refuse(Some(&name), refusal);
    }
    let dry_run = dry_run_of(&command);
    let tools = match tools() {
        Ok(tools) => tools.for_json(),
        Err(err) => return failure(Some(&name), &err, &stripped, Vec::new()),
    };
    match dispatch(&tools, command) {
        Ok(parts) => success(&name, dry_run, parts),
        Err(failed) => failure(Some(&name), &failed.error, &stripped, failed.done),
    }
}

/// `args` without the `--json` flags before `--`: what the queue would store.
fn without_json(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut positional = false;
    for arg in args {
        if arg == "--" {
            positional = true;
        }
        if !positional && arg == "--json" {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn success(name: &str, dry_run: bool, parts: Vec<Part>) -> Response {
    Response {
        body: json!({
            "ok": true,
            "command": name,
            "dry_run": dry_run,
            "results": parts.iter().map(Part::value).collect::<Vec<_>>(),
        }),
        code: 0,
    }
}

fn error_body(
    name: Option<&str>,
    kind: &str,
    message: &str,
    hint: Option<&str>,
    queue_args: Option<Vec<String>>,
    done: &[Part],
) -> Value {
    json!({
        "ok": false,
        "command": name,
        "error": {
            "kind": kind,
            "message": message,
            "hint": hint,
            "queue_args": queue_args,
        },
        "results": done.iter().map(Part::value).collect::<Vec<_>>(),
    })
}

fn refuse(name: Option<&str>, refusal: Refusal) -> Response {
    let code = if refusal.kind == "usage" { USAGE } else { 1 };
    Response {
        body: error_body(
            name,
            refusal.kind,
            &refusal.message,
            refusal.hint.as_deref(),
            None,
            &[],
        ),
        code,
    }
}

fn unsupported(name: &str) -> Refusal {
    Refusal::new(
        "unsupported_in_json",
        format!("{name} has no JSON output"),
        Some("Run it without --json."),
    )
}

/// A clap error as one message line, without its `error:` prefix and usage block.
fn usage_refusal(rendered: &str) -> Refusal {
    let plain = ui::layout::strip_ansi(rendered);
    let message = plain
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("the arguments could not be parsed")
        .trim()
        .trim_start_matches("error:")
        .trim()
        .to_string();
    let hint = plain
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("tip:"))
        .map(|tip| tip.trim().to_string());
    Refusal {
        kind: "usage",
        message,
        hint,
    }
}

/// Classify `err` into the error envelope.
fn failure(name: Option<&str>, err: &anyhow::Error, argv: &[String], done: Vec<Part>) -> Response {
    let message = message_of(err);
    let hinted = err.chain().find_map(|cause| cause.downcast_ref::<Hinted>());
    if let Some(running) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<CursorRunning>())
    {
        let report = queue_cmd::cursor_blocked_report(running, argv);
        let queue_args = match running.hint {
            CursorRunningHint::QueueAdd => {
                Some(engine::queue::with_flags(argv, &running.queue_args))
            }
            CursorRunningHint::QueueExecute { .. } => None,
        };
        let hint = match &report.leftover {
            Some(leftover) => format!("{leftover} {}", report.hint),
            None => report.hint,
        };
        return Response {
            body: error_body(
                name,
                "tool_running",
                &message,
                Some(&hint),
                queue_args,
                &done,
            ),
            code: BLOCKED,
        };
    }
    if let Some(busy) = err.chain().find_map(|cause| cause.downcast_ref::<Busy>()) {
        let report = queue_cmd::claude_blocked_report(busy, argv);
        let queue_args = busy.pending.is_none().then(|| argv.to_vec());
        return Response {
            body: error_body(
                name,
                "tool_running",
                &message,
                Some(&report.hint),
                queue_args,
                &done,
            ),
            code: BLOCKED,
        };
    }
    if err
        .chain()
        .any(|cause| cause.downcast_ref::<ClapParseError>().is_some())
    {
        let refusal = usage_refusal(&message);
        return Response {
            body: error_body(
                name,
                "usage",
                &refusal.message,
                refusal.hint.as_deref(),
                None,
                &done,
            ),
            code: USAGE,
        };
    }
    let kind = match hinted.and_then(|hinted| hinted.lacking) {
        Some(Lacking::Argument) => "missing_argument",
        Some(Lacking::Confirmation) => "needs_confirmation",
        None => "failed",
    };
    Response {
        body: error_body(
            name,
            kind,
            &message,
            hinted.map(|hinted| hinted.hint.as_str()),
            None,
            &done,
        ),
        code: 1,
    }
}

/// The error and its causes on one line, without the hint (it has its own field).
fn message_of(err: &anyhow::Error) -> String {
    err.chain()
        .map(|cause| match cause.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => cause.to_string(),
        })
        .collect::<Vec<_>>()
        .join(": ")
}

/// The command as its words: `mv`, `claude accounts ls`, `queue list`.
pub fn label(command: &Command) -> String {
    let words: &[&str] = match command {
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(_) => &["cursor", "save"],
            CursorCommand::Rx(_) => &["cursor", "rx"],
            CursorCommand::Cache(args) => match &args.action {
                cli::CacheAction::Clear(_) => &["cursor", "cache", "clear"],
                cli::CacheAction::Scan(_) => &["cursor", "cache", "scan"],
                cli::CacheAction::Stats(_) => &["cursor", "cache", "stats"],
            },
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => match &args.action {
                AccountsAction::Ls => &["claude", "accounts", "ls"],
                AccountsAction::Cp(_) => &["claude", "accounts", "cp"],
                AccountsAction::Name(_) => &["claude", "accounts", "name"],
                AccountsAction::Doctor => &["claude", "accounts", "doctor"],
                AccountsAction::Prune(_) => &["claude", "accounts", "prune"],
            },
            ClaudeCommand::Cache(args) => match &args.action {
                ClaudeCacheAction::Clear(_) => &["claude", "cache", "clear"],
                ClaudeCacheAction::Stats => &["claude", "cache", "stats"],
            },
            ClaudeCommand::Sync(args) => match &args.action {
                None => &["claude", "sync"],
                Some(SyncAction::Profiles) => &["claude", "sync", "profiles"],
                Some(SyncAction::Set(_)) => &["claude", "sync", "set"],
                Some(SyncAction::Add(_)) => &["claude", "sync", "add"],
                Some(SyncAction::Rm(_)) => &["claude", "sync", "rm"],
                Some(SyncAction::Watch(_)) => &["claude", "sync", "watch"],
                Some(SyncAction::Log(_)) => &["claude", "sync", "log"],
                Some(SyncAction::Auto(args)) => match args.action {
                    SyncAutoAction::On => &["claude", "sync", "auto", "on"],
                    SyncAutoAction::Off => &["claude", "sync", "auto", "off"],
                    SyncAutoAction::Status => &["claude", "sync", "auto", "status"],
                },
            },
        },
        Command::Queue(args) => match &args.action {
            QueueAction::Add(_) => &["queue", "add"],
            QueueAction::List(_) => &["queue", "list"],
            QueueAction::Rm(_) => &["queue", "rm"],
            QueueAction::Clear(_) => &["queue", "clear"],
            QueueAction::Retry(_) => &["queue", "retry"],
            QueueAction::Execute(_) => &["queue", "execute"],
        },
        other => return cli::command_name(other).to_string(),
    };
    words.join(" ")
}

/// Whether the run is a dry run, as its flags say.
fn dry_run_of(command: &Command) -> bool {
    match command {
        Command::Queue(args) => cli::queue_flags(&args.action).0,
        other => cli::common_of(other).dry_run,
    }
}

/// Commands that change data and take `-y`: with `--json` they need `-y` or `-n`.
fn confirmed_write(command: &Command) -> bool {
    match command {
        Command::Mv(_)
        | Command::Cp(_)
        | Command::Split(_)
        | Command::Combine(_)
        | Command::Rm(_)
        | Command::Export(_)
        | Command::Import(_) => true,
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(_) | CursorCommand::Rx(_) => true,
            CursorCommand::Cache(args) => matches!(args.action, cli::CacheAction::Clear(_)),
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => {
                matches!(
                    args.action,
                    AccountsAction::Cp(_) | AccountsAction::Prune(_)
                )
            }
            ClaudeCommand::Cache(args) => matches!(args.action, ClaudeCacheAction::Clear(_)),
            ClaudeCommand::Sync(args) => matches!(
                args.action,
                None | Some(SyncAction::Set(_) | SyncAction::Add(_) | SyncAction::Rm(_))
            ),
        },
        Command::Queue(args) => !matches!(args.action, QueueAction::Add(_) | QueueAction::List(_)),
        _ => false,
    }
}

/// Commands that change data: logged in chatkeep's history when they are no dry run, as
/// without `--json`. Next to the writes that take `-y`, those are `claude accounts name`,
/// `claude sync auto on|off`, and `cursor cache scan` (it rewrites the index).
fn is_write(command: &Command) -> bool {
    if confirmed_write(command) {
        return true;
    }
    match command {
        Command::Cursor(args) => matches!(
            &args.command,
            CursorCommand::Cache(cli::CacheArgs {
                action: cli::CacheAction::Scan(_)
            })
        ),
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => matches!(args.action, AccountsAction::Name(_)),
            ClaudeCommand::Sync(cli::SyncArgs {
                action: Some(SyncAction::Auto(args)),
                ..
            }) => !matches!(args.action, SyncAutoAction::Status),
            _ => false,
        },
        _ => false,
    }
}

/// What stops a command before it runs: no JSON output, a missing argument, or no `-y`.
fn check(command: &Command) -> Option<Refusal> {
    let name = label(command);
    match command {
        Command::Update | Command::Uninstall(_) | Command::Github | Command::Help(_) => {
            return Some(unsupported(&name));
        }
        Command::RefreshIndex => return Some(unsupported(&name)),
        Command::Claude(cli::ClaudeArgs {
            command:
                ClaudeCommand::Sync(cli::SyncArgs {
                    action: Some(SyncAction::Watch(_)),
                    ..
                }),
        }) => return Some(unsupported(&name)),
        _ => {}
    }
    if let Some(what) = missing(command) {
        return Some(Refusal::new(
            "missing_argument",
            format!("{name} needs {what}"),
            Some("Pass it on the command line; --json never asks."),
        ));
    }
    let common = match command {
        Command::Queue(args) => cli::queue_flags(&args.action),
        other => {
            let common = cli::common_of(other);
            (common.dry_run, common.yes)
        }
    };
    if confirmed_write(command) && !common.0 && !common.1 {
        return Some(Refusal::new(
            "needs_confirmation",
            format!("{name} changes data and --json never asks"),
            Some("Pass -n to see the plan, or -y to run it."),
        ));
    }
    None
}

/// What a command lacks that a terminal would have asked for.
fn missing(command: &Command) -> Option<&'static str> {
    match command {
        Command::Mv(args) | Command::Cp(args) => (args.common.replace.is_empty()
            && (args.from.is_none() || args.to.is_none()))
        .then_some("FROM and TO, or --replace FROM TO"),
        Command::Rm(args) => args
            .target
            .is_none()
            .then_some("a project path, a Cursor workspace hash, or a chat id"),
        Command::Split(args) if args.source.is_none() => Some("the project to split"),
        Command::Split(args) if args.targets.is_empty() => Some("at least one target folder"),
        Command::Combine(args) if args.target.is_none() => Some("the target project"),
        Command::Combine(args) if args.sources.is_empty() => Some("at least one source"),
        Command::Export(args) if args.target.is_none() => Some("the project to export"),
        Command::Export(args) if args.file.is_none() => Some("the archive path"),
        Command::Import(args) => args.file.is_none().then_some("the archive file"),
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(args) if args.id.is_none() => Some("the unsaved session id"),
            CursorCommand::Save(args) if args.to.is_none() => Some("the destination folder"),
            CursorCommand::Rx(args) => args
                .target
                .is_none()
                .then_some("a project path or Cursor workspace hash"),
            _ => None,
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(cli::AccountsArgs {
                action: AccountsAction::Name(args),
            }) if args.account.is_none() => Some("the account to name"),
            ClaudeCommand::Accounts(cli::AccountsArgs {
                action: AccountsAction::Name(args),
            }) if args.name.is_none() && !args.clear => Some("the name, or --clear"),
            ClaudeCommand::Sync(cli::SyncArgs {
                action: Some(SyncAction::Set(args) | SyncAction::Add(args)),
                ..
            }) if args.accounts.is_empty() => Some("the accounts"),
            _ => None,
        },
        Command::Queue(args) => match &args.action {
            QueueAction::Rm(args) if args.ids.is_empty() => Some("at least one queue entry id"),
            _ => None,
        },
        _ => None,
    }
}

fn dispatch(tools: &Tools, command: Command) -> std::result::Result<Vec<Part>, Failed> {
    match command {
        Command::Queue(args) => local::queue(tools, args),
        Command::History(common) => Ok(vec![Part::of(None, local::history(tools, &common)?)]),
        command => run_route(cli::route(tools, command)?),
    }
}

/// Run `route` for each tool it covers, Claude Code first, as a direct run does.
fn run_route(route: Route) -> std::result::Result<Vec<Part>, Failed> {
    match route {
        Route::Cursor(rt, command) => {
            let write = is_write(&command);
            let started = Instant::now();
            let name = cli::command_name(&command);
            let args = cli::command_args(&command);
            let result = cursor::perform(&rt, command);
            if write {
                record_history(&rt, name, &args, outcome(&result), started);
            }
            Ok(vec![Part::of(Some("cursor"), result?)])
        }
        Route::Claude(rt, command) => {
            let write = is_write(&command);
            let started = Instant::now();
            let name = cli::command_name(&command);
            let args = cli::command_args(&command);
            let result = claude::perform(&rt, command);
            if write && !rt.dry_run {
                append_history(
                    &rt.layout.history_file(),
                    name,
                    &args,
                    outcome(&result),
                    started,
                );
            }
            Ok(vec![Part::of(Some("claude"), result?)])
        }
        Route::Both {
            cursor,
            claude,
            cursor_command,
            claude_command,
        } => {
            let write = is_write(&cursor_command);
            let started = Instant::now();
            let name = cli::command_name(&cursor_command);
            let args = cli::command_args(&cursor_command);
            let mut done = Vec::new();
            let result = claude::perform(&claude, claude_command).and_then(|data| {
                done.push(Part::of(Some("claude"), data));
                cursor::perform(&cursor, cursor_command).map_err(cli::after_claude)
            });
            if write {
                record_history(&cursor, name, &args, outcome(&result), started);
            }
            match result {
                Ok(data) => {
                    done.push(Part::of(Some("cursor"), data));
                    Ok(done)
                }
                Err(error) => Err(Failed { error, done }),
            }
        }
    }
}

fn outcome<T>(result: &Result<T>) -> &'static str {
    if result.is_ok() { "ok" } else { "error" }
}

/// A write command's result: its title and what it did.
fn finished(outcome: cli::Outcome) -> Result<Value> {
    match outcome {
        cli::Outcome::Finished { title, report, .. } => Ok(shape::report(title, &report)),
        cli::Outcome::Text { .. } | cli::Outcome::Standalone => {
            anyhow::bail!("this command has no JSON output")
        }
    }
}

/// `engine::Report` with the split plan: per chat, where the suggestion put it and where it
/// goes.
fn split_value(
    title: &str,
    report: &engine::Report,
    suggested: &engine::SplitSuggestion,
    assigned: &Assigned,
    resolve: impl Fn(&Path) -> PathBuf,
) -> Value {
    let mut ids: Vec<&String> = suggested
        .assigned
        .keys()
        .chain(suggested.unassigned.iter())
        .collect();
    ids.sort();
    ids.dedup();
    let paths = |found: Option<&Vec<PathBuf>>| -> Vec<String> {
        found
            .map(|list| {
                list.iter()
                    .map(|path| shape::path(&resolve(path)))
                    .collect()
            })
            .unwrap_or_default()
    };
    let chats: Vec<Value> = ids
        .into_iter()
        .map(|id| {
            json!({
                "id": id,
                "title": suggested.titles.get(id).cloned().flatten(),
                "suggested": paths(suggested.assigned.get(id)),
                "targets": paths(assigned.get(id)),
            })
        })
        .collect();
    let mut value = shape::report(title, report);
    if let Some(object) = value.as_object_mut() {
        object.insert("chats".to_string(), Value::Array(chats));
    }
    value
}

/// What split sends where: the suggestion with `--assign` applied. A chat that still has no
/// target stops a real run; a dry run leaves it out and says so.
fn settle(suggestion: &engine::SplitSuggestion, dry_run: bool) -> Result<(Assigned, Vec<String>)> {
    let mut assigned = suggestion.assigned.clone();
    if suggestion.unassigned.is_empty() {
        return Ok((assigned, Vec::new()));
    }
    let count = ui::plural(suggestion.unassigned.len(), "chat", "chats");
    if !dry_run {
        return Err(ui::lacking(
            Lacking::Argument,
            format!("{count} match no target folder"),
            "Pass --assign CHAT=TARGET for each, or CHAT= to leave one out.",
        ));
    }
    for id in &suggestion.unassigned {
        assigned.insert(id.clone(), Vec::new());
    }
    Ok((
        assigned,
        vec![format!(
            "{count} match no target folder: a real run needs --assign CHAT=TARGET for each, or CHAT= to leave one out"
        )],
    ))
}
