//! Command surface for chatkeep.

use anyhow::{Context, Result, bail};
use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{
    ArgAction, Args, ColorChoice, CommandFactory, FromArgMatches, Parser, Subcommand, ValueHint,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::claude;
use crate::config;
use crate::cursor::install::{self, DEFAULT};
use crate::cursor::process::NativeProcesses;
use crate::cursor::uri::{self, Platform};
use crate::engine::{
    self, Layout, Runtime, SystemProbe, Workspace, cache, combine_workspaces, copy_paths,
    export_workspace, import_archive, index, move_paths, record_history, reindex, remove_targets,
    save_unsaved, show_history, split_workspace, suggest_split,
};
use crate::ui::{self, ColorMode, Theme, validation};

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Blue.on_default().effects(Effects::BOLD))
        .usage(AnsiColor::Blue.on_default().effects(Effects::BOLD))
        .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
        .placeholder(AnsiColor::BrightBlack.on_default())
        .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
        .valid(AnsiColor::Green.on_default())
        .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD))
}

#[derive(Parser, Debug)]
#[command(
    name = "chatkeep",
    version,
    about = "Keep AI chat history attached to your projects",
    styles = styles(),
    disable_help_subcommand = true,
    disable_help_flag = true
)]
pub struct Cli {
    /// Show help.
    #[arg(short = 'h', long = "help", action = ArgAction::SetTrue)]
    pub help: bool,
    /// Color output: auto, always, or never.
    #[arg(long, global = true, value_enum, value_name = "WHEN", default_value_t = ColorMode::Auto, value_hint = ValueHint::Other)]
    pub color: ColorMode,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Repath chats after a project folder moved. Alias: move.
    #[command(alias = "move")]
    Mv(PathArgs),
    /// Copy a project's chats to another folder. Alias: copy.
    #[command(alias = "copy")]
    Cp(PathArgs),
    /// Copy chats from one project into separate projects.
    Split(SplitArgs),
    /// Bring chats from several sources into one project.
    Combine(CombineArgs),
    /// List projects and their chats. Alias: list.
    #[command(alias = "list")]
    Ls(ListArgs),
    /// Remove a project's chats or a single chat.
    Rm(TargetArgs),
    /// Write a .chatkeep gzip archive.
    Export(ExportArgs),
    /// Restore a .chatkeep archive.
    Import(ImportArgs),
    /// Show the local command log.
    History(CommonArgs),
    /// Show projects, chats, tokens, and models.
    Stats(StatsArgs),
    /// Queue write commands while a tool is still open.
    Queue(QueueArgs),
    /// Commands that only exist for Cursor.
    Cursor(CursorArgs),
    /// Commands that only exist for Claude Code.
    Claude(ClaudeArgs),
    #[command(name = "__refresh-index", hide = true)]
    RefreshIndex,
    /// Download and install the latest release.
    Update,
    /// Remove this install and its PATH entry.
    Uninstall(UninstallArgs),
    /// Open the GitHub repository in the browser.
    Github,
    /// Show help, or every option of one command.
    Help(HelpArgs),
}

#[derive(Args, Debug, Clone)]
pub struct CursorArgs {
    #[command(subcommand)]
    pub command: CursorCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CursorCommand {
    /// Attach an unsaved Workspaces/<ts> session to a real folder.
    Save(SaveArgs),
    /// Rebuild registry refs, rewrite leftover paths, and clear caches. Alias: reindex.
    #[command(alias = "reindex")]
    Rx(TargetArgs),
    /// Inspect, rebuild, or clear the persistent index.
    Cache(CacheArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ClaudeArgs {
    #[command(subcommand)]
    pub command: ClaudeCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ClaudeCommand {
    /// The accounts of the Claude desktop app and the chats each one lists.
    Accounts(AccountsArgs),
    /// Inspect or clear the index of what every transcript says.
    Cache(ClaudeCacheArgs),
    /// Keep the chat lists of several desktop app accounts the same.
    Sync(SyncArgs),
}

#[derive(Args, Debug, Clone)]
#[command(args_conflicts_with_subcommands = true)]
pub struct SyncArgs {
    #[command(subcommand)]
    pub action: Option<SyncAction>,
    #[command(flatten)]
    pub flags: WriteFlags,
    /// The sync profile to run. Without it, every profile.
    #[arg(value_hint = ValueHint::Other)]
    pub profile: Option<String>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SyncAction {
    /// Show the sync profiles and their accounts. Alias: ls.
    #[command(alias = "ls")]
    Profiles,
    /// Create or replace a sync profile: the accounts that share their chats.
    Set(SyncSetArgs),
    /// Remove a sync profile. The chat lists stay as they are.
    Rm(SyncRmArgs),
    /// Keep syncing: act whenever a chat list changes, until stopped.
    Watch(SyncWatchArgs),
    /// Run the watcher in the background from login on.
    Auto(SyncAutoArgs),
    /// Show what the syncs changed, newest last.
    Log(SyncLogArgs),
}

#[derive(Args, Debug, Clone)]
pub struct SyncLogArgs {
    /// How many rows to show, counted from the newest.
    #[arg(long, default_value_t = 30, value_name = "ROWS", value_hint = ValueHint::Other)]
    pub last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct SyncSetArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// The name of the profile, e.g. work.
    #[arg(value_hint = ValueHint::Other)]
    pub name: String,
    /// The accounts in it: each an id or the start of one, or ACCOUNT/ORGANIZATION.
    #[arg(required = true, num_args = 2.., value_hint = ValueHint::Other)]
    pub accounts: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SyncRmArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// The profile to remove.
    #[arg(value_hint = ValueHint::Other)]
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct SyncWatchArgs {
    /// The sync profile to watch. Without it, every profile.
    #[arg(value_hint = ValueHint::Other)]
    pub profile: Option<String>,
    /// Seconds between two looks at the chat lists.
    #[arg(long, default_value_t = 5, value_name = "SECONDS", value_hint = ValueHint::Other)]
    pub interval: u64,
}

#[derive(Args, Debug, Clone)]
pub struct SyncAutoArgs {
    #[command(subcommand)]
    pub action: SyncAutoAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum SyncAutoAction {
    /// Start the background sync now and at every login.
    On,
    /// Stop the background sync and remove it.
    Off,
    /// Say whether the background sync runs.
    Status,
}

#[derive(Args, Debug, Clone)]
pub struct ClaudeCacheArgs {
    #[command(subcommand)]
    pub action: ClaudeCacheAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ClaudeCacheAction {
    /// Delete the index; the next ls or stats reads every transcript again.
    Clear(WriteFlags),
    /// Show index size, rows, and when it last changed.
    Stats,
}

#[derive(Args, Debug, Clone)]
pub struct AccountsArgs {
    #[command(subcommand)]
    pub action: AccountsAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AccountsAction {
    /// Show every desktop app account and how many chats it lists. Alias: list.
    #[command(alias = "list")]
    Ls,
    /// Make the chats one account lists show up for another account.
    Cp(AccountCpArgs),
}

#[derive(Args, Debug, Clone)]
pub struct AccountCpArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// The account the chats are listed under now: its id, or the start of it.
    #[arg(value_hint = ValueHint::Other)]
    pub from: Option<String>,
    /// The account that should list them too. Default: the account signed in now.
    #[arg(value_hint = ValueHint::Other)]
    pub to: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct UninstallArgs {
    /// Show the plan and change nothing.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Also delete local state: history, index, and backups.
    #[arg(long)]
    pub purge: bool,
}

#[derive(Args, Debug, Clone)]
pub struct HelpArgs {
    /// Command to describe, e.g. mv or queue add.
    #[arg(value_hint = ValueHint::Other)]
    pub command: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct CommonArgs {
    /// Show the plan and change nothing.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Skip the single warning prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
    /// Limit work to one Cursor installation, or NAME/PROFILE for one VS Code profile in it.
    #[arg(long, value_hint = ValueHint::Other)]
    pub profile: Option<String>,
    /// Rewrite every matching path from FROM to TO.
    #[arg(long, num_args = 2, value_names = ["FROM", "TO"], value_hint = ValueHint::Other)]
    pub replace: Vec<String>,
    /// Treat --replace FROM as a regular expression.
    #[arg(long)]
    pub regex: bool,
    /// Only consider unsaved Workspaces/<ts> sessions.
    #[arg(long)]
    pub unsaved: bool,
    /// Limit work to one tool: cursor or claude. Without it, chatkeep covers every tool it finds.
    #[arg(long, value_enum, value_name = "TOOL", value_hint = ValueHint::Other)]
    pub tool: Option<Tool>,
}

/// The AI coding tools whose chats chatkeep looks after.
#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Cursor,
    Claude,
}

impl Tool {
    /// The value `--tool` takes for it.
    pub fn flag(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Claude => "claude",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::Claude => "Claude Code",
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct PathArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The project: its folder path, or a Cursor workspace hash.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub from: Option<String>,
    /// The new project folder or .code-workspace file.
    #[arg(value_hint = ValueHint::DirPath)]
    pub to: Option<String>,
    /// Also move or copy the real project folder.
    #[arg(long)]
    pub project: bool,
}

#[derive(Args, Debug, Clone)]
pub struct SaveArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The unsaved session: its Workspaces/<ts> number or hash.
    #[arg(value_hint = ValueHint::Other)]
    pub id: Option<String>,
    /// The folder to attach it to.
    #[arg(value_hint = ValueHint::DirPath)]
    pub to: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct SplitArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The project whose chats are split up.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub source: Option<String>,
    /// The project folders that receive the chats.
    #[arg(value_hint = ValueHint::DirPath)]
    pub targets: Vec<String>,
    /// Remove chats from the source after they land on the targets.
    #[arg(long = "move")]
    pub move_chats: bool,
}

#[derive(Args, Debug, Clone)]
pub struct CombineArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The project that receives the chats.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub target: Option<String>,
    /// The projects or single chats the chats come from.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub sources: Vec<String>,
    /// Keep the chats in the sources (default).
    #[arg(long, conflicts_with = "move_chats")]
    pub copy: bool,
    /// Remove chats from the sources.
    #[arg(long = "move")]
    pub move_chats: bool,
}

#[derive(Args, Debug, Clone)]
pub struct TargetArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// A project path or Cursor workspace hash (rm also takes a chat id).
    #[arg(value_hint = ValueHint::AnyPath)]
    pub target: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ListArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Show one project and its chats: its folder path or Cursor workspace hash.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub id: Option<String>,
    /// Read live data instead of the index, then refresh the index.
    #[arg(long)]
    pub fresh: bool,
}

#[derive(Args, Debug, Clone)]
pub struct StatsArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Read live data instead of the index, then refresh the index.
    #[arg(long)]
    pub fresh: bool,
}

#[derive(Args, Debug, Clone)]
pub struct CacheArgs {
    #[command(subcommand)]
    pub action: CacheAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CacheAction {
    /// Delete the index, or with --profile only that installation's rows.
    Clear(CommonArgs),
    /// Refresh the index now, re-reading only what changed.
    Scan(ScanArgs),
    /// Show index size, freshness, and the background refresh.
    Stats(CommonArgs),
}

impl CacheAction {
    fn name(&self) -> &'static str {
        match self {
            Self::Clear(_) => "clear",
            Self::Scan(_) => "scan",
            Self::Stats(_) => "stats",
        }
    }

    fn common(&self) -> &CommonArgs {
        match self {
            Self::Clear(common) | Self::Stats(common) => common,
            Self::Scan(args) => &args.common,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct ScanArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Ignore what is stored and rebuild from scratch.
    #[arg(long)]
    pub full: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The project: its folder path, or a Cursor workspace hash.
    #[arg(value_hint = ValueHint::AnyPath)]
    pub target: Option<String>,
    /// The .chatkeep archive to write.
    #[arg(value_hint = ValueHint::FilePath)]
    pub file: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// The .chatkeep archive to restore.
    #[arg(value_hint = ValueHint::FilePath)]
    pub file: Option<String>,
    /// Restore into this folder instead of the original path.
    #[arg(value_hint = ValueHint::DirPath)]
    pub to: Option<String>,
    /// Replace chats that already exist instead of skipping them.
    #[arg(long)]
    pub overwrite: bool,
}

#[derive(Args, Debug, Clone)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub action: QueueAction,
}

#[derive(Subcommand, Debug, Clone)]
pub enum QueueAction {
    /// Validate and store a write command for later.
    Add(QueueAddArgs),
    /// Show the queue. Alias: ls.
    #[command(alias = "ls")]
    List(QueueListArgs),
    /// Remove entries by id (any status).
    Rm(QueueRmArgs),
    /// Remove every queue entry.
    Clear(QueueClearArgs),
    /// Reset failed or skipped entries to pending.
    Retry(QueueRetryArgs),
    /// Run pending entries once the tools they change are closed.
    Execute(QueueExecuteArgs),
}

#[derive(Args, Debug, Clone)]
pub struct QueueAddArgs {
    /// The write command and its arguments (after --).
    #[arg(
        required = true,
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_hint = ValueHint::Other
    )]
    pub argv: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct QueueListArgs {
    /// Print each command exactly as stored instead of the shortened form.
    #[arg(long)]
    pub full: bool,
}

#[derive(Args, Debug, Clone)]
pub struct WriteFlags {
    /// Show the plan and change nothing.
    #[arg(short = 'n', long)]
    pub dry_run: bool,
    /// Skip the confirmation prompt.
    #[arg(short = 'y', long)]
    pub yes: bool,
}

#[derive(Args, Debug, Clone)]
pub struct QueueRmArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// Queue entry ids, as queue list shows them.
    #[arg(value_hint = ValueHint::Other)]
    pub ids: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct QueueClearArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
}

#[derive(Args, Debug, Clone)]
pub struct QueueRetryArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// Failed or skipped ids to reset. Empty means every failed or skipped entry.
    #[arg(value_hint = ValueHint::Other)]
    pub ids: Vec<String>,
}

#[derive(Args, Debug, Clone)]
pub struct QueueExecuteArgs {
    #[command(flatten)]
    pub flags: WriteFlags,
    /// Keep going after an entry fails.
    #[arg(long)]
    pub continue_on_error: bool,
}

pub fn run() -> i32 {
    ui::signal::install();
    let settings = ui::term::init(ui::term::mode_from_args(std::env::args_os()));
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if let Some(path) = help_request(&args) {
        return match show_help(&path) {
            Ok(()) => 0,
            Err(err) => crate::queue_cmd::report_failure(&err),
        };
    }
    let matches = Cli::command()
        .color(clap_color(settings.stderr))
        .get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|err| err.exit());
    match dispatch(cli, None) {
        Ok(()) => 0,
        Err(err) => crate::queue_cmd::report_failure(&err),
    }
}

fn clap_color(enabled: bool) -> ColorChoice {
    if enabled {
        ColorChoice::Always
    } else {
        ColorChoice::Never
    }
}

/// Where each tool's runtime comes from: the real machine, or one a test built.
#[derive(Clone)]
pub struct Tools {
    cursor: CursorSource,
    claude: Option<claude::Runtime>,
    /// chatkeep's own state folder: history, queue, backups.
    home: PathBuf,
    mode: Option<Mode>,
}

/// How the queue runs a stored command: from the folder it was queued in, without prompts.
#[derive(Debug, Clone)]
pub struct Mode {
    /// Where relative paths resolve, when not the folder chatkeep runs in.
    pub cwd: Option<PathBuf>,
    /// Plan only.
    pub dry_run: bool,
}

#[derive(Clone)]
enum CursorSource {
    System,
    Given(Box<Runtime>),
    Absent,
}

/// How a run talks to the user, whichever tool it ends up using.
#[derive(Debug, Clone)]
pub struct Session {
    pub home: PathBuf,
    pub dry_run: bool,
    pub yes: bool,
    pub quiet: bool,
    pub interactive: bool,
}

impl Tools {
    /// The tools installed on this machine.
    pub fn system() -> Result<Self> {
        let home = config::chatkeep_home()?;
        let layout = claude::Layout::system(home.clone())?;
        let claude = if layout.exists() {
            let mut rt = claude::Runtime::system(layout.chatkeep_home.clone())?;
            rt.layout = layout;
            Some(rt)
        } else {
            None
        };
        Ok(Self {
            cursor: CursorSource::System,
            claude,
            home,
            mode: None,
        })
    }

    pub fn new(cursor: Option<Runtime>, claude: Option<claude::Runtime>) -> Self {
        let home = match (&cursor, &claude) {
            (Some(rt), _) => rt.layout.chatkeep_home.clone(),
            (None, Some(rt)) => rt.layout.chatkeep_home.clone(),
            (None, None) => PathBuf::new(),
        };
        Self {
            cursor: match cursor {
                Some(rt) => CursorSource::Given(Box::new(rt)),
                None => CursorSource::Absent,
            },
            claude,
            home,
            mode: None,
        }
    }

    /// The same tools, running stored commands the way the queue does.
    pub fn in_mode(&self, mode: Mode) -> Self {
        let mut tools = self.clone();
        tools.mode = Some(mode);
        tools
    }

    pub fn has_cursor(&self) -> bool {
        !matches!(self.cursor, CursorSource::Absent)
    }

    pub fn has_claude(&self) -> bool {
        self.claude.is_some()
    }

    /// The Cursor runtime with the shared flags applied, or `None` when Cursor is not here.
    pub fn cursor(&self, common: &CommonArgs) -> Result<Option<Runtime>> {
        let rt = match &self.cursor {
            CursorSource::System => cursor_runtime(common)?,
            CursorSource::Given(rt) => Some(configure((**rt).clone(), common)?),
            CursorSource::Absent => None,
        };
        Ok(rt.map(|mut rt| {
            if let Some(mode) = &self.mode {
                if let Some(cwd) = &mode.cwd {
                    rt.cwd = cwd.clone();
                }
                rt.dry_run |= mode.dry_run;
                rt.yes = true;
                rt.quiet = true;
                rt.interactive = false;
            }
            rt
        }))
    }

    fn require_cursor(&self, common: &CommonArgs) -> Result<Runtime> {
        self.cursor(common)?
            .ok_or_else(|| anyhow::anyhow!("no Cursor installation was found"))
    }

    pub fn claude(&self, common: &CommonArgs) -> Option<claude::Runtime> {
        self.claude.clone().map(|rt| {
            let mut rt = crate::claude_cmd::configure(rt, common);
            if let Some(mode) = &self.mode {
                if let Some(cwd) = &mode.cwd {
                    rt.cwd = cwd.clone();
                }
                rt.dry_run |= mode.dry_run;
                rt.yes = true;
                rt.quiet = true;
                rt.interactive = false;
            }
            rt
        })
    }

    fn require_claude(&self, common: &CommonArgs) -> Result<claude::Runtime> {
        self.claude(common).ok_or_else(|| {
            ui::hinted(
                "no Claude Code data was found",
                "Claude Code keeps it in ~/.claude, or in CLAUDE_CONFIG_DIR when that is set.",
            )
        })
    }

    /// The prompt and output settings of this run. A runtime a test handed in decides them;
    /// on a real machine they come from the terminal.
    pub fn session(&self, dry_run: bool, yes: bool) -> Session {
        let (quiet, interactive, given_yes) = match (&self.cursor, &self.claude) {
            (CursorSource::Given(rt), _) => (rt.quiet, rt.interactive, rt.yes),
            (CursorSource::Absent, Some(rt)) => (rt.quiet, rt.interactive, rt.yes),
            _ => (false, ui::interactive(), false),
        };
        Session {
            home: self.home.clone(),
            dry_run,
            yes: yes || given_yes,
            quiet,
            interactive,
        }
    }
}

pub fn dispatch(cli: Cli, runtime: Option<Runtime>) -> Result<()> {
    let tools = match runtime {
        Some(runtime) => Tools::new(Some(runtime), None),
        None => Tools::system()?,
    };
    dispatch_with(cli, &tools)
}

pub fn dispatch_with(cli: Cli, tools: &Tools) -> Result<()> {
    let command = match cli.command {
        Some(command) if !cli.help => command,
        _ => {
            crate::update::notify_if_outdated();
            ui::print_help();
            return Ok(());
        }
    };
    if matches!(command, Command::RefreshIndex) {
        return match tools.cursor(&common_of(&command))? {
            Some(rt) => index::background(&rt),
            None => Ok(()),
        };
    }
    if let Command::Uninstall(args) = &command {
        return crate::uninstall::run(args);
    }
    if matches!(command, Command::Update) {
        return crate::update::run_update();
    }
    crate::update::notify_if_outdated();
    if matches!(command, Command::Github) {
        return crate::update::open_github();
    }
    if let Command::Help(args) = &command {
        return show_help(&args.command);
    }
    if let Command::Queue(args) = command {
        return crate::queue_cmd::run(tools, args);
    }
    if let Command::History(common) = &command {
        return show_shared_history(tools, common);
    }
    // A command for both tools prints one result per tool, a blank line apart.
    let mut printed = 0usize;
    run_command(tools, command, &mut |name, outcome| {
        if printed > 0 {
            println!();
        }
        printed += 1;
        print_outcome(name, outcome)
    })
    .result
}

fn show_shared_history(tools: &Tools, common: &CommonArgs) -> Result<()> {
    let path = match (tools.cursor(common)?, tools.claude(common)) {
        (Some(rt), _) => rt.layout.history_file(),
        (None, Some(claude)) => claude.layout.history_file(),
        (None, None) => config::chatkeep_home()?.join("history.jsonl"),
    };
    print!("{}", engine::render_history_file(&path, Theme::stdout())?);
    Ok(())
}

/// Which tools a command runs for.
#[allow(clippy::large_enum_variant)]
pub enum Route {
    Cursor(Runtime, Command),
    Claude(claude::Runtime, Command),
    /// Claude Code goes first, so a Cursor that is still open can queue its own part.
    Both {
        cursor: Runtime,
        claude: claude::Runtime,
        cursor_command: Command,
        claude_command: Command,
    },
}

impl Route {
    pub fn uses_cursor(&self) -> bool {
        !matches!(self, Self::Claude(..))
    }

    pub fn uses_claude(&self) -> bool {
        !matches!(self, Self::Cursor(..))
    }
}

/// The tool a command names by itself: its namespace, `--tool`, or a flag only Cursor has.
fn named_tool(command: &Command, common: &CommonArgs) -> Result<Option<Tool>> {
    let implied = match command {
        Command::Cursor(_) => Some(Tool::Cursor),
        Command::Claude(_) => Some(Tool::Claude),
        _ if common.profile.is_some() || common.unsaved => Some(Tool::Cursor),
        _ => None,
    };
    match (implied, common.tool) {
        (Some(implied), Some(tool)) if implied != tool => {
            let why = match command {
                Command::Cursor(_) => "chatkeep cursor commands only work on Cursor",
                Command::Claude(_) => "chatkeep claude commands only work on Claude Code",
                _ if common.unsaved => "--unsaved only exists for Cursor",
                _ => "--profile only exists for Cursor",
            };
            bail!("{why}; drop --tool {}", tool.flag());
        }
        (implied, tool) => Ok(tool.or(implied)),
    }
}

/// Decide which tools `command` runs for. Without `--tool`, a command that names a project
/// runs where that project exists.
pub fn route(tools: &Tools, command: Command) -> Result<Route> {
    let common = common_of(&command);
    match named_tool(&command, &common)? {
        Some(Tool::Cursor) => return Ok(Route::Cursor(tools.require_cursor(&common)?, command)),
        Some(Tool::Claude) => return Ok(Route::Claude(tools.require_claude(&common)?, command)),
        None => {}
    }
    let (rt, claude) = match (tools.cursor(&common)?, tools.claude(&common)) {
        (Some(rt), Some(claude)) => (rt, claude),
        (Some(rt), None) => return Ok(Route::Cursor(rt, command)),
        (None, Some(claude)) => return Ok(Route::Claude(claude, command)),
        (None, None) => {
            return Err(ui::hinted(
                "neither Cursor nor Claude Code was found",
                "chatkeep looks for Cursor's data folder and for ~/.claude.",
            ));
        }
    };
    let cursor_has = |spec: &str| -> Result<bool> { Ok(!engine::locate(&rt, spec)?.is_empty()) };
    // A command typed without its arguments asks for them; which tool it asks for is the
    // first question.
    let ask = |rt: Runtime, claude: claude::Runtime, command: Command| -> Result<Route> {
        if !rt.interactive {
            return Ok(Route::Cursor(rt, command));
        }
        Ok(
            match ui::select(
                "Which tool?",
                &[Tool::Cursor.label(), Tool::Claude.label()],
                rt.interactive,
            )? {
                1 => Route::Claude(claude, command),
                _ => Route::Cursor(rt, command),
            },
        )
    };
    let (in_cursor, in_claude, spec) = match &command {
        Command::Ls(args) => match &args.id {
            Some(id) => (
                cursor_has(id)?,
                crate::claude_cmd::has_project(&claude, id)?,
                id.clone(),
            ),
            None => (true, true, String::new()),
        },
        Command::Stats(_) => (true, true, String::new()),
        Command::Mv(args) | Command::Cp(args) => {
            let replace = replace_pair(&args.common)?;
            if replace.is_none() && (args.from.is_none() || args.to.is_none()) {
                return ask(rt, claude, command);
            }
            let in_cursor = match (&replace, &args.from) {
                (Some((from, to)), _) => {
                    let mut hit = false;
                    for scoped in rt.scope() {
                        hit |= !engine::replaced(&scoped, from, to, args.common.regex)?.is_empty();
                    }
                    hit
                }
                (None, Some(from)) => cursor_has(from)?,
                (None, None) => false,
            };
            let spec = match &replace {
                Some((from, _)) => format!("--replace {from}"),
                None => args.from.clone().unwrap_or_default(),
            };
            (
                in_cursor,
                crate::claude_cmd::move_matches(&claude, args)?,
                spec,
            )
        }
        Command::Rm(args) => match &args.target {
            Some(target) => (
                cursor_has(target)?,
                crate::claude_cmd::has_target(&claude, target)?,
                target.clone(),
            ),
            None => return ask(rt, claude, command),
        },
        Command::Split(args) => match &args.source {
            Some(source) => (
                cursor_has(source)?,
                crate::claude_cmd::has_project(&claude, source)?,
                source.clone(),
            ),
            None => return ask(rt, claude, command),
        },
        Command::Export(args) => match &args.target {
            Some(target) => {
                let found = (
                    cursor_has(target)?,
                    crate::claude_cmd::has_project(&claude, target)?,
                );
                if found == (true, true) {
                    return Err(ui::hinted(
                        format!("{target} has chats in both Cursor and Claude Code"),
                        "One archive holds one tool. Run export once with --tool cursor and once with --tool claude.",
                    ));
                }
                (found.0, found.1, target.clone())
            }
            None => return ask(rt, claude, command),
        },
        Command::Import(args) => match &args.file {
            Some(file) => {
                let tool = engine::archive_tool(&rt.resolve_path(file))?;
                (
                    tool == engine::archive::CURSOR,
                    tool == engine::archive::CLAUDE,
                    file.clone(),
                )
            }
            None => return ask(rt, claude, command),
        },
        Command::Combine(args) => {
            if args.target.is_none() || args.sources.is_empty() {
                return ask(rt, claude, command);
            }
            let mut for_cursor = Vec::new();
            let mut for_claude = Vec::new();
            for source in &args.sources {
                let known = (
                    cursor_has(source)?,
                    crate::claude_cmd::has_source(&claude, source)?,
                );
                if known == (false, false) {
                    bail!("no Cursor workspace, Claude Code project, or chat matches {source}");
                }
                if known.0 {
                    for_cursor.push(source.clone());
                }
                if known.1 {
                    for_claude.push(source.clone());
                }
            }
            let narrowed = |sources: Vec<String>| {
                let mut args = args.clone();
                args.sources = sources;
                Command::Combine(args)
            };
            return Ok(match (for_cursor.is_empty(), for_claude.is_empty()) {
                (false, true) => Route::Cursor(rt, command),
                (true, false) => Route::Claude(claude, command),
                _ => Route::Both {
                    cursor: rt,
                    claude,
                    cursor_command: narrowed(for_cursor),
                    claude_command: narrowed(for_claude),
                },
            });
        }
        // `history` and `queue` never get here; the namespaces were routed above.
        _ => return Ok(Route::Cursor(rt, command)),
    };
    match (in_cursor, in_claude) {
        (false, false) if matches!(command, Command::Import(_)) => {
            bail!("{spec} holds chats of a tool chatkeep does not know")
        }
        (false, false) => bail!("no Cursor workspace or Claude Code project matches {spec}"),
        (true, false) => Ok(Route::Cursor(rt, command)),
        (false, true) => Ok(Route::Claude(claude, command)),
        (true, true) => Ok(Route::Both {
            cursor: rt,
            claude,
            cursor_command: command.clone(),
            claude_command: command,
        }),
    }
}

/// Plan a route without changing anything: its warnings, and whether the real run would have
/// to wait for a tool to be closed. The runtimes of the route must already be in dry-run mode.
pub fn plan_route(route: Route) -> (Result<Vec<String>>, bool) {
    fn warnings(outcome: Outcome) -> Vec<String> {
        match outcome {
            Outcome::Finished { report, .. } => report.warnings,
            Outcome::Text { .. } | Outcome::Standalone => Vec::new(),
        }
    }
    let cursor_plan = |mut rt: Runtime, command: Command| {
        rt.reset_guard_reached();
        let planned = perform(&rt, command).map(warnings);
        (planned, rt.guard_was_reached())
    };
    let claude_plan = |rt: claude::Runtime, command: Command| {
        let waits = crate::claude_cmd::is_write(&command);
        (
            crate::claude_cmd::perform(&rt, command).map(warnings),
            waits,
        )
    };
    match route {
        Route::Cursor(rt, command) => cursor_plan(rt, command),
        Route::Claude(rt, command) => claude_plan(rt, command),
        Route::Both {
            cursor,
            claude,
            cursor_command,
            claude_command,
        } => {
            let (first, claude_waits) = claude_plan(claude, claude_command);
            let (second, cursor_waits) = cursor_plan(cursor, cursor_command);
            let planned = first.and_then(|mut found| {
                found.extend(second?);
                Ok(found)
            });
            (planned, claude_waits || cursor_waits)
        }
    }
}

/// How a command ended, and whether Claude Code's half of it is already done. A command that
/// failed in its Cursor half must not run its Claude Code half again.
pub struct Ran {
    pub result: Result<()>,
    pub claude_done: bool,
}

/// Run `command` for the tools it belongs to, handing each result to `sink` as it is ready.
pub fn run_command(
    tools: &Tools,
    command: Command,
    sink: &mut dyn FnMut(&'static str, Outcome) -> Result<()>,
) -> Ran {
    let route = match route(tools, command) {
        Ok(route) => route,
        Err(err) => {
            return Ran {
                result: Err(err),
                claude_done: false,
            };
        }
    };
    run_route(route, sink)
}

pub fn run_route(route: Route, sink: &mut dyn FnMut(&'static str, Outcome) -> Result<()>) -> Ran {
    match route {
        Route::Cursor(rt, command) => {
            let name = command_name(&command);
            Ran {
                result: perform_recorded(&rt, command).and_then(|outcome| sink(name, outcome)),
                claude_done: false,
            }
        }
        Route::Claude(rt, command) => {
            let name = command_name(&command);
            let result = crate::claude_cmd::perform_recorded(&rt, command)
                .and_then(|outcome| sink(name, outcome));
            Ran {
                claude_done: result.is_ok(),
                result,
            }
        }
        Route::Both {
            cursor,
            claude,
            cursor_command,
            claude_command,
        } => {
            let started = Instant::now();
            let name = command_name(&cursor_command);
            let args = command_args(&cursor_command);
            let first = crate::claude_cmd::perform(&claude, claude_command)
                .and_then(|outcome| sink(name, outcome));
            let claude_done = first.is_ok();
            let result = first.and_then(|()| {
                perform(&cursor, cursor_command)
                    .map_err(after_claude)
                    .and_then(|outcome| sink(name, outcome))
            });
            if !cursor.dry_run {
                let outcome = if result.is_ok() { "ok" } else { "error" };
                record_history(&cursor, name, &args, outcome, started);
            }
            Ran {
                result,
                claude_done,
            }
        }
    }
}

/// Claude Code's half already ran, so only Cursor's half is left: an open Cursor queues just
/// that, and any other failure says so.
fn after_claude(mut err: anyhow::Error) -> anyhow::Error {
    if let Some(running) = err.downcast_mut::<engine::CursorRunning>() {
        running.queue_args = vec!["--tool".to_string(), "cursor".to_string()];
        return err;
    }
    ui::hinted(
        format!("{err:#}"),
        "The Claude Code part of this command is already done. Once this is fixed, run it again with --tool cursor.",
    )
}

fn perform_recorded(rt: &Runtime, command: Command) -> Result<Outcome> {
    let started = Instant::now();
    let name = command_name(&command);
    let args = command_args(&command);
    let result = perform(rt, command);
    let outcome = if result.is_ok() { "ok" } else { "error" };
    record_history(rt, name, &args, outcome, started);
    result
}

fn run_recorded(rt: &Runtime, command: Command) -> Result<()> {
    let name = command_name(&command);
    print_outcome(name, perform_recorded(rt, command)?)
}

/// What a command produced, before anything is printed.
pub enum Outcome {
    Finished {
        dry_run: bool,
        title: &'static str,
        report: engine::Report,
    },
    Text {
        text: String,
        note: Option<index::Note>,
    },
    /// Needs no chat data; `dispatch` runs it before a runtime exists.
    Standalone,
}

/// Prints what `name` produced, exactly as a direct run does.
pub fn print_outcome(name: &str, outcome: Outcome) -> Result<()> {
    match outcome {
        Outcome::Finished {
            dry_run,
            title,
            report,
        } => print_report(title, dry_run, &report),
        Outcome::Text { text, note } => {
            print!("{text}");
            print_note(note);
        }
        Outcome::Standalone => bail!("{name} cannot run from a queue entry"),
    }
    Ok(())
}

/// `command` exactly as a direct run performs it, without printing its result.
pub fn plan(rt: &Runtime, command: Command) -> Result<Outcome> {
    let rt = configure(rt.clone(), &common_of(&command))?;
    perform(&rt, command)
}

fn perform(rt: &Runtime, command: Command) -> Result<Outcome> {
    let finished = |rt: &Runtime, title, report| Outcome::Finished {
        dry_run: rt.dry_run,
        title,
        report,
    };
    Ok(match command {
        Command::Mv(args) => run_path(rt, args, false)?,
        Command::Cp(args) => run_path(rt, args, true)?,
        Command::Split(args) => run_split(rt, args)?,
        Command::Combine(args) => run_combine(rt, args)?,
        Command::Cursor(args) => match args.command {
            CursorCommand::Save(args) => run_save(rt, args)?,
            CursorCommand::Rx(args) => {
                let (rt, target) = one_target(rt, args.target, "Reindex which workspace?")?;
                let report = reindex(&rt, &target)?;
                finished(&rt, "Reindex", report)
            }
            CursorCommand::Cache(args) => run_cache(rt, &args.action)?,
        },
        Command::Claude(_) => bail!("chatkeep claude commands only work on Claude Code"),
        Command::Ls(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            let (text, note) = engine::render_workspaces_noted(
                &rt,
                args.common.unsaved,
                args.id.as_deref(),
                Theme::stdout(),
            )?;
            Outcome::Text { text, note }
        }
        Command::Rm(args) => {
            let (rt, targets) = match args.target {
                Some(target) => (resolve_install(rt, &[target.as_str()])?, vec![target]),
                None => pick_many(rt, "Remove which workspaces?")?,
            };
            let rt = &rt;
            if !rt.dry_run && !rt.yes {
                let rows: Vec<Vec<String>> = targets
                    .iter()
                    .map(|target| vec![target.clone(), removal_label(rt, target)])
                    .collect();
                print!(
                    "{}",
                    validation(
                        Theme::stdout(),
                        "Remove plan",
                        &["target", "location"],
                        &rows,
                        &[]
                    )
                );
            }
            if !rt.dry_run
                && !ui::confirm(
                    "Remove the selected Cursor metadata?",
                    rt.yes,
                    rt.interactive,
                )?
            {
                bail!("aborted");
            }
            let report = remove_targets(rt, &targets)?;
            finished(rt, "Remove", report)
        }
        Command::Export(args) => {
            let (rt, target) = one_target(rt, args.target, "Export which workspace?")?;
            let file = match args.file {
                Some(file) => file,
                None if rt.interactive => ui::input("Archive path", true)?,
                None => bail!("export needs an archive path"),
            };
            let report = export_workspace(&rt, &target, PathBuf::from(file).as_path())?;
            finished(&rt, "Export", report)
        }
        Command::Import(args) => {
            let file = match args.file {
                Some(file) => PathBuf::from(file),
                None if rt.interactive => PathBuf::from(ui::input("Archive path", true)?),
                None => bail!("import needs an archive file"),
            };
            let rt = import_install(rt, args.to.as_deref())?;
            let dest = args.to.map(PathBuf::from);
            let report = import_archive(&rt, &file, dest.as_deref(), args.overwrite)?;
            finished(&rt, "Import", report)
        }
        Command::History(_) => Outcome::Text {
            text: show_history(rt)?,
            note: None,
        },
        Command::Stats(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            let (text, note) = engine::render_stats_noted(&rt, Theme::stdout())?;
            Outcome::Text { text, note }
        }
        Command::Queue(_) => bail!("cannot nest queue commands"),
        Command::RefreshIndex
        | Command::Update
        | Command::Uninstall(_)
        | Command::Github
        | Command::Help(_) => Outcome::Standalone,
    })
}

pub fn run_parsed(rt: &Runtime, command: Command) -> Result<()> {
    let rt = configure(rt.clone(), &common_of(&command))?;
    run_recorded(&rt, command)
}

/// The dry-run and confirmation flags of a queue action.
pub fn queue_flags(action: &QueueAction) -> (bool, bool) {
    match action {
        QueueAction::Add(_) | QueueAction::List(_) => (false, true),
        QueueAction::Rm(QueueRmArgs { flags, .. })
        | QueueAction::Clear(QueueClearArgs { flags })
        | QueueAction::Retry(QueueRetryArgs { flags, .. })
        | QueueAction::Execute(QueueExecuteArgs { flags, .. }) => (flags.dry_run, flags.yes),
    }
}

fn queue_common(action: &QueueAction) -> CommonArgs {
    let (dry_run, yes) = queue_flags(action);
    CommonArgs {
        dry_run,
        yes,
        ..unprompted()
    }
}

pub fn unprompted() -> CommonArgs {
    CommonArgs {
        dry_run: false,
        yes: true,
        profile: None,
        replace: Vec::new(),
        regex: false,
        unsaved: false,
        tool: None,
    }
}

fn print_note(note: Option<index::Note>) {
    if let Some(note) = note {
        let theme = Theme::stderr();
        eprintln!("{}", ui::hint_line(theme, &note.text(theme)));
    }
}

fn run_cache(rt: &Runtime, action: &CacheAction) -> Result<Outcome> {
    let text = match action {
        CacheAction::Clear(_) => {
            let plan = cache::clear_plan(rt)?;
            if !plan.items.is_empty()
                && !rt.dry_run
                && !ui::confirm(&plan.question, rt.yes, rt.interactive)?
            {
                bail!("aborted");
            }
            return Ok(Outcome::Finished {
                dry_run: rt.dry_run,
                title: "Cache clear",
                report: cache::clear(rt, &plan)?,
            });
        }
        CacheAction::Scan(_) if rt.dry_run => {
            cache::render_stale(Theme::stdout(), &cache::collect(rt)?)
        }
        CacheAction::Scan(args) => {
            cache::render_scan(Theme::stdout(), &cache::scan(rt, args.full)?)
        }
        CacheAction::Stats(_) => {
            cache::render_stats(Theme::stdout(), &cache::collect(rt)?, index::now_ms())
        }
    };
    Ok(Outcome::Text { text, note: None })
}

pub fn show_help(path: &[String]) -> Result<()> {
    if path.is_empty() {
        ui::print_help();
        return Ok(());
    }
    let mut root = Cli::command();
    root.build();
    let mut names: Vec<&str> = Vec::new();
    let mut current = &root;
    for name in path {
        let Some(sub) = current.get_subcommands().find(|sub| {
            !sub.is_hide_set()
                && (sub.get_name() == name || sub.get_all_aliases().any(|alias| alias == name))
        }) else {
            return Err(ui::hinted(
                format!("unknown command: {}", path.join(" ")),
                "Run chatkeep help to see every command.",
            ));
        };
        names.push(sub.get_name());
        current = sub;
    }
    print!(
        "{}",
        ui::command_help(Theme::stdout(), current, &names, ui::term::width())
    );
    Ok(())
}

/// The command path a `-h`/`--help` anywhere before `--` asks about, or a command that only
/// groups actions (`queue`, `cache`) given without one.
pub fn help_request(args: &[String]) -> Option<Vec<String>> {
    let root = Cli::command();
    let mut path = Vec::new();
    let mut current = &root;
    let mut wants = false;
    let mut others = 0usize;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                others += 1;
                break;
            }
            "-h" | "--help" => wants = true,
            "--color" => {
                args.next();
            }
            flag if flag.starts_with('-') => others += 1,
            name => {
                let sub = current.get_subcommands().find(|sub| {
                    sub.get_name() == name || sub.get_all_aliases().any(|alias| alias == name)
                });
                match sub {
                    Some(sub) if sub.get_name() == "help" && path.is_empty() => return None,
                    Some(sub) if others == 0 => {
                        path.push(sub.get_name().to_string());
                        current = sub;
                    }
                    _ => others += 1,
                }
            }
        }
    }
    // A command that works without an action (`claude sync`) runs; it is not a bare group.
    let bare_group = !path.is_empty() && current.is_subcommand_required_set() && others == 0;
    ((wants && !path.is_empty()) || bare_group).then_some(path)
}

fn removal_label(rt: &Runtime, target: &str) -> String {
    match engine::find_workspace(rt, target) {
        Ok(Some(workspace)) => workspace
            .path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| workspace.kind.label().to_string()),
        _ => "chat".to_string(),
    }
}

fn run_path(rt: &Runtime, args: PathArgs, copy: bool) -> Result<Outcome> {
    let replace = replace_pair(&args.common)?;
    let (rt, pairs) = match (&args.from, &args.to, &replace) {
        (Some(from), Some(to), None) => (
            resolve_install(rt, &[from.as_str()])?,
            vec![(from.clone(), to.clone())],
        ),
        (_, _, Some((from, to))) => (
            replace_install(rt, from, to, args.common.regex)?,
            Vec::new(),
        ),
        _ if !rt.interactive => bail!(
            "{} needs FROM and TO, or --replace FROM TO",
            if copy { "cp" } else { "mv" }
        ),
        _ => pick_path_pairs(rt, copy)?,
    };
    let rt = &rt;
    let report = if copy {
        copy_paths(
            rt,
            &pairs,
            replace
                .as_ref()
                .map(|(from, to)| (from.as_str(), to.as_str())),
            args.common.regex,
            args.project,
        )?
    } else {
        move_paths(
            rt,
            &pairs,
            replace
                .as_ref()
                .map(|(from, to)| (from.as_str(), to.as_str())),
            args.common.regex,
            args.project,
        )?
    };
    Ok(Outcome::Finished {
        dry_run: rt.dry_run,
        title: if copy { "Copy" } else { "Move" },
        report,
    })
}

fn run_save(rt: &Runtime, args: SaveArgs) -> Result<Outcome> {
    let (rt, id) = match args.id {
        Some(id) => (resolve_install(rt, &[id.as_str()])?, id),
        None => pick_unsaved(rt)?,
    };
    let to = match args.to {
        Some(to) => PathBuf::from(to),
        None if rt.interactive => PathBuf::from(ui::input("Destination folder", true)?),
        None => bail!("save needs a destination folder"),
    };
    let report = save_unsaved(&rt, &id, &to)?;
    Ok(Outcome::Finished {
        dry_run: rt.dry_run,
        title: "Save",
        report,
    })
}

fn run_split(rt: &Runtime, args: SplitArgs) -> Result<Outcome> {
    let (rt, source) = one_target(rt, args.source, "Split which workspace?")?;
    let rt = &rt;
    let targets = split_targets(args.targets, rt.interactive)?;
    let target_paths: Vec<PathBuf> = targets.iter().map(PathBuf::from).collect();
    let workspace = engine::find_workspace(rt, &source)?
        .ok_or_else(|| anyhow::anyhow!("no workspace matches {source}"))?;
    let suggestion = suggest_split(rt, &workspace, &target_paths)?;
    let assigned = split_assignments(rt.interactive, &suggestion, &target_paths)?;
    confirm_split(rt.quiet, rt.dry_run, rt.yes, rt.interactive, &assigned)?;
    let report = split_workspace(rt, &source, &target_paths, &assigned, args.move_chats)?;
    Ok(Outcome::Finished {
        dry_run: rt.dry_run,
        title: "Split",
        report,
    })
}

/// The target folders `split` got, or the ones the user types when it got none.
pub fn split_targets(given: Vec<String>, interactive: bool) -> Result<Vec<String>> {
    if !given.is_empty() {
        return Ok(given);
    }
    Ok(
        ui::input("Target folders, separated by commas", interactive)?
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect(),
    )
}

/// Which targets each chat goes to: the suggestion, every target, or what the user picks.
/// A chat that matches no target is asked about, or stops a run that cannot ask.
pub fn split_assignments(
    interactive: bool,
    suggestion: &engine::SplitSuggestion,
    target_paths: &[PathBuf],
) -> Result<std::collections::BTreeMap<String, Vec<PathBuf>>> {
    let mode = if interactive && ui::stdout_is_tty() {
        match ui::select(
            "Assignment",
            &["Auto-suggest", "All to all", "Manual"],
            interactive,
        )? {
            1 => "all",
            2 => "manual",
            _ => "auto",
        }
    } else {
        "auto"
    };
    let mut assigned = suggestion.assigned.clone();
    if mode == "all" {
        for id in suggestion.titles.keys() {
            assigned.insert(id.clone(), target_paths.to_vec());
        }
    }
    if mode == "manual" || !suggestion.unassigned.is_empty() {
        for id in suggestion.unassigned.iter().chain(
            if mode == "manual" {
                suggestion.titles.keys().cloned().collect::<Vec<_>>()
            } else {
                Vec::new()
            }
            .iter(),
        ) {
            if assigned.contains_key(id) && mode != "manual" {
                continue;
            }
            if !interactive {
                return Err(ui::hinted(
                    format!(
                        "{} match no target folder",
                        ui::plural(suggestion.unassigned.len(), "chat", "chats")
                    ),
                    "Run split in a terminal to assign them, or pass targets that match.",
                ));
            }
            let total = ui::term::stderr_width();
            let labels: Vec<String> = target_paths
                .iter()
                .map(|path| {
                    ui::picker_label(
                        "",
                        &ui::home_relative(&path.display().to_string()),
                        total,
                        Theme::stderr().icons().ellipsis,
                    )
                })
                .collect();
            let picked = ui::multi_select(&format!("Targets for {id}"), &labels, interactive)?;
            let chosen = picked
                .into_iter()
                .map(|index| target_paths[index].clone())
                .collect();
            assigned.insert(id.clone(), chosen);
        }
    }
    Ok(assigned)
}

/// Show which chat goes where and ask before a split changes anything.
pub fn confirm_split(
    quiet: bool,
    dry_run: bool,
    yes: bool,
    interactive: bool,
    assigned: &std::collections::BTreeMap<String, Vec<PathBuf>>,
) -> Result<()> {
    let rows: Vec<Vec<String>> = assigned
        .iter()
        .map(|(id, dests)| {
            vec![
                id.clone(),
                dests
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            ]
        })
        .collect();
    if !quiet {
        print!(
            "{}",
            validation(
                Theme::stdout(),
                "Split plan",
                &["chat", "targets"],
                &rows,
                &[]
            )
        );
    }
    if !dry_run && !ui::confirm("Continue with this split?", yes, interactive)? {
        bail!("aborted");
    }
    Ok(())
}

fn run_combine(rt: &Runtime, args: CombineArgs) -> Result<Outcome> {
    let target = args
        .target
        .or_else(|| ui::input("Target folder or workspace", rt.interactive).ok())
        .ok_or_else(|| anyhow::anyhow!("a target is required"))?;
    let (rt, sources) = if args.sources.is_empty() {
        pick_many(rt, "Combine which sources?")?
    } else {
        let specs: Vec<&str> = args.sources.iter().map(String::as_str).collect();
        (resolve_install(rt, &specs)?, args.sources)
    };
    let rt = &rt;
    let rows = sources
        .iter()
        .map(|source| vec![source.clone(), target.clone()])
        .collect::<Vec<_>>();
    if !rt.quiet {
        print!(
            "{}",
            validation(
                Theme::stdout(),
                "Combine plan",
                &["source", "target"],
                &rows,
                &[]
            )
        );
    }
    if !rt.dry_run && !ui::confirm("Continue with this combine?", rt.yes, rt.interactive)? {
        bail!("aborted");
    }
    let report = combine_workspaces(
        rt,
        PathBuf::from(&target).as_path(),
        &sources,
        args.move_chats,
    )?;
    Ok(Outcome::Finished {
        dry_run: rt.dry_run,
        title: "Combine",
        report,
    })
}

fn pick_path_pairs(rt: &Runtime, copy: bool) -> Result<(Runtime, Vec<(String, String)>)> {
    let (rt, ids) = pick_many(
        rt,
        if copy {
            "Copy which workspaces?"
        } else {
            "Move which workspaces?"
        },
    )?;
    let rt = &rt;
    let mut pairs = Vec::new();
    if ids.len() == 1 {
        let dest = ui::input("Destination path", rt.interactive)?;
        pairs.push((ids[0].clone(), dest));
    } else {
        let from = ui::input("Replace from", rt.interactive)?;
        let to = ui::input("Replace to", rt.interactive)?;
        for id in &ids {
            if let Some(workspace) = engine::find_workspace(rt, id)?
                && let Some(path) = workspace.path
            {
                let updated = path.to_string_lossy().replace(&from, &to);
                pairs.push((id.clone(), updated));
            }
        }
    }
    let rows = pairs
        .iter()
        .map(|(from, to)| vec![from.clone(), to.clone()])
        .collect::<Vec<_>>();
    print!(
        "{}",
        validation(
            Theme::stdout(),
            if copy { "Copy plan" } else { "Move plan" },
            &["workspace", "destination"],
            &rows,
            &[]
        )
    );
    if !rt.dry_run
        && !ui::confirm(
            if copy {
                "Continue with this copy?"
            } else {
                "Continue with this move?"
            },
            rt.yes,
            rt.interactive,
        )?
    {
        bail!("aborted");
    }
    Ok((rt.clone(), pairs))
}

fn pin(rt: &Runtime) -> Runtime {
    let mut rt = rt.clone();
    rt.pinned = true;
    rt
}

fn mixed(first: (&str, &str), second: (&str, &str)) -> anyhow::Error {
    ui::hinted(
        format!(
            "{} is in profile {}, but {} is in profile {}",
            first.0, first.1, second.0, second.1
        ),
        "chatkeep never mixes Cursor installations. Run the command once per profile with --profile NAME.",
    )
}

fn ask_install(rt: &Runtime, names: &[String], why: &str) -> Result<Runtime> {
    let name = if names.is_empty() {
        bail!("{why}: no Cursor installation was found");
    } else if names.len() == 1 {
        names[0].clone()
    } else if !rt.interactive {
        return Err(ui::hinted(
            why.to_string(),
            format!("Pass --profile with one of: {}.", names.join(", ")),
        ));
    } else {
        let items: Vec<&str> = names.iter().map(String::as_str).collect();
        names[ui::select(
            &format!("{why}. Which Cursor profile?"),
            &items,
            rt.interactive,
        )?]
        .clone()
    };
    let layout = rt
        .installs
        .iter()
        .find(|layout| layout.name == name)
        .ok_or_else(|| anyhow::anyhow!("unknown Cursor profile {name}"))?;
    Ok(pin(&rt.scoped(layout)))
}

/// The one installation that holds every spec, asking when several do.
fn resolve_install(rt: &Runtime, specs: &[&str]) -> Result<Runtime> {
    if rt.installs.len() <= 1 {
        return Ok(rt.clone());
    }
    if rt.pinned {
        for spec in specs {
            if !engine::locate(rt, spec)?.is_empty() {
                continue;
            }
            let mut owners = Vec::new();
            for sibling in rt.siblings() {
                if !engine::locate(&sibling, spec)?.is_empty() {
                    owners.push(sibling.layout.name);
                }
            }
            if !owners.is_empty() {
                return Err(ui::hinted(
                    format!(
                        "{spec} belongs to profile {}, not {}",
                        owners.join(", "),
                        rt.layout.name
                    ),
                    "chatkeep never moves data between Cursor installations. Pass the profile that holds it.",
                ));
            }
        }
        return Ok(rt.clone());
    }
    let mut shared: Option<BTreeSet<String>> = None;
    let mut seen: Vec<(&str, BTreeSet<String>)> = Vec::new();
    for spec in specs {
        let names: BTreeSet<String> = engine::locate(rt, spec)?
            .into_iter()
            .map(|layout| layout.name)
            .collect();
        if names.is_empty() {
            continue;
        }
        let next: BTreeSet<String> = match &shared {
            Some(prev) => prev.intersection(&names).cloned().collect(),
            None => names.clone(),
        };
        if next.is_empty()
            && let Some((first, owners)) = seen.first()
        {
            let join = |set: &BTreeSet<String>| set.iter().cloned().collect::<Vec<_>>().join(", ");
            return Err(mixed((first, &join(owners)), (spec, &join(&names))));
        }
        seen.push((spec, names));
        shared = Some(next);
    }
    let Some(shared) = shared else {
        return Ok(rt.clone());
    };
    let names: Vec<String> = shared.into_iter().collect();
    ask_install(
        rt,
        &names,
        &format!(
            "{} exists in profiles {}",
            specs.join(", "),
            names.join(", ")
        ),
    )
}

fn replace_install(rt: &Runtime, from: &str, to: &str, regex: bool) -> Result<Runtime> {
    if rt.pinned || rt.installs.len() <= 1 {
        return Ok(rt.clone());
    }
    let mut names = Vec::new();
    for scoped in rt.scope() {
        if !engine::replaced(&scoped, from, to, regex)?.is_empty() {
            names.push(scoped.layout.name);
        }
    }
    if names.is_empty() {
        return Ok(rt.clone());
    }
    ask_install(
        rt,
        &names,
        &format!(
            "--replace {from} matches workspaces in profiles {}",
            names.join(", ")
        ),
    )
}

fn import_install(rt: &Runtime, to: Option<&str>) -> Result<Runtime> {
    if rt.pinned || rt.installs.len() <= 1 {
        return Ok(rt.clone());
    }
    if let Some(to) = to {
        let names: Vec<String> = engine::locate(rt, to)?
            .into_iter()
            .map(|layout| layout.name)
            .collect();
        if !names.is_empty() {
            return ask_install(
                rt,
                &names,
                &format!("{to} exists in profiles {}", names.join(", ")),
            );
        }
    }
    let names: Vec<String> = rt
        .installs
        .iter()
        .map(|layout| layout.name.clone())
        .collect();
    ask_install(
        rt,
        &names,
        "import needs one Cursor installation to write into",
    )
}

fn pick_from(
    rt: &Runtime,
    prompt: &str,
    keep: impl Fn(&Workspace) -> bool,
) -> Result<(Runtime, Vec<String>)> {
    let scopes = rt.scope();
    let labelled = scopes.len() > 1;
    let mut found: Vec<(Runtime, Workspace)> = Vec::new();
    for scoped in scopes {
        for workspace in engine::picker_workspaces(&scoped)? {
            let wanted = scoped
                .profile
                .as_ref()
                .is_none_or(|profile| &workspace.profile == profile);
            if wanted && keep(&workspace) {
                found.push((scoped.clone(), workspace));
            }
        }
    }
    let theme = Theme::stderr();
    let total = ui::term::stderr_width();
    let labels: Vec<String> = found
        .iter()
        .map(|(_, workspace)| {
            let location = workspace
                .path
                .as_ref()
                .map(|path| ui::home_relative(&path.display().to_string()))
                .unwrap_or_else(|| workspace.kind.label().to_string());
            let mut lead = engine::short_hash(&workspace.id).to_string();
            if labelled {
                lead.push_str("  ");
                lead.push_str(&workspace.profile_label());
            }
            lead.push_str("  ");
            if workspace.destination_missing {
                lead.push_str(theme.icons().cross);
                lead.push(' ');
            }
            ui::picker_label(&lead, &location, total, theme.icons().ellipsis)
        })
        .collect();
    let picked = ui::multi_select(prompt, &labels, rt.interactive)?;
    let mut chosen: Option<(Runtime, String)> = None;
    let mut ids = Vec::new();
    for index in picked {
        let (scoped, workspace) = &found[index];
        match &chosen {
            Some((existing, first)) if existing.layout.name != scoped.layout.name => {
                return Err(mixed(
                    (first, &existing.layout.name),
                    (&workspace.id, &scoped.layout.name),
                ));
            }
            Some(_) => {}
            None => chosen = Some((pin(scoped), workspace.id.clone())),
        }
        ids.push(workspace.id.clone());
    }
    Ok((chosen.map_or_else(|| rt.clone(), |(scoped, _)| scoped), ids))
}

fn pick_many(rt: &Runtime, prompt: &str) -> Result<(Runtime, Vec<String>)> {
    pick_from(rt, prompt, |_| true)
}

fn one_target(rt: &Runtime, target: Option<String>, prompt: &str) -> Result<(Runtime, String)> {
    match target {
        Some(target) => Ok((resolve_install(rt, &[target.as_str()])?, target)),
        None => {
            let (rt, mut ids) = pick_many(rt, prompt)?;
            let target = ids
                .pop()
                .ok_or_else(|| anyhow::anyhow!("a target is required"))?;
            Ok((rt, target))
        }
    }
}

fn pick_unsaved(rt: &Runtime) -> Result<(Runtime, String)> {
    let (rt, ids) = pick_from(rt, "Unsaved workspace", |workspace| {
        workspace.kind == engine::Kind::Unsaved
    })?;
    let id = ids
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("an unsaved workspace id is required"))?;
    Ok((rt, id))
}

pub fn replace_pair(common: &CommonArgs) -> Result<Option<(String, String)>> {
    if common.replace.is_empty() {
        return Ok(None);
    }
    if common.replace.len() != 2 {
        bail!("--replace needs FROM and TO");
    }
    Ok(Some((common.replace[0].clone(), common.replace[1].clone())))
}

/// A write command's result: what it did, its warnings, and the summary line.
pub fn print_report(title: &str, dry_run: bool, report: &engine::Report) {
    print!("{}", finish_text(Theme::stdout(), title, dry_run, report));
    let theme = Theme::stderr();
    for warning in &report.warnings {
        eprintln!("{}", ui::warn_line(theme, warning));
    }
    print!("{}", finish_summary(Theme::stdout(), dry_run, report));
}

pub fn finish_text(theme: Theme, title: &str, dry_run: bool, report: &engine::Report) -> String {
    let heading = if dry_run {
        format!("{title} (dry run)")
    } else {
        title.to_string()
    };
    let mut out = format!("{}\n", ui::section_line(theme, &heading));
    if !report.applied.is_empty() || !report.skipped.is_empty() {
        let sheet = ui::results(
            theme,
            &title.to_lowercase(),
            dry_run,
            &report.applied,
            &report.skipped,
        );
        out.push_str(&format!("{sheet}\n"));
    }
    if !report.rewritten_keys.is_empty() {
        out.push_str(&ui::info_line(
            theme,
            &format!(
                "rewrote {} storage keys",
                theme.number(ui::count(report.rewritten_keys.len() as u64))
            ),
        ));
        out.push('\n');
    }
    out
}

pub fn finish_summary(theme: Theme, dry_run: bool, report: &engine::Report) -> String {
    if report.applied.is_empty() && report.skipped.is_empty() {
        return format!("{}\n", ui::info_line(theme, "Nothing to do."));
    }
    let applied = report.applied.len();
    let mut parts = vec![theme.good(if dry_run {
        ui::plural(applied, "item planned", "items planned")
    } else {
        ui::plural(applied, "item applied", "items applied")
    })];
    if !report.skipped.is_empty() {
        parts.push(theme.caution(ui::plural(
            report.skipped.len(),
            "item skipped",
            "items skipped",
        )));
    }
    if !report.warnings.is_empty() {
        parts.push(theme.caution(ui::plural(report.warnings.len(), "warning", "warnings")));
    }
    let mut out = ui::summary(theme, &parts);
    if dry_run {
        out.push_str(&ui::hint_line(
            theme,
            "Nothing was changed. Run again without -n to apply.",
        ));
        out.push('\n');
    }
    out
}

fn flags_only(dry_run: bool, yes: bool) -> CommonArgs {
    CommonArgs {
        dry_run,
        yes,
        profile: None,
        replace: Vec::new(),
        regex: false,
        unsaved: false,
        tool: None,
    }
}

pub fn common_of(command: &Command) -> CommonArgs {
    match command {
        Command::Mv(args) | Command::Cp(args) => args.common.clone(),
        Command::Split(args) => args.common.clone(),
        Command::Combine(args) => args.common.clone(),
        Command::Rm(args) => args.common.clone(),
        Command::Ls(args) => args.common.clone(),
        Command::Export(args) => args.common.clone(),
        Command::Import(args) => args.common.clone(),
        Command::History(args) => args.clone(),
        Command::Stats(args) => args.common.clone(),
        Command::Queue(args) => queue_common(&args.action),
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(args) => args.common.clone(),
            CursorCommand::Rx(args) => args.common.clone(),
            CursorCommand::Cache(args) => args.action.common().clone(),
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => match &args.action {
                AccountsAction::Ls => flags_only(false, false),
                AccountsAction::Cp(args) => flags_only(args.flags.dry_run, args.flags.yes),
            },
            ClaudeCommand::Cache(args) => match &args.action {
                ClaudeCacheAction::Clear(flags) => flags_only(flags.dry_run, flags.yes),
                ClaudeCacheAction::Stats => flags_only(false, false),
            },
            ClaudeCommand::Sync(args) => match &args.action {
                None => flags_only(args.flags.dry_run, args.flags.yes),
                Some(SyncAction::Set(args)) => flags_only(args.flags.dry_run, args.flags.yes),
                Some(SyncAction::Rm(args)) => flags_only(args.flags.dry_run, args.flags.yes),
                Some(_) => flags_only(false, false),
            },
        },
        Command::Uninstall(args) => flags_only(args.dry_run, args.yes),
        Command::Update | Command::Github | Command::Help(_) | Command::RefreshIndex => {
            unprompted()
        }
    }
}

/// Index settings from `CHATKEEP_NO_INDEX`.
pub fn index_config() -> Option<index::Config> {
    index::Config::from_env(std::env::var("CHATKEEP_NO_INDEX").ok().as_deref())
}

/// The runtime for every Cursor installation on this machine, or `None` when there is none.
pub fn cursor_runtime(common: &CommonArgs) -> Result<Option<Runtime>> {
    let projects_dir = config::cursor_projects_dir()?;
    let chatkeep_home = config::chatkeep_home()?;
    let installs: Vec<Layout> = install::discover(&install::Roots::system()?)
        .into_iter()
        .map(|found| Layout {
            name: found.name,
            cursor_root: found.root,
            projects_dir: projects_dir.clone(),
            chatkeep_home: chatkeep_home.clone(),
        })
        .collect();
    let Some(layout) = installs.first().cloned() else {
        return Ok(None);
    };
    let roots = installs
        .iter()
        .map(|layout| layout.cursor_root.clone())
        .collect();
    configure(
        Runtime {
            layout,
            installs,
            pinned: false,
            dry_run: false,
            yes: false,
            profile: None,
            probe: Arc::new(SystemProbe::new(NativeProcesses, roots)),
            quiet: false,
            index: index_config(),
            guard_reached: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cwd: std::env::current_dir().context("cannot read the current directory")?,
            interactive: ui::interactive(),
        },
        common,
    )
    .map(Some)
}

/// Apply the shared flags to a runtime; `--profile` pins one installation.
pub fn configure(mut rt: Runtime, common: &CommonArgs) -> Result<Runtime> {
    rt.dry_run |= common.dry_run;
    rt.yes |= common.yes;
    match &common.profile {
        Some(selector) => select_profile(rt, selector),
        None => Ok(rt),
    }
}

fn profile_labels(rt: &Runtime) -> Result<Vec<String>> {
    let mut labels = Vec::new();
    for layout in &rt.installs {
        labels.push(layout.name.clone());
        for profile in engine::user_profiles(&rt.scoped(layout))? {
            labels.push(format!("{}/{}", layout.name, profile.name));
        }
    }
    Ok(labels)
}

fn unknown_profile(rt: &Runtime, selector: &str) -> Result<anyhow::Error> {
    Ok(ui::hinted(
        format!("no Cursor profile named {selector}"),
        format!("Known profiles: {}.", profile_labels(rt)?.join(", ")),
    ))
}

/// `NAME` picks an installation, `NAME/PROFILE` a VS Code profile inside it, and a bare
/// VS Code profile name works when exactly one installation has it.
pub fn select_profile(mut rt: Runtime, selector: &str) -> Result<Runtime> {
    let (install_name, profile) = match selector.split_once('/') {
        Some((install_name, profile)) => (install_name, Some(profile)),
        None => (selector, None),
    };
    if let Some(layout) = rt
        .installs
        .iter()
        .find(|layout| layout.name.eq_ignore_ascii_case(install_name))
        .cloned()
    {
        let profile = match profile {
            None => None,
            Some(wanted) if wanted.eq_ignore_ascii_case(DEFAULT) => Some(DEFAULT.to_string()),
            Some(wanted) => {
                let found = engine::user_profiles(&rt.scoped(&layout))?
                    .into_iter()
                    .find(|found| found.name.eq_ignore_ascii_case(wanted) || found.id == wanted);
                match found {
                    Some(found) => Some(found.name),
                    None => return Err(unknown_profile(&rt, selector)?),
                }
            }
        };
        rt.layout = layout;
        rt.pinned = true;
        rt.profile = profile;
        return Ok(rt);
    }
    if profile.is_none() {
        let mut hits = Vec::new();
        for layout in &rt.installs {
            for found in engine::user_profiles(&rt.scoped(layout))? {
                if found.name.eq_ignore_ascii_case(selector) || found.id == selector {
                    hits.push((layout.clone(), found.name));
                }
            }
        }
        if hits.len() > 1 {
            return Err(ui::hinted(
                format!(
                    "VS Code profile {selector} exists in {}",
                    hits.iter()
                        .map(|(layout, _)| layout.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                format!("Pass --profile INSTALLATION/{selector}."),
            ));
        }
        if let Some((layout, name)) = hits.pop() {
            rt.layout = layout;
            rt.pinned = true;
            rt.profile = Some(name);
            return Ok(rt);
        }
    }
    Err(unknown_profile(&rt, selector)?)
}

impl Command {
    pub fn resolve_paths(&mut self, cwd: &Path) {
        match self {
            Self::Mv(args) | Self::Cp(args) => {
                if let Some(to) = &mut args.to {
                    *to = anchor_user_path(to, cwd);
                }
            }
            Self::Cursor(CursorArgs {
                command: CursorCommand::Save(args),
            }) => {
                if let Some(to) = &mut args.to {
                    *to = anchor_user_path(to, cwd);
                }
            }
            Self::Split(args) => {
                for target in &mut args.targets {
                    *target = anchor_user_path(target, cwd);
                }
            }
            Self::Export(args) => {
                if let Some(file) = &mut args.file {
                    *file = anchor_user_path(file, cwd);
                }
            }
            Self::Import(args) => {
                if let Some(file) = &mut args.file {
                    *file = anchor_user_path(file, cwd);
                }
                if let Some(to) = &mut args.to {
                    *to = anchor_user_path(to, cwd);
                }
            }
            _ => {}
        }
    }
}

fn anchor_user_path(value: &str, cwd: &Path) -> String {
    if uri::path_is_absolute(Platform::current(), value) {
        value.to_string()
    } else {
        format!("{}{}{value}", cwd.display(), std::path::MAIN_SEPARATOR)
    }
}

pub fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Mv(_) => "mv",
        Command::Cp(_) => "cp",
        Command::Split(_) => "split",
        Command::Combine(_) => "combine",
        Command::Ls(_) => "ls",
        Command::Rm(_) => "rm",
        Command::Export(_) => "export",
        Command::Import(_) => "import",
        Command::History(_) => "history",
        Command::Stats(_) => "stats",
        Command::Queue(_) => "queue",
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(_) => "cursor save",
            CursorCommand::Rx(_) => "cursor rx",
            CursorCommand::Cache(_) => "cursor cache",
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(_) => "claude accounts",
            ClaudeCommand::Cache(_) => "claude cache",
            ClaudeCommand::Sync(_) => "claude sync",
        },
        Command::RefreshIndex => index::REFRESH_COMMAND,
        Command::Update => "update",
        Command::Uninstall(_) => "uninstall",
        Command::Github => "github",
        Command::Help(_) => "help",
    }
}

pub fn command_args(command: &Command) -> Vec<String> {
    match command {
        Command::Mv(args) | Command::Cp(args) => vec![
            args.from.clone().unwrap_or_default(),
            args.to.clone().unwrap_or_default(),
        ],
        Command::Split(args) => std::iter::once(args.source.clone().unwrap_or_default())
            .chain(args.targets.clone())
            .collect(),
        Command::Combine(args) => std::iter::once(args.target.clone().unwrap_or_default())
            .chain(args.sources.clone())
            .collect(),
        Command::Rm(args) => vec![args.target.clone().unwrap_or_default()],
        Command::Ls(args) => vec![args.id.clone().unwrap_or_default()],
        Command::Export(args) => vec![
            args.target.clone().unwrap_or_default(),
            args.file.clone().unwrap_or_default(),
        ],
        Command::Import(args) => vec![
            args.file.clone().unwrap_or_default(),
            args.to.clone().unwrap_or_default(),
        ],
        Command::Queue(args) => vec![match &args.action {
            QueueAction::Add(_) => "add".into(),
            QueueAction::List(_) => "list".into(),
            QueueAction::Rm(_) => "rm".into(),
            QueueAction::Clear(_) => "clear".into(),
            QueueAction::Retry(_) => "retry".into(),
            QueueAction::Execute(_) => "execute".into(),
        }],
        Command::Cursor(args) => match &args.command {
            CursorCommand::Save(args) => vec![
                args.id.clone().unwrap_or_default(),
                args.to.clone().unwrap_or_default(),
            ],
            CursorCommand::Rx(args) => vec![args.target.clone().unwrap_or_default()],
            CursorCommand::Cache(args) => vec![args.action.name().to_string()],
        },
        Command::Claude(args) => match &args.command {
            ClaudeCommand::Accounts(args) => match &args.action {
                AccountsAction::Ls => vec!["ls".into()],
                AccountsAction::Cp(args) => vec![
                    "cp".into(),
                    args.from.clone().unwrap_or_default(),
                    args.to.clone().unwrap_or_default(),
                ],
            },
            ClaudeCommand::Cache(args) => vec![match &args.action {
                ClaudeCacheAction::Clear(_) => "clear".into(),
                ClaudeCacheAction::Stats => "stats".into(),
            }],
            ClaudeCommand::Sync(args) => match &args.action {
                None => vec![args.profile.clone().unwrap_or_default()],
                Some(SyncAction::Profiles) => vec!["profiles".into()],
                Some(SyncAction::Set(args)) => std::iter::once("set".to_string())
                    .chain(std::iter::once(args.name.clone()))
                    .chain(args.accounts.clone())
                    .collect(),
                Some(SyncAction::Rm(args)) => vec!["rm".into(), args.name.clone()],
                Some(SyncAction::Watch(args)) => {
                    vec!["watch".into(), args.profile.clone().unwrap_or_default()]
                }
                Some(SyncAction::Log(_)) => vec!["log".into()],
                Some(SyncAction::Auto(args)) => vec![
                    "auto".into(),
                    match args.action {
                        SyncAutoAction::On => "on".into(),
                        SyncAutoAction::Off => "off".into(),
                        SyncAutoAction::Status => "status".into(),
                    },
                ],
            },
        },
        Command::History(_)
        | Command::Stats(_)
        | Command::Update
        | Command::Uninstall(_)
        | Command::Github
        | Command::Help(_)
        | Command::RefreshIndex => Vec::new(),
    }
}
