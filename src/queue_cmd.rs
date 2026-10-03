//! `chatkeep queue` subcommands: add, list, rm, clear, retry, execute.

use anyhow::{Context, Result, bail};
use clap::Parser;
use clap::error::ErrorKind;
use std::env;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Plans run side by side on at most this many threads.
const MAX_PLANNERS: usize = 8;

use crate::claude::Busy;
use crate::cli::{
    self, Cli, Command, Mode, Outcome, QueueAction, QueueAddArgs, QueueArgs, QueueExecuteArgs,
    QueueListArgs, QueueRetryArgs, QueueRmArgs, Route, Session, Tools,
};
use crate::engine::{CursorRunning, CursorRunningHint, Runtime, index, queue};
use crate::ui::{self, Theme};

pub fn run(tools: &Tools, args: QueueArgs) -> Result<()> {
    let (dry_run, yes) = cli::queue_flags(&args.action);
    let session = tools.session(dry_run, yes);
    match args.action {
        QueueAction::Add(args) => run_add(tools, &session, args),
        QueueAction::List(args) => run_list(&session, args),
        QueueAction::Rm(args) => run_rm(&session, args),
        QueueAction::Clear(_) => run_clear(&session),
        QueueAction::Retry(args) => run_retry(&session, args),
        QueueAction::Execute(args) => run_execute(tools, &session, args),
    }
}

#[derive(Debug)]
pub struct ClapParseError {
    pub message: String,
}

impl std::fmt::Display for ClapParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message.trim_end())
    }
}

impl std::error::Error for ClapParseError {}

pub fn parse_command(argv: &[String]) -> Result<Command> {
    let full = std::iter::once("chatkeep".to_string()).chain(argv.iter().cloned());
    let cli = Cli::try_parse_from(full).map_err(|err| {
        if matches!(
            err.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        ) {
            anyhow::anyhow!("queue add needs a write command after --")
        } else {
            anyhow::Error::new(ClapParseError {
                message: err.render().to_string(),
            })
        }
    })?;
    match cli.command {
        Some(command) if !cli.help => Ok(command),
        _ => bail!("queue add needs a write command after --"),
    }
}

/// The stored command of `entry`, with its paths resolved from the folder it was queued in.
fn prepared(entry: &queue::Entry) -> Result<Command> {
    if !entry.cwd.is_dir() {
        bail!("queued cwd no longer exists: {}", entry.cwd.display());
    }
    let mut command = parse_command(&entry.argv)?;
    if matches!(command, Command::Queue(_)) {
        bail!("cannot nest queue commands");
    }
    command.resolve_paths(&entry.cwd);
    Ok(command)
}

/// A dry run of `command` without prompts: its warnings, whether the real run would wait for
/// a tool to be closed, and which tools it is for.
struct Preflight {
    planned: Result<Vec<String>>,
    waits: bool,
    tools: &'static str,
}

fn preflight(tools: &Tools, cwd: Option<&Path>, command: Command) -> Preflight {
    let tools = tools.in_mode(Mode {
        cwd: cwd.map(Path::to_path_buf),
        dry_run: true,
    });
    match cli::route(&tools, command) {
        Ok(route) => {
            let names = match &route {
                Route::Cursor(..) => "Cursor",
                Route::Claude(..) => "Claude Code",
                Route::Both { .. } => "Cursor or Claude Code",
            };
            let (planned, waits) = cli::plan_route(route);
            Preflight {
                planned,
                waits,
                tools: names,
            }
        }
        Err(err) => Preflight {
            planned: Err(err),
            waits: false,
            tools: "Cursor",
        },
    }
}

/// Whether a dry run of `argv` gets as far as checking that Cursor is closed.
pub fn dry_run_reaches_guard(rt: &Runtime, argv: &[String]) -> Result<bool> {
    let tools = Tools::new(Some(rt.clone()), None);
    Ok(preflight(&tools, None, parse_command(argv)?).waits)
}

fn run_add(tools: &Tools, session: &Session, args: QueueAddArgs) -> Result<()> {
    let command = parse_command(&args.argv)?;
    if matches!(command, Command::Queue(_)) {
        bail!("cannot nest queue commands");
    }
    let name = cli::command_name(&command);
    let profile = cli::common_of(&command).profile;
    let found = preflight(tools, None, command);
    let warnings = found.planned.map_err(|err| {
        ui::hinted(
            format!("refusing to queue: {err:#}"),
            "Fix the arguments and retry queue add.",
        )
    })?;
    if !found.waits {
        return Err(ui::hinted(
            format!(
                "{name} does not need {} closed and can run directly",
                found.tools
            ),
            "Run the command without queue add.",
        ));
    }
    let cwd = env::current_dir().context("cannot read the current directory")?;
    let (entry, position) = queue::add(&session.home, args.argv, cwd, profile, warnings)?;
    let theme = Theme::stdout();
    println!(
        "{}",
        ui::ok_line(
            theme,
            &format!(
                "queued {} at position {}",
                theme.id(&entry.id),
                theme.number(ui::count(position as u64))
            )
        )
    );
    if !entry.warnings.is_empty() {
        let theme = Theme::stderr();
        for warning in &entry.warnings {
            eprintln!("{}", ui::warn_line(theme, warning));
        }
    }
    Ok(())
}

fn run_list(rt: &Session, args: QueueListArgs) -> Result<()> {
    let entries = queue::load(&rt.home)?;
    print!(
        "{}",
        queue::render_list(Theme::stdout(), &entries, args.full)
    );
    Ok(())
}

fn run_rm(rt: &Session, args: QueueRmArgs) -> Result<()> {
    if args.ids.is_empty() {
        bail!("queue rm needs at least one id");
    }
    let home = &rt.home;
    let entries = queue::load(home)?;
    let rows: Vec<Vec<String>> = args
        .ids
        .iter()
        .map(|id| {
            let command = entries
                .iter()
                .find(|entry| entry.id == *id)
                .map(|entry| queue::shell_join(&entry.argv))
                .unwrap_or_else(|| "?".into());
            vec![id.clone(), command]
        })
        .collect();
    if !rt.dry_run {
        print!(
            "{}",
            ui::validation(
                Theme::stdout(),
                "Remove from queue",
                &["id", "command"],
                &rows,
                &[]
            )
        );
        if !ui::confirm("Remove these queue entries?", rt.yes, rt.interactive)? {
            bail!("aborted");
        }
    }
    if rt.dry_run {
        println!(
            "{}",
            ui::info_line(
                Theme::stdout(),
                &format!(
                    "dry-run remove {}",
                    ui::plural(args.ids.len(), "entry", "entries")
                )
            )
        );
        return Ok(());
    }
    let removed = queue::remove(home, &args.ids)?;
    println!(
        "{}",
        ui::ok_line(
            Theme::stdout(),
            &format!("removed {}", ui::plural(removed.len(), "entry", "entries"))
        )
    );
    Ok(())
}

fn run_clear(rt: &Session) -> Result<()> {
    let home = &rt.home;
    let total = queue::load(home)?.len();
    if total == 0 {
        println!("{}", ui::info_line(Theme::stdout(), "Queue is empty."));
        return Ok(());
    }
    if !rt.dry_run
        && !ui::confirm(
            &format!(
                "Clear {} from the queue?",
                ui::plural(total, "entry", "entries")
            ),
            rt.yes,
            rt.interactive,
        )?
    {
        bail!("aborted");
    }
    if rt.dry_run {
        println!(
            "{}",
            ui::info_line(
                Theme::stdout(),
                &format!("dry-run clear {}", ui::plural(total, "entry", "entries"))
            )
        );
        return Ok(());
    }
    let removed = queue::clear_all(home)?;
    println!(
        "{}",
        ui::ok_line(
            Theme::stdout(),
            &format!("cleared {}", ui::plural(removed, "entry", "entries"))
        )
    );
    Ok(())
}

fn run_retry(rt: &Session, args: QueueRetryArgs) -> Result<()> {
    let home = &rt.home;
    let ids = if args.ids.is_empty() {
        None
    } else {
        Some(args.ids.as_slice())
    };
    if rt.dry_run {
        let entries = queue::load(home)?;
        let count = match ids {
            Some(ids) => ids.len(),
            None => entries
                .iter()
                .filter(|entry| {
                    matches!(entry.status, queue::Status::Failed | queue::Status::Skipped)
                })
                .count(),
        };
        println!(
            "{}",
            ui::info_line(
                Theme::stdout(),
                &format!("dry-run retry {}", ui::plural(count, "entry", "entries"))
            )
        );
        return Ok(());
    }
    let reset = queue::retry(home, ids)?;
    if reset.is_empty() {
        println!(
            "{}",
            ui::info_line(Theme::stdout(), "No failed or skipped entries to retry.")
        );
        return Ok(());
    }
    println!(
        "{}",
        ui::ok_line(
            Theme::stdout(),
            &format!("retrying {}", ui::plural(reset.len(), "entry", "entries"))
        )
    );
    Ok(())
}

/// A tool that is still in use stops the queue; the error says how many entries wait.
fn map_execute_blocked(err: anyhow::Error, pending: usize) -> anyhow::Error {
    if let Some(busy) = claude_busy(&err) {
        let mut busy = busy.clone();
        busy.pending = Some(pending);
        return anyhow::Error::new(busy);
    }
    let Some(leftover) = cursor_running(&err).map(|running| running.leftover.clone()) else {
        return err;
    };
    anyhow::Error::new(
        CursorRunning::for_queue_execute(format!("{err:#}"), pending).with_leftover(leftover),
    )
}

fn cursor_running(err: &anyhow::Error) -> Option<&CursorRunning> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<CursorRunning>())
}

fn claude_busy(err: &anyhow::Error) -> Option<&Busy> {
    err.chain().find_map(|cause| cause.downcast_ref::<Busy>())
}

/// Whether a tool that is still open is what stopped a command.
fn is_blocked(err: &anyhow::Error) -> bool {
    cursor_running(err).is_some() || claude_busy(err).is_some()
}

/// Whether running `entry` may change Cursor data. An entry chatkeep cannot route yet counts,
/// so a Cursor that is open is never written to by accident.
fn may_use_cursor(tools: &Tools, entry: &queue::Entry) -> bool {
    if !tools.has_cursor() {
        return false;
    }
    if !tools.has_claude() {
        return true;
    }
    let tools = tools.in_mode(Mode {
        cwd: Some(entry.cwd.clone()),
        dry_run: true,
    });
    match prepared(entry).and_then(|command| cli::route(&tools, command)) {
        Ok(route) => route.uses_cursor(),
        Err(_) => true,
    }
}

fn require_cursor_closed(tools: &Tools) -> Result<()> {
    match tools.cursor(&cli::unprompted())? {
        Some(rt) => rt.require_cursor_closed(),
        None => Ok(()),
    }
}

fn run_execute(tools: &Tools, rt: &Session, args: QueueExecuteArgs) -> Result<()> {
    let home = rt.home.clone();
    let entries = queue::load(&home)?;
    let waiting = queue::pending(&entries);
    // A machine with only Cursor checks it even for an empty queue, as it always did.
    let guards_cursor = tools.has_cursor()
        && (!tools.has_claude() || waiting.iter().any(|entry| may_use_cursor(tools, entry)));
    if !rt.dry_run
        && guards_cursor
        && let Err(err) = require_cursor_closed(tools)
    {
        return Err(map_execute_blocked(err, waiting.len()));
    }
    let _lock = queue::require_lock(&home)?;
    if !rt.dry_run {
        queue::fail_interrupted_locked(&home)?;
    }
    let entries = queue::load(&home)?;
    let pending: Vec<queue::Entry> = queue::pending(&entries).into_iter().cloned().collect();
    if pending.is_empty() {
        println!(
            "{}",
            ui::info_line(Theme::stdout(), "Queue has no pending entries.")
        );
        return Ok(());
    }

    let labels: Vec<String> = pending
        .iter()
        .map(|entry| format!("{} {}", entry.id, queue::shell_join(&entry.argv)))
        .collect();
    // One refresh up front, so the parallel plans below only read the index.
    if let Some(cursor) = tools.cursor(&cli::unprompted())?
        && let Err(err) = index::load(&cursor)
    {
        ui::warn(&format!(
            "the index was not refreshed before planning: {err:#}"
        ));
    }
    let checklist = ui::Checklist::new("plan", labels.clone(), rt.quiet, false);
    let planned = plan_all(tools, &pending, &checklist);
    let planned_in = checklist.elapsed();
    drop(checklist);

    let mut plans = Vec::new();
    let mut all_warnings = Vec::new();
    let mut rows = Vec::new();
    for (entry, (result, took)) in pending.iter().zip(planned) {
        let time = ui::elapsed(took);
        match result {
            Ok(warnings) => {
                for warning in &warnings {
                    all_warnings.push(format!("{}: {warning}", entry.id));
                }
                rows.push(vec![
                    entry.id.clone(),
                    queue::shell_join(&entry.argv),
                    if warnings.is_empty() {
                        "ok".into()
                    } else {
                        ui::plural(warnings.len(), "warning", "warnings")
                    },
                    time,
                ]);
                plans.push((entry.clone(), None::<String>));
            }
            Err(err) => {
                let message = format!("{err:#}");
                all_warnings.push(format!("{}: {message}", entry.id));
                rows.push(vec![
                    entry.id.clone(),
                    queue::shell_join(&entry.argv),
                    "skip".into(),
                    time,
                ]);
                plans.push((entry.clone(), Some(message)));
            }
        }
    }

    if !rt.quiet {
        println!(
            "{}",
            ui::ok_line(
                Theme::stdout(),
                &format!(
                    "Planned {} in {}",
                    ui::plural(rows.len(), "entry", "entries"),
                    ui::elapsed(planned_in)
                )
            )
        );
    }
    print!(
        "{}",
        ui::validation(
            Theme::stdout(),
            "Queue execute plan",
            &["id", "command", "plan", "time"],
            &rows,
            &all_warnings,
        )
    );
    if rt.dry_run {
        println!(
            "{}",
            ui::hint_line(
                Theme::stdout(),
                "Nothing was changed. Run again without -n to apply."
            )
        );
        return Ok(());
    }
    if !ui::confirm("Execute the pending queue entries?", rt.yes, rt.interactive)? {
        bail!("aborted");
    }

    let checklist = ui::Checklist::new("queue", labels, rt.quiet, true);
    let mut stopped = false;
    let mut done = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let total = plans.len();
    for (index, (entry, skip)) in plans.into_iter().enumerate() {
        let remaining = total - index;
        if skip.is_none()
            && may_use_cursor(tools, &entry)
            && let Err(err) = require_cursor_closed(tools)
        {
            return Err(map_execute_blocked(err, remaining));
        }
        checklist.start(index);
        if let Some(message) = skip {
            queue::update_locked(&home, &entry.id, queue::Status::Skipped, Some(message))?;
            checklist.finish(index, ui::Mark::Skip);
            skipped += 1;
            continue;
        }
        queue::update_locked(&home, &entry.id, queue::Status::Running, None)?;
        let performed = checklist.hosting(true, || perform_entry(tools, &entry));
        let print = |outcomes: Vec<(&'static str, Outcome)>| -> Result<()> {
            checklist.suspend(|| {
                for (name, outcome) in outcomes {
                    cli::print_outcome(name, outcome)?;
                }
                Ok(())
            })
        };
        match performed.result {
            Ok(()) => {
                queue::remove_locked(&home, std::slice::from_ref(&entry.id))?;
                checklist.finish(index, ui::Mark::Ok);
                done += 1;
                print(performed.outcomes)?;
            }
            Err(err) => {
                // Claude Code's half is done; what is left of this entry is Cursor's half.
                if performed.claude_done {
                    let mut argv = entry.argv.clone();
                    argv.extend(["--tool".to_string(), "cursor".to_string()]);
                    queue::set_argv_locked(&home, &entry.id, argv)?;
                }
                if is_blocked(&err) {
                    queue::update_locked(&home, &entry.id, queue::Status::Pending, None)?;
                    print(performed.outcomes)?;
                    return Err(map_execute_blocked(err, remaining));
                }
                let message = format!("{err:#}");
                queue::update_locked(
                    &home,
                    &entry.id,
                    queue::Status::Failed,
                    Some(message.clone()),
                )?;
                checklist.finish(index, ui::Mark::Fail);
                print(performed.outcomes)?;
                checklist.suspend(|| eprintln!("{}", ui::err_line(Theme::stderr(), &message)));
                failed += 1;
                if !args.continue_on_error {
                    stopped = true;
                    break;
                }
            }
        }
    }
    let executed_in = checklist.elapsed();
    drop(checklist);
    if !rt.quiet {
        let mut parts = vec![ui::plural(done, "entry", "entries")];
        if failed > 0 {
            parts.push(format!("{failed} failed"));
        }
        if skipped > 0 {
            parts.push(format!("{skipped} skipped"));
        }
        eprintln!(
            "{}",
            ui::ok_line(
                Theme::stderr(),
                &format!(
                    "Executed {} in {}",
                    parts.join(", "),
                    ui::elapsed(executed_in)
                )
            )
        );
    }
    if stopped {
        bail!("queue execute stopped after a failure");
    }
    if failed > 0 || skipped > 0 {
        bail!("queue execute finished with failures");
    }
    Ok(())
}

/// Plans never write, so they run side by side on a few threads; each result comes back with
/// how long its plan took, in queue order.
fn plan_all(
    tools: &Tools,
    pending: &[queue::Entry],
    checklist: &ui::Checklist,
) -> Vec<(Result<Vec<String>>, Duration)> {
    let workers = std::thread::available_parallelism()
        .map_or(1, NonZeroUsize::get)
        .clamp(1, MAX_PLANNERS)
        .min(pending.len().max(1));
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, Result<Vec<String>>, Duration)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    checklist.hosting(false, || {
                        let mut mine = Vec::new();
                        loop {
                            let index = next.fetch_add(1, Ordering::SeqCst);
                            let Some(entry) = pending.get(index) else {
                                break;
                            };
                            checklist.start(index);
                            let planned = replan(tools, entry);
                            let mark = match &planned {
                                Ok(warnings) if warnings.is_empty() => ui::Mark::Ok,
                                Ok(_) => ui::Mark::Warn,
                                Err(_) => ui::Mark::Skip,
                            };
                            let took = checklist.finish(index, mark);
                            mine.push((index, planned, took));
                        }
                        mine
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(index, ..)| *index);
    done.into_iter()
        .map(|(_, planned, took)| (planned, took))
        .collect()
}

/// What running one entry gave: every result that is ready to print, how it ended, and
/// whether Claude Code's half is done.
struct Performed {
    outcomes: Vec<(&'static str, Outcome)>,
    result: Result<()>,
    claude_done: bool,
}

/// Runs `entry` without prompts or its own plan table; the queue prints its results.
fn perform_entry(tools: &Tools, entry: &queue::Entry) -> Performed {
    let command = match prepared(entry) {
        Ok(command) => command,
        Err(err) => {
            return Performed {
                outcomes: Vec::new(),
                result: Err(err),
                claude_done: false,
            };
        }
    };
    let tools = tools.in_mode(Mode {
        cwd: Some(entry.cwd.clone()),
        dry_run: false,
    });
    let mut outcomes = Vec::new();
    let ran = cli::run_command(&tools, command, &mut |name, outcome| {
        outcomes.push((name, outcome));
        Ok(())
    });
    Performed {
        outcomes,
        result: ran.result,
        claude_done: ran.claude_done,
    }
}

fn replan(tools: &Tools, entry: &queue::Entry) -> Result<Vec<String>> {
    preflight(tools, Some(&entry.cwd), prepared(entry)?).planned
}

pub struct CursorBlockedReport {
    pub summary: String,
    /// How to end the processes a closed Cursor left behind, when no main process runs.
    pub leftover: Option<String>,
    pub hint: String,
    pub stable: String,
}

pub fn cursor_blocked_report(running: &CursorRunning, argv: &[String]) -> CursorBlockedReport {
    let leftover = (!running.leftover.is_empty()).then(|| {
        format!(
            "No main Cursor process is running. If Cursor is closed, these were left behind; end them with: {}",
            kill_command(&running.leftover)
        )
    });
    match &running.hint {
        CursorRunningHint::QueueAdd => {
            let argv: Vec<String> = argv
                .iter()
                .chain(running.queue_args.iter())
                .cloned()
                .collect();
            let queued = queue::queue_add_command(&argv);
            CursorBlockedReport {
                summary: running.summary.clone(),
                leftover,
                hint: format!("Queue it with: {queued}"),
                stable: format!("chatkeep: cursor-running; queue with: {queued}"),
            }
        }
        CursorRunningHint::QueueExecute { pending } => CursorBlockedReport {
            summary: running.summary.clone(),
            leftover,
            hint: "Close Cursor, then run: chatkeep queue execute".into(),
            stable: format!(
                "chatkeep: cursor-running; queued: {pending} pending; run later: chatkeep queue execute"
            ),
        },
    }
}

fn kill_command(pids: &[u32]) -> String {
    let (program, flag) = if cfg!(windows) {
        ("taskkill /F", " /PID ")
    } else {
        ("kill", " ")
    };
    let mut command = program.to_string();
    for pid in pids {
        command.push_str(flag);
        command.push_str(&pid.to_string());
    }
    command
}

pub fn report_failure(err: &anyhow::Error) -> i32 {
    report_failure_with(err, &env::args().skip(1).collect::<Vec<_>>())
}

/// What a blocked Claude Code command tells the user: how to free the chats, and how to queue.
pub struct ClaudeBlockedReport {
    pub summary: String,
    pub hint: String,
    pub stable: String,
}

pub fn claude_blocked_report(busy: &Busy, argv: &[String]) -> ClaudeBlockedReport {
    match busy.pending {
        None => {
            let queued = queue::queue_add_command(argv);
            ClaudeBlockedReport {
                summary: busy.summary.clone(),
                hint: format!(
                    "{}, then run the command again. Or queue it with: {queued}",
                    busy.remedy
                ),
                stable: format!("chatkeep: claude-busy; queue with: {queued}"),
            }
        }
        Some(pending) => ClaudeBlockedReport {
            summary: busy.summary.clone(),
            hint: format!("{}, then run: chatkeep queue execute", busy.remedy),
            stable: format!(
                "chatkeep: claude-busy; queued: {pending} pending; run later: chatkeep queue execute"
            ),
        },
    }
}

pub fn report_failure_with(err: &anyhow::Error, argv: &[String]) -> i32 {
    if let Some(running) = cursor_running(err) {
        let report = cursor_blocked_report(running, argv);
        let theme = Theme::stderr();
        eprintln!("{}", ui::err_line(theme, &format!("{err:#}")));
        if let Some(leftover) = &report.leftover {
            eprintln!("{}", ui::hint_line(theme, leftover));
        }
        eprintln!("{}", ui::hint_line(theme, &report.hint));
        eprintln!("{}", report.stable);
        return 75;
    }
    if let Some(busy) = claude_busy(err) {
        let report = claude_blocked_report(busy, argv);
        let theme = Theme::stderr();
        eprintln!("{}", ui::err_line(theme, &format!("{err:#}")));
        eprintln!("{}", ui::hint_line(theme, &report.hint));
        eprintln!("{}", report.stable);
        return 75;
    }
    if let Some(clap_err) = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<ClapParseError>())
    {
        eprint!("{}", clap_err.message);
        if !clap_err.message.ends_with('\n') {
            eprintln!();
        }
        return 2;
    }
    ui::report_error(err);
    1
}
