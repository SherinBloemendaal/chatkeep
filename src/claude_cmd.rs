//! The Claude Code side of every command.

use anyhow::{Result, bail};
use std::path::PathBuf;
use std::time::Instant;

use crate::claude::accounts::Listing;
use crate::claude::identity::Book;
use crate::claude::{
    self, accounts, archive, autosync, doctor, identity, index, ops, stats, store, sync, transfer,
    view,
};
use crate::cli::{
    self, AccountCpArgs, AccountNameArgs, AccountsAction, ClaudeCacheAction, ClaudeCommand,
    CombineArgs, Command, CommonArgs, ExportArgs, ImportArgs, Outcome, PathArgs, SplitArgs,
    SyncAction, SyncAutoAction,
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
            ClaudeCommand::Cache(_) | ClaudeCommand::Sync(_) => false,
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
                AccountsAction::Name(args) => name_account(rt, args),
                AccountsAction::Doctor => {
                    let checkup = doctor::checkup(rt, doctor::cli_on_path())?;
                    let book = identity::book(rt)?;
                    Ok(text(doctor::render(Theme::stdout(), &checkup, &book)))
                }
                AccountsAction::Prune(_) => prune_caches(rt),
            },
            ClaudeCommand::Cache(args) => match args.action {
                ClaudeCacheAction::Clear(_) => clear_cache(rt),
                ClaudeCacheAction::Stats => Ok(text(cache_stats(rt)?)),
            },
            ClaudeCommand::Sync(args) => match args.action {
                None => run_sync(rt, args.profile.as_deref()),
                Some(SyncAction::Profiles) => Ok(text(sync_profiles(rt)?)),
                Some(SyncAction::Set(args)) => set_sync_profile(rt, &args.name, args.accounts),
                Some(SyncAction::Add(args)) => add_to_sync_profile(rt, &args.name, args.accounts),
                Some(SyncAction::Rm(args)) => remove_sync_profile(rt, &args.name, args.accounts),
                Some(SyncAction::Watch(args)) => {
                    watch_sync(rt, args.profile.as_deref(), args.interval)
                }
                Some(SyncAction::Auto(args)) => auto_sync(rt, &args.action),
                Some(SyncAction::Log(args)) => Ok(text(sync_log(rt, args.last)?)),
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
    let suggestion = cli::with_assignments(
        transfer::suggest_split(rt, project, &target_paths)?,
        &args.assign,
        &target_paths,
        |path| rt.resolve_path(path),
    )?;
    let assigned = cli::split_assignments(
        rt.interactive,
        !args.assign.is_empty(),
        &suggestion,
        &target_paths,
    )?;
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
    let book = identity::book(rt)?;
    let target = target.map(|target| book.expand(&target));
    let listings = accounts::listings(&rt.layout)?;
    let mut others: Vec<&Listing> = listings
        .iter()
        .filter(|listing| {
            target
                .as_deref()
                .is_none_or(|target| !listing.account.starts_with(target))
        })
        .collect();
    others.dedup_by(|a, b| a.account == b.account);
    match others.as_slice() {
        [] => bail!("the desktop app has no other account on this machine to copy chats from"),
        [only] => Ok(only.account.clone()),
        _ if !rt.interactive => Err(ui::lacking(
            ui::Lacking::Argument,
            "accounts cp needs the account to copy from",
            format!(
                "Pass one of: {}.",
                others
                    .iter()
                    .map(|listing| book.label(&listing.account))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
        _ => {
            let rows = accounts::picker_rows(&book, &others, ui::term::stderr_width());
            let labels: Vec<&str> = rows.iter().map(String::as_str).collect();
            Ok(
                others[ui::select("Copy chats from which account?", &labels, rt.interactive)?]
                    .account
                    .clone(),
            )
        }
    }
}

/// `accounts name`: give an account a name, picking the account and asking the name when they
/// are not given.
fn name_account(rt: &claude::Runtime, args: AccountNameArgs) -> Result<Outcome> {
    let book = identity::book(rt)?;
    let listings = accounts::listings(&rt.layout)?;
    let account = match args.account {
        Some(spec) => account_of(&listings, &book, &spec)?,
        None => {
            let mut choices: Vec<&Listing> = listings.iter().collect();
            choices.dedup_by(|a, b| a.account == b.account);
            if choices.is_empty() {
                bail!("the desktop app has no account on this machine to name");
            }
            if !rt.interactive {
                return Err(ui::lacking(
                    ui::Lacking::Argument,
                    "accounts name needs the account to name",
                    "Pass its id or the start of it; chatkeep claude accounts ls shows them.",
                ));
            }
            let rows = accounts::picker_rows(&book, &choices, ui::term::stderr_width());
            let labels: Vec<&str> = rows.iter().map(String::as_str).collect();
            choices[ui::select("Name which account?", &labels, rt.interactive)?]
                .account
                .clone()
        }
    };
    let name = match (args.clear, args.name) {
        (true, _) => None,
        (false, Some(name)) => Some(name),
        (false, None) => Some(ui::input(
            &format!("Name for {}", book.label(&account)),
            rt.interactive,
        )?),
    };
    identity::set_name(rt, &account, name.as_deref())?;
    let mut report = Report::default();
    report.applied.push(match &name {
        Some(name) => format!("name account {} -> {name}", identity::short(&account)),
        None => format!("clear the name of account {}", identity::short(&account)),
    });
    Ok(finished(rt, "Account name", report))
}

/// The one account `spec` names, whatever organizations it has.
fn account_of(listings: &[Listing], book: &Book, spec: &str) -> Result<String> {
    let wanted = book.expand(spec);
    let wanted = wanted.split('/').next().unwrap_or_default();
    let mut found: Vec<&str> = listings
        .iter()
        .map(|listing| listing.account.as_str())
        .filter(|account| !wanted.is_empty() && account.starts_with(wanted))
        .collect();
    found.dedup();
    match found.as_slice() {
        [only] => Ok((*only).to_string()),
        [] => Err(ui::hinted(
            format!("the desktop app has no account {spec} on this machine"),
            "Run chatkeep claude accounts ls to see the accounts it has.",
        )),
        _ => bail!("{spec} matches several accounts: {}", found.join(", ")),
    }
}

/// `accounts prune`: show the cache entries of accounts not in use, and remove them after a yes.
fn prune_caches(rt: &claude::Runtime) -> Result<Outcome> {
    let plan = doctor::plan_prune(rt)?;
    let book = identity::book(rt)?;
    if !plan.stale.is_empty() && !rt.quiet {
        print!(
            "{}",
            validation(
                Theme::stdout(),
                "Cache entries of accounts not in use",
                &["cache", "belongs to"],
                &plan.rows(&book),
                &[]
            )
        );
    }
    if !plan.stale.is_empty()
        && !rt.dry_run
        && !ui::confirm("Remove these cache entries?", rt.yes, rt.interactive)?
    {
        bail!("aborted");
    }
    let report = doctor::execute_prune(rt, &plan)?;
    Ok(finished(rt, "Accounts prune", report))
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

fn sync_rows(steps: &[&sync::Step]) -> Vec<Vec<String>> {
    steps
        .iter()
        .map(|step| {
            vec![
                step.action.verb().to_string(),
                if step.title.is_empty() {
                    step.chat.clone()
                } else {
                    step.title.clone()
                },
                step.account.clone(),
            ]
        })
        .collect()
}

/// `claude sync`: bring the accounts of each profile in step. Additions and updates need one
/// yes (or `-y`); removing a chat another account deleted is always asked on its own, and
/// only of a person at a terminal.
fn run_sync(rt: &claude::Runtime, name: Option<&str>) -> Result<Outcome> {
    let config = sync::load(&rt.layout.chatkeep_home)?;
    let mut report = Report::default();
    for (name, profile) in sync::selected(&config, name)? {
        let plan = sync::plan(rt, &name, &profile)?;
        let ready: Vec<&sync::Step> = plan.ready().collect();
        if !ready.is_empty() && !rt.quiet {
            print!(
                "{}",
                validation(
                    Theme::stdout(),
                    &format!("Sync plan for {name}"),
                    &["change", "chat", "account"],
                    &sync_rows(&ready),
                    &[]
                )
            );
        }
        if !ready.is_empty()
            && !rt.dry_run
            && !ui::confirm("Sync these chats?", rt.yes, rt.interactive)?
        {
            bail!("aborted");
        }
        let removable: Vec<&sync::Step> = plan.removals.iter().filter(|step| !step.waits).collect();
        let mut remove = false;
        if !removable.is_empty() && rt.interactive && !rt.dry_run {
            print!(
                "{}",
                validation(
                    Theme::stdout(),
                    &format!("Deleted in another account of {name}"),
                    &["change", "chat", "account"],
                    &sync_rows(&removable),
                    &[]
                )
            );
            remove = ui::confirm("Remove these chats from these accounts too?", false, true)?;
        }
        let done = sync::execute(rt, &plan, remove, "manual")?;
        report.applied.extend(done.applied);
        report.warnings.extend(done.warnings);
    }
    Ok(finished(rt, "Sync (Claude Code)", report))
}

fn sync_profiles(rt: &claude::Runtime) -> Result<String> {
    let theme = Theme::stdout();
    let config = sync::load(&rt.layout.chatkeep_home)?;
    let heading = ui::section_line(theme, "Sync profiles");
    if config.profiles.is_empty() {
        return Ok(format!(
            "{heading}\n{}\n{}\n",
            ui::info_line(theme, "No sync profile exists yet."),
            ui::hint_line(
                theme,
                "Create one with: chatkeep claude sync set NAME (pick the accounts from a list)"
            )
        ));
    }
    let signed_in = accounts::current(&rt.layout);
    let book = identity::book(rt)?;
    let mut sheet = ui::Sheet::new(
        theme,
        &[
            ("profile", ui::Align::Left),
            ("account", ui::Align::Left),
            ("id", ui::Align::Left),
            ("state", ui::Align::Left),
        ],
    )
    .optional(&[2]);
    for (name, profile) in &config.profiles {
        for account in &profile.accounts {
            let here = signed_in
                .as_deref()
                .is_some_and(|current| account.starts_with(current));
            sheet.row(vec![
                theme.cell(name, Some(comfy_table::Color::Cyan), &[]),
                theme.cell(
                    book.label(account_id(account)),
                    Some(comfy_table::Color::Magenta),
                    &[],
                ),
                theme.cell(account, None, &[comfy_table::Attribute::Dim]),
                theme.cell(if here { "signed in" } else { "" }, None, &[]),
            ]);
        }
    }
    Ok(format!(
        "{heading}\n{sheet}\n{}\n",
        ui::info_line(theme, &autosync::status_line(&rt.layout.chatkeep_home)?)
    ))
}

/// The account part of a profile's `account/organization` label.
fn account_id(label: &str) -> &str {
    label.split('/').next().unwrap_or(label)
}

/// A profile's accounts as people read them. An account that syncs with several of its
/// organizations names the organization too.
fn describe(book: &Book, labels: &[String]) -> String {
    labels
        .iter()
        .map(|label| {
            let account = account_id(label);
            let mut text = book.label(account);
            let repeated = labels
                .iter()
                .filter(|other| account_id(other) == account)
                .count()
                > 1;
            if repeated && let Some((_, organization)) = label.split_once('/') {
                text.push_str(&format!(" (org {})", identity::short(organization)));
            }
            text
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Let the user check accounts in a list. Only `offered` listings are shown; those in
/// `checked` start checked. Returns the `account/organization` labels picked.
fn pick_listings(
    rt: &claude::Runtime,
    book: &Book,
    prompt: &str,
    offered: &[&Listing],
    checked: &[String],
) -> Result<Vec<String>> {
    let rows = accounts::picker_rows(book, offered, ui::term::stderr_width());
    let ticks: Vec<bool> = offered
        .iter()
        .map(|listing| checked.contains(&listing.label()))
        .collect();
    let picked = ui::multi_select_checked(prompt, &rows, &ticks, rt.interactive)?;
    Ok(picked
        .into_iter()
        .map(|index| offered[index].label())
        .collect())
}

/// Accounts that sync in a profile other than `name`, by label, with that profile.
fn taken_elsewhere(config: &sync::Config, name: &str) -> Vec<(String, String)> {
    config
        .profiles
        .iter()
        .filter(|(other, _)| other.as_str() != name)
        .flat_map(|(other, profile)| {
            profile
                .accounts
                .iter()
                .map(move |label| (label.clone(), other.clone()))
        })
        .collect()
}

fn needs_terminal(action: &str, name: &str) -> anyhow::Error {
    ui::lacking(
        ui::Lacking::Argument,
        format!("sync {action} {name} needs the accounts"),
        "Name them (a name, e-mail address or id each), or run this in a terminal to pick them from a list. chatkeep claude accounts ls shows them.",
    )
}

fn set_sync_profile(rt: &claude::Runtime, name: &str, specs: Vec<String>) -> Result<Outcome> {
    let book = identity::book(rt)?;
    let specs = if specs.is_empty() {
        if !rt.interactive {
            return Err(needs_terminal("set", name));
        }
        let config = sync::load(&rt.layout.chatkeep_home)?;
        let taken = taken_elsewhere(&config, name);
        let listings = accounts::listings(&rt.layout)?;
        let offered: Vec<&Listing> = listings
            .iter()
            .filter(|listing| !taken.iter().any(|(label, _)| *label == listing.label()))
            .collect();
        if offered.len() < 2 {
            bail!("fewer than two accounts are free to sync in profile {name}");
        }
        note_taken(&book, &listings, &taken, rt.quiet);
        let current = config
            .profiles
            .get(name)
            .map(|profile| profile.accounts.clone())
            .unwrap_or_default();
        let picked = pick_listings(
            rt,
            &book,
            &format!("Accounts of profile {name} (space checks, enter saves)"),
            &offered,
            &current,
        )?;
        if picked.len() < 2 {
            bail!("pick at least two accounts for profile {name}");
        }
        picked
    } else {
        specs
    };
    let profile = sync::set_profile(rt, name, &specs)?;
    let mut report = Report::default();
    report.applied.push(format!(
        "set profile {name} -> {}",
        describe(&book, &profile.accounts)
    ));
    Ok(finished(rt, "Sync profile", report))
}

/// Say which accounts the picker leaves out because another profile has them.
fn note_taken(book: &Book, listings: &[Listing], taken: &[(String, String)], quiet: bool) {
    let shown: Vec<String> = taken
        .iter()
        .filter(|(label, _)| listings.iter().any(|listing| listing.label() == *label))
        .map(|(label, profile)| format!("{} (in {profile})", book.label(account_id(label))))
        .collect();
    if !shown.is_empty() && !quiet {
        eprintln!(
            "{}",
            ui::hint_line(
                Theme::stderr(),
                &format!(
                    "Not listed, they sync in another profile: {}",
                    shown.join(", ")
                )
            )
        );
    }
}

fn add_to_sync_profile(rt: &claude::Runtime, name: &str, specs: Vec<String>) -> Result<Outcome> {
    let book = identity::book(rt)?;
    let specs = if specs.is_empty() {
        let config = sync::load(&rt.layout.chatkeep_home)?;
        if !config.profiles.contains_key(name) {
            return Err(ui::hinted(
                format!("no sync profile named {name}"),
                format!("Create it with: chatkeep claude sync set {name}"),
            ));
        }
        if !rt.interactive {
            return Err(needs_terminal("add", name));
        }
        let in_any: Vec<(String, String)> = taken_elsewhere(&config, "");
        let listings = accounts::listings(&rt.layout)?;
        let offered: Vec<&Listing> = listings
            .iter()
            .filter(|listing| !in_any.iter().any(|(label, _)| *label == listing.label()))
            .collect();
        if offered.is_empty() {
            bail!("every account on this machine already syncs in a profile");
        }
        let elsewhere: Vec<(String, String)> = in_any
            .into_iter()
            .filter(|(_, profile)| profile != name)
            .collect();
        note_taken(&book, &listings, &elsewhere, rt.quiet);
        let picked = pick_listings(
            rt,
            &book,
            &format!("Add to profile {name} (space checks, enter saves)"),
            &offered,
            &[],
        )?;
        if picked.is_empty() {
            bail!("no account picked");
        }
        picked
    } else {
        specs
    };
    let profile = sync::add_to_profile(rt, name, &specs)?;
    let mut report = Report::default();
    report.applied.push(format!(
        "profile {name} -> {}",
        describe(&book, &profile.accounts)
    ));
    Ok(finished(rt, "Sync profile", report))
}

/// `sync rm NAME [ACCOUNTS]`: take accounts out of a profile, or remove the profile. At a
/// terminal, a bare `sync rm NAME` asks which of the two is meant.
fn remove_sync_profile(rt: &claude::Runtime, name: &str, specs: Vec<String>) -> Result<Outcome> {
    let book = identity::book(rt)?;
    let mut specs = specs;
    if specs.is_empty() && rt.interactive && !rt.yes {
        let config = sync::load(&rt.layout.chatkeep_home)?;
        let Some(profile) = config.profiles.get(name) else {
            bail!("no sync profile named {name}");
        };
        let choice = ui::select(
            &format!("What should go from profile {name}?"),
            &["Some of its accounts", "The whole profile"],
            true,
        )?;
        if choice == 0 {
            let listings = accounts::listings(&rt.layout)?;
            let members: Vec<&Listing> = profile
                .accounts
                .iter()
                .filter_map(|label| listings.iter().find(|listing| listing.label() == *label))
                .collect();
            let missing: Vec<&String> = profile
                .accounts
                .iter()
                .filter(|label| !listings.iter().any(|listing| listing.label() == **label))
                .collect();
            let mut rows = accounts::picker_rows(&book, &members, ui::term::stderr_width());
            rows.extend(missing.iter().map(|label| {
                format!(
                    "{}  (no longer on this machine)",
                    book.label(account_id(label))
                )
            }));
            let labels: Vec<String> = members
                .iter()
                .map(|listing| listing.label())
                .chain(missing.iter().map(|label| (*label).clone()))
                .collect();
            let picked = ui::multi_select(
                &format!("Remove from profile {name} (space checks, enter saves)"),
                &rows,
                true,
            )?;
            if picked.is_empty() {
                bail!("no account picked");
            }
            specs = picked
                .into_iter()
                .map(|index| labels[index].clone())
                .collect();
        }
    }
    let mut report = Report::default();
    if specs.is_empty() {
        if !sync::remove_profile(rt, name)? {
            bail!("no sync profile named {name}");
        }
        report.applied.push(format!("remove profile {name}"));
    } else {
        let profile = sync::remove_from_profile(rt, name, &specs)?;
        report.applied.push(format!(
            "profile {name} -> {}",
            describe(&book, &profile.accounts)
        ));
    }
    Ok(finished(rt, "Sync profile", report))
}

/// What the watcher says about itself: on the terminal, and as a note in the sync log.
fn watch_note(rt: &claude::Runtime, line: &str) {
    watch_line(line);
    let _ = sync::log_note(&rt.layout.chatkeep_home, line);
}

fn watch_line(line: &str) {
    println!(
        "{} {line}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );
}

/// `claude sync log`: the newest changes. A change made several times in a row is one row.
fn sync_log(rt: &claude::Runtime, last: usize) -> Result<String> {
    let theme = Theme::stdout();
    let heading = ui::section_line(theme, "Sync log");
    let mut rows = sync::read_log(&rt.layout.chatkeep_home)?;
    let skip = rows.len().saturating_sub(last.max(1));
    let rows = rows.split_off(skip);
    if rows.is_empty() {
        return Ok(format!(
            "{heading}\n{}\n",
            ui::info_line(theme, "No sync has changed anything yet.")
        ));
    }
    let mut sheet = ui::Sheet::new(
        theme,
        &[
            ("when", ui::Align::Left),
            ("change", ui::Align::Left),
            ("chat", ui::Align::Left),
            ("what", ui::Align::Left),
            ("account", ui::Align::Left),
            ("by", ui::Align::Left),
        ],
    )
    .flex(2)
    .min_flex(16)
    .optional(&[3, 5]);
    for entry in &rows {
        let when = chrono::DateTime::parse_from_rfc3339(&entry.at)
            .map(|at| {
                at.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|_| entry.at.clone());
        let change = if entry.times > 1 {
            format!("{} x{}", entry.action, entry.times)
        } else {
            entry.action.clone()
        };
        let color = match entry.action.as_str() {
            "add" => Some(comfy_table::Color::Green),
            "update" => Some(comfy_table::Color::Cyan),
            "remove" => Some(comfy_table::Color::Red),
            _ => None,
        };
        let chat = if entry.title.is_empty() {
            entry.chat.clone()
        } else {
            entry.title.clone()
        };
        sheet.row(vec![
            theme.cell(when, None, &[comfy_table::Attribute::Dim]),
            theme.cell(change, color, &[]),
            theme.cell(chat, None, &[]),
            theme.cell(
                entry.changed.join(", "),
                None,
                &[comfy_table::Attribute::Dim],
            ),
            theme.cell(
                entry.account.split('-').next().unwrap_or_default(),
                Some(comfy_table::Color::Magenta),
                &[],
            ),
            theme.cell(&entry.source, None, &[comfy_table::Attribute::Dim]),
        ]);
    }
    Ok(format!("{heading}\n{sheet}\n"))
}

/// `claude sync watch`: look at the chat lists every few seconds and sync when one changed.
/// It never asks and never removes; it runs until it is stopped.
fn watch_sync(rt: &claude::Runtime, name: Option<&str>, interval: u64) -> Result<Outcome> {
    let config = sync::load(&rt.layout.chatkeep_home)?;
    if let Some(name) = name {
        sync::selected(&config, Some(name))?;
    }
    let mut rt = rt.clone();
    rt.yes = true;
    rt.quiet = true;
    rt.interactive = false;
    let pause = std::time::Duration::from_secs(interval.max(1));
    watch_note(&rt, "watching the chat lists of every sync profile");
    let mut last = String::new();
    let mut failing: Option<String> = None;
    loop {
        match sync::watch_round(&rt, name, &mut last) {
            Ok(done) => {
                failing = None;
                if let Some(report) = done {
                    for line in report.applied.iter().chain(&report.warnings) {
                        watch_line(line);
                    }
                }
            }
            // A profile may not exist yet, or a list may be half written: say it once, retry.
            Err(err) => {
                let message = format!("{err:#}");
                if failing.as_deref() != Some(message.as_str()) {
                    watch_note(&rt, &format!("waiting: {message}"));
                    failing = Some(message);
                }
            }
        }
        std::thread::sleep(pause);
    }
}

fn auto_sync(rt: &claude::Runtime, action: &SyncAutoAction) -> Result<Outcome> {
    let home = &rt.layout.chatkeep_home;
    let theme = Theme::stdout();
    Ok(text(match action {
        SyncAutoAction::Status => {
            format!("{}\n", ui::info_line(theme, &autosync::status_line(home)?))
        }
        SyncAutoAction::On => {
            let config = sync::load(home)?;
            sync::selected(&config, None)?;
            let path = autosync::enable()?;
            format!(
                "{}\n{}\n",
                ui::ok_line(
                    theme,
                    &format!(
                        "background sync is on ({})",
                        ui::home_relative(&path.display().to_string())
                    )
                ),
                ui::hint_line(
                    theme,
                    "It adds and updates on its own. Chats deleted in one account wait until you run chatkeep claude sync."
                )
            )
        }
        SyncAutoAction::Off => {
            let line = if autosync::disable()? {
                "background sync is off"
            } else {
                "background sync was not on"
            };
            format!("{}\n", ui::ok_line(theme, line))
        }
    }))
}
