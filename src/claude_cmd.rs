//! The Claude Code side of every command.

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::time::Instant;

use crate::claude::{self, accounts, archive, index, ops, stats, store, transfer, view};
use crate::cli::{
    self, AccountCpArgs, AccountsAction, ClaudeCacheAction, ClaudeCommand, CombineArgs, Command,
    CommonArgs, ExportArgs, ImportArgs, Outcome, PathArgs, SplitArgs,
};
use crate::engine::{Report, append_history};
use crate::ui::{self, Theme, validation};

pub fn configure(mut rt: claude::Runtime, common: &CommonArgs) -> claude::Runtime {
    rt.dry_run |= common.dry_run;
    rt.yes |= common.yes;
    rt
}

/// Commands that change Claude Code data, and so wait in the queue while it is in use.
pub fn is_write(command: &Command) -> bool {
    match command {
        Command::Mv(_)
        | Command::Cp(_)
        | Command::Split(_)
        | Command::Combine(_)
        | Command::Rm(_)
        | Command::Export(_)
        | Command::Import(_) => true,
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => matches!(args.action, AccountsAction::Cp(_)),
            ClaudeCommand::Cache(_) => false,
        },
        _ => false,
    }
}

/// Run `command` and log it in chatkeep's history, as a Cursor command is logged.
pub fn perform_recorded(rt: &claude::Runtime, command: Command) -> Result<Outcome> {
    let started = Instant::now();
    let name = cli::command_name(&command);
    let args = cli::command_args(&command);
    let result = perform(rt, command);
    if !rt.dry_run {
        let outcome = if result.is_ok() { "ok" } else { "error" };
        append_history(&rt.layout.history_file(), name, &args, outcome, started);
    }
    result
}

fn finished(rt: &claude::Runtime, title: &'static str, report: Report) -> Outcome {
    Outcome::Finished {
        dry_run: rt.dry_run,
        title,
        report,
    }
}

fn text(text: String) -> Outcome {
    Outcome::Text { text, note: None }
}

pub fn perform(rt: &claude::Runtime, command: Command) -> Result<Outcome> {
    match command {
        Command::Ls(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            list(&rt, args.id.as_deref())
        }
        Command::Stats(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            Ok(text(stats::render(Theme::stdout(), &stats::collect(&rt)?)))
        }
        Command::Mv(args) => relocate(rt, args, false),
        Command::Cp(args) => relocate(rt, args, true),
        Command::Split(args) => split(rt, args),
        Command::Combine(args) => combine(rt, args),
        Command::Rm(args) => remove(rt, args.target),
        Command::Export(args) => export(rt, args),
        Command::Import(args) => import(rt, args),
        Command::Claude(args) => match args.command {
            ClaudeCommand::Accounts(args) => match args.action {
                AccountsAction::Ls => Ok(text(accounts::render(rt, Theme::stdout())?)),
                AccountsAction::Cp(args) => copy_account(rt, args),
            },
            ClaudeCommand::Cache(args) => match args.action {
                ClaudeCacheAction::Clear(_) => clear_cache(rt),
                ClaudeCacheAction::Stats => Ok(text(cache_stats(rt)?)),
            },
        },
        Command::Cursor(_) => bail!("chatkeep cursor commands only work on Cursor"),
        other => bail!(
            "{} does not work on Claude Code data",
            cli::command_name(&other)
        ),
    }
}

fn list(rt: &claude::Runtime, id: Option<&str>) -> Result<Outcome> {
    let mut projects = store::discover(rt)?;
    let theme = Theme::stdout();
    Ok(text(match id {
        Some(id) => match store::find(&projects, rt, id) {
            Some(project) => view::render_detail(theme, project),
            None => bail!("no Claude Code project matches {id}"),
        },
        None => {
            view::sort(&mut projects);
            view::render_list(theme, &projects)
        }
    }))
}

pub fn has_project(rt: &claude::Runtime, spec: &str) -> Result<bool> {
    Ok(store::find(&store::discover(rt)?, rt, spec).is_some())
}

pub fn has_target(rt: &claude::Runtime, spec: &str) -> Result<bool> {
    Ok(ops::find_target(rt, spec)?.is_some())
}

/// Whether `combine` can take chats from `spec`: a project or a single session.
pub fn has_source(rt: &claude::Runtime, spec: &str) -> Result<bool> {
    transfer::is_source(rt, spec)
}

pub fn move_matches(rt: &claude::Runtime, args: &PathArgs) -> Result<bool> {
    let replace = cli::replace_pair(&args.common)?;
    let pairs: Vec<(String, String)> = match (&args.from, &args.to) {
        (Some(from), Some(to)) => vec![(from.clone(), to.clone())],
        _ => Vec::new(),
    };
    if replace.is_none() && pairs.is_empty() {
        return Ok(false);
    }
    ops::move_matches(
        rt,
        &pairs,
        replace
            .as_ref()
            .map(|(from, to)| (from.as_str(), to.as_str())),
        args.common.regex,
    )
}

/// How a project shows up in a picker and what a command takes to name it.
fn project_spec(project: &store::Project) -> String {
    project
        .path
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.slug.clone())
}

fn pick_projects(rt: &claude::Runtime, prompt: &str) -> Result<Vec<String>> {
    let mut projects = store::discover(rt)?;
    if projects.is_empty() {
        bail!("no Claude Code projects were found");
    }
    view::sort(&mut projects);
    let theme = Theme::stderr();
    let total = ui::term::stderr_width();
    let labels: Vec<String> = projects
        .iter()
        .map(|project| {
            let mut lead = ui::plural(project.sessions.len(), "session", "sessions");
            lead.push_str("  ");
            if project.destination_missing() {
                lead.push_str(theme.icons().cross);
                lead.push(' ');
            }
            ui::picker_label(&lead, &view::name(project), total, theme.icons().ellipsis)
        })
        .collect();
    Ok(ui::multi_select(prompt, &labels, rt.interactive)?
        .into_iter()
        .map(|index| project_spec(&projects[index]))
        .collect())
}

fn pick_project(rt: &claude::Runtime, prompt: &str) -> Result<String> {
    pick_projects(rt, prompt)?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("a project is required"))
}

/// `mv` and `cp`: the same arguments, the same plan table, a different operation.
fn relocate(rt: &claude::Runtime, args: PathArgs, copy: bool) -> Result<Outcome> {
    let name = if copy { "cp" } else { "mv" };
    let replace = cli::replace_pair(&args.common)?;
    let pairs = match (args.from, args.to, &replace) {
        (_, _, Some(_)) => Vec::new(),
        (Some(from), Some(to), None) => vec![(from, to)],
        _ if !rt.interactive => bail!("{name} needs FROM and TO, or --replace FROM TO"),
        _ => {
            let from = pick_project(
                rt,
                if copy {
                    "Copy which project?"
                } else {
                    "Move which project?"
                },
            )?;
            let to = ui::input("Destination path", rt.interactive)?;
            vec![(from, to)]
        }
    };
    let replace = replace
        .as_ref()
        .map(|(from, to)| (from.as_str(), to.as_str()));
    let regex = args.common.regex;
    if copy {
        let (plans, report) = transfer::plan_copy(rt, &pairs, replace, regex, args.project)?;
        let rows: Vec<Vec<String>> = plans
            .iter()
            .map(|plan| vec![plan.batch.from.clone(), plan.batch.to.display().to_string()])
            .collect();
        confirm_warnings(rt, "Copy plan (Claude Code)", &rows, &report)?;
        let report = transfer::execute_copy(rt, plans, report)?;
        return Ok(finished(rt, "Copy (Claude Code)", report));
    }
    let (plans, report) = ops::plan_move(rt, &pairs, replace, regex, args.project)?;
    let rows: Vec<Vec<String>> = plans
        .iter()
        .map(|plan| vec![plan.from.clone(), plan.to.display().to_string()])
        .collect();
    confirm_warnings(rt, "Move plan (Claude Code)", &rows, &report)?;
    let report = ops::execute_move(rt, plans, report)?;
    Ok(finished(rt, "Move (Claude Code)", report))
}

/// A plan with warnings is shown and needs `-y` before it runs.
fn confirm_warnings(
    rt: &claude::Runtime,
    title: &str,
    rows: &[Vec<String>],
    report: &Report,
) -> Result<()> {
    if report.warnings.is_empty() || rt.yes || rt.quiet || rows.is_empty() {
        return Ok(());
    }
    print!(
        "{}",
        validation(
            Theme::stdout(),
            title,
            &["project", "destination"],
            rows,
            &report.warnings,
        )
    );
    Err(ui::hinted(
        "warnings need confirmation",
        "Pass -y to continue.",
    ))
}

fn split(rt: &claude::Runtime, args: SplitArgs) -> Result<Outcome> {
    let source = match args.source {
        Some(source) => source,
        None => pick_project(rt, "Split which project?")?,
    };
    let targets = cli::split_targets(args.targets, rt.interactive)?;
    let target_paths: Vec<PathBuf> = targets.iter().map(PathBuf::from).collect();
    let projects = store::discover(rt)?;
    let project = store::find(&projects, rt, &source)
        .ok_or_else(|| anyhow::anyhow!("no Claude Code project matches {source}"))?;
    let suggestion = transfer::suggest_split(rt, project, &target_paths)?;
    let assigned = cli::split_assignments(rt.interactive, &suggestion, &target_paths)?;
    cli::confirm_split(rt.quiet, rt.dry_run, rt.yes, rt.interactive, &assigned)?;
    let report = transfer::split(rt, project, &target_paths, &assigned, args.move_chats)?;
    Ok(finished(rt, "Split (Claude Code)", report))
}

fn combine(rt: &claude::Runtime, args: CombineArgs) -> Result<Outcome> {
    let target = match args.target {
        Some(target) => target,
        None => ui::input("Target folder or project", rt.interactive)?,
    };
    let sources = if args.sources.is_empty() {
        pick_projects(rt, "Combine which sources?")?
    } else {
        args.sources
    };
    let rows: Vec<Vec<String>> = sources
        .iter()
        .map(|source| vec![source.clone(), target.clone()])
        .collect();
    if !rt.quiet {
        print!(
            "{}",
            validation(
                Theme::stdout(),
                "Combine plan (Claude Code)",
                &["source", "target"],
                &rows,
                &[]
            )
        );
    }
    if !rt.dry_run && !ui::confirm("Continue with this combine?", rt.yes, rt.interactive)? {
        bail!("aborted");
    }
    let report = transfer::combine(rt, &target, &sources, args.move_chats)?;
    Ok(finished(rt, "Combine (Claude Code)", report))
}

fn remove(rt: &claude::Runtime, target: Option<String>) -> Result<Outcome> {
    let specs = match target {
        Some(target) => vec![target],
        None if rt.interactive => pick_projects(rt, "Remove which projects?")?,
        None => bail!("rm needs a project path or a session id"),
    };
    let mut targets = Vec::new();
    for spec in &specs {
        let Some(target) = ops::find_target(rt, spec)? else {
            bail!("no Claude Code project or session matches {spec}");
        };
        targets.push(target);
    }
    if !rt.dry_run && !rt.yes {
        let rows: Vec<Vec<String>> = targets
            .iter()
            .map(|target| vec![target.label(), target.location()])
            .collect();
        print!(
            "{}",
            validation(
                Theme::stdout(),
                "Remove plan (Claude Code)",
                &["target", "holds"],
                &rows,
                &[]
            )
        );
    }
    if !rt.dry_run
        && !ui::confirm(
            "Remove the selected Claude Code chats?",
            rt.yes,
            rt.interactive,
        )?
    {
        bail!("aborted");
    }
    let report = ops::remove(rt, &targets)?;
    Ok(finished(rt, "Remove (Claude Code)", report))
}

fn export(rt: &claude::Runtime, args: ExportArgs) -> Result<Outcome> {
    let target = match args.target {
        Some(target) => target,
        None => pick_project(rt, "Export which project?")?,
    };
    let file = match args.file {
        Some(file) => file,
        None if rt.interactive => ui::input("Archive path", true)?,
        None => bail!("export needs an archive path"),
    };
    let projects = store::discover(rt)?;
    let project = store::find(&projects, rt, &target)
        .ok_or_else(|| anyhow::anyhow!("no Claude Code project matches {target}"))?;
    let report = archive::export(rt, project, PathBuf::from(file).as_path())?;
    Ok(finished(rt, "Export (Claude Code)", report))
}

fn import(rt: &claude::Runtime, args: ImportArgs) -> Result<Outcome> {
    let file = match args.file {
        Some(file) => PathBuf::from(file),
        None if rt.interactive => PathBuf::from(ui::input("Archive path", true)?),
        None => bail!("import needs an archive file"),
    };
    let dest = args.to.map(PathBuf::from);
    let report = archive::import(rt, &file, dest.as_deref(), args.overwrite)?;
    Ok(finished(rt, "Import (Claude Code)", report))
}

fn copy_account(rt: &claude::Runtime, args: AccountCpArgs) -> Result<Outcome> {
    let from = match args.from {
        Some(from) => from,
        None => pick_account(rt, args.to.as_deref())?,
    };
    let (plan, report) = accounts::plan_copy(rt, &from, args.to.as_deref())?;
    if !plan.is_empty() && !rt.quiet {
        print!(
            "{}",
            validation(
                Theme::stdout(),
                &format!("Chats account {} will list too", plan.to.account),
                &["chat", "folder"],
                &plan.rows(),
                &[]
            )
        );
    }
    if !plan.is_empty()
        && !rt.dry_run
        && !ui::confirm("Copy these chats to the account?", rt.yes, rt.interactive)?
    {
        bail!("aborted");
    }
    let report = accounts::execute_copy(rt, &plan, report)?;
    Ok(finished(rt, "Accounts copy", report))
}

/// The account to copy from when none was named: the only other one, or the one picked.
fn pick_account(rt: &claude::Runtime, to: Option<&str>) -> Result<String> {
    let target = match to {
        Some(to) => Some(to.to_string()),
        None => accounts::current(&rt.layout),
    };
    let mut others: Vec<String> = accounts::listings(&rt.layout)?
        .into_iter()
        .map(|listing| listing.account)
        .filter(|account| {
            target
                .as_deref()
                .is_none_or(|target| !account.starts_with(target))
        })
        .collect();
    others.dedup();
    match others.as_slice() {
        [] => bail!("the desktop app has no other account on this machine to copy chats from"),
        [only] => Ok(only.clone()),
        _ if !rt.interactive => Err(ui::hinted(
            "accounts cp needs the account to copy from",
            format!("Pass one of: {}.", others.join(", ")),
        )),
        _ => {
            let labels: Vec<&str> = others.iter().map(String::as_str).collect();
            Ok(
                others[ui::select("Copy chats from which account?", &labels, rt.interactive)?]
                    .clone(),
            )
        }
    }
}

fn clear_cache(rt: &claude::Runtime) -> Result<Outcome> {
    let home = &rt.layout.chatkeep_home;
    let files = index::files(home);
    let mut report = Report::default();
    if files.is_empty() {
        return Ok(finished(rt, "Claude Code cache clear", report));
    }
    let size: u64 = files
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum();
    report.applied.push(format!(
        "remove index {} ({})",
        ui::home_relative(&index::db_path(home).display().to_string()),
        ui::format_size(size)
    ));
    if rt.dry_run {
        return Ok(finished(rt, "Claude Code cache clear", report));
    }
    if !ui::confirm("Delete the Claude Code index?", rt.yes, rt.interactive)? {
        bail!("aborted");
    }
    for file in files {
        std::fs::remove_file(&file)
            .map_err(|err| anyhow::anyhow!("failed to remove {}: {err}", file.display()))?;
    }
    Ok(finished(rt, "Claude Code cache clear", report))
}

fn cache_stats(rt: &claude::Runtime) -> Result<String> {
    let theme = Theme::stdout();
    let home = &rt.layout.chatkeep_home;
    let heading = ui::section_line(theme, "Claude Code index");
    if !rt.cache {
        return Ok(format!(
            "{heading}\n{}\n",
            ui::info_line(theme, "The index is off (CHATKEEP_NO_INDEX).")
        ));
    }
    let Some(cache) = index::Cache::open_existing(home)? else {
        return Ok(format!(
            "{heading}\n{}\n",
            ui::info_line(
                theme,
                "No index yet. The next ls or stats for Claude Code builds it."
            )
        ));
    };
    let counts = cache.counts()?;
    let size: u64 = index::files(home)
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum();
    let rows = vec![
        (
            "file",
            theme.cell(
                ui::home_relative(&cache.path().display().to_string()),
                Some(comfy_table::Color::Cyan),
                &[],
            ),
        ),
        ("size", theme.size_cell(size)),
        ("transcripts", theme.count_cell(counts.transcripts, None)),
        (
            "last change",
            theme.cell(
                counts
                    .refreshed_at
                    .and_then(ui::datetime)
                    .unwrap_or_else(|| "never".to_string()),
                None,
                &[],
            ),
        ),
        ("schema", theme.cell(counts.schema, None, &[])),
    ];
    Ok(format!("{heading}\n{}\n", ui::panel(theme, rows)))
}
