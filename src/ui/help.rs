//! The top-level `chatkeep` help screen: banner, then one strictly aligned grid.

use owo_colors::Style;
use std::io::{self, Write};

use super::banner::banner;
use super::layout::{Grid, wrap_indented};
use super::{Theme, section_line, term};

const INDENT: usize = 2;

pub struct Entry {
    pub icon: (&'static str, &'static str),
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub about: &'static str,
}

pub struct Group {
    pub title: &'static str,
    pub style: fn() -> Style,
    pub entries: &'static [Entry],
}

pub struct Flag {
    pub short: &'static str,
    pub long: &'static str,
    pub value: &'static str,
    pub about: &'static str,
}

const fn entry(
    icon: (&'static str, &'static str),
    name: &'static str,
    aliases: &'static [&'static str],
    args: &'static str,
    about: &'static str,
) -> Entry {
    Entry {
        icon,
        name,
        aliases,
        args,
        about,
    }
}

const fn flag(
    short: &'static str,
    long: &'static str,
    value: &'static str,
    about: &'static str,
) -> Flag {
    Flag {
        short,
        long,
        value,
        about,
    }
}

pub const GROUPS: &[Group] = &[
    Group {
        title: "Migrate",
        style: || Style::new().cyan(),
        entries: &[
            entry(
                ("➜", ">"),
                "mv",
                &["move"],
                "[FROM] [TO]",
                "Repath chats after a project folder moved",
            ),
            entry(
                ("✚", "+"),
                "cp",
                &["copy"],
                "[FROM] [TO]",
                "Copy a project's chats to another folder",
            ),
        ],
    },
    Group {
        title: "Chats",
        style: || Style::new().magenta(),
        entries: &[
            entry(
                ("⇉", "<"),
                "split",
                &[],
                "[SOURCE] [TARGETS...]",
                "Copy chats from one project into separate projects",
            ),
            entry(
                ("⊕", "+"),
                "combine",
                &[],
                "[TARGET] [SOURCES...]",
                "Bring chats from several sources into one project",
            ),
            entry(
                ("✗", "x"),
                "rm",
                &[],
                "[TARGET]",
                "Remove a project's chats or a single chat",
            ),
        ],
    },
    Group {
        title: "Inspect",
        style: || Style::new().green(),
        entries: &[
            entry(
                ("≡", "="),
                "ls",
                &["list"],
                "[ID]",
                "List projects, missing folders first, then by size",
            ),
            entry(
                ("▤", "#"),
                "stats",
                &[],
                "",
                "Projects, chats, tokens, and models at a glance",
            ),
            entry(
                ("↺", "~"),
                "history",
                &[],
                "",
                "The local command log of the last 30 days",
            ),
        ],
    },
    Group {
        title: "Archive",
        style: || Style::new().yellow(),
        entries: &[
            entry(
                ("⇧", "^"),
                "export",
                &[],
                "[TARGET] [FILE]",
                "Write a project and its chats to a .chatkeep archive",
            ),
            entry(
                ("⇩", "v"),
                "import",
                &[],
                "[FILE] [TO]",
                "Restore a .chatkeep archive, optionally into a new folder",
            ),
        ],
    },
    Group {
        title: "Queue",
        style: || Style::new().magenta(),
        entries: &[entry(
            ("◇", "o"),
            "queue",
            &[],
            "ACTION",
            "Queue write commands while a tool is still open",
        )],
    },
    Group {
        title: "Cursor only",
        style: || Style::new().cyan(),
        entries: &[
            entry(
                ("✎", "*"),
                "cursor",
                &[],
                "save [ID] [TO]",
                "Attach an unsaved Workspaces/<ts> session to a folder",
            ),
            entry(
                ("↻", "@"),
                "cursor",
                &[],
                "rx [TARGET]",
                "Rebuild registry refs, rewrite stale paths, clear caches",
            ),
            entry(
                ("◫", "%"),
                "cursor",
                &[],
                "cache clear|scan|stats",
                "Clear, rescan, or inspect the persistent index",
            ),
        ],
    },
    Group {
        title: "Claude Code only",
        style: || Style::new().magenta(),
        entries: &[
            entry(
                ("◈", "o"),
                "claude",
                &[],
                "accounts ls|cp",
                "Desktop app accounts; copy chats from one to another",
            ),
            entry(
                ("⇄", "="),
                "claude",
                &[],
                "sync [PROFILE]",
                "Keep the chat lists of several accounts the same",
            ),
            entry(
                ("◫", "%"),
                "claude",
                &[],
                "cache clear|stats",
                "Clear or inspect the index of the transcripts",
            ),
        ],
    },
    Group {
        title: "Maintenance",
        style: || Style::new().blue(),
        entries: &[
            entry(
                ("✦", "*"),
                "update",
                &[],
                "",
                "Download, verify, and install the latest release",
            ),
            entry(
                ("⌫", "-"),
                "uninstall",
                &[],
                "",
                "Remove the installed binary and its PATH entry",
            ),
            entry(
                ("⌂", "&"),
                "github",
                &[],
                "",
                "Open the GitHub repository in the browser",
            ),
            entry(
                ("?", "?"),
                "help",
                &[],
                "[COMMAND]",
                "This screen, or every option of one command",
            ),
        ],
    },
];

pub const FLAGS: &[Flag] = &[
    flag("-n", "--dry-run", "", "Show the plan and change nothing"),
    flag("-y", "--yes", "", "Skip the confirmation prompt"),
    flag(
        "",
        "--profile",
        "NAME",
        "Limit work to one Cursor installation, or NAME/PROFILE",
    ),
    flag(
        "",
        "--tool",
        "TOOL",
        "Only cursor or claude; without it, every tool found",
    ),
    flag(
        "",
        "--replace",
        "FROM TO",
        "Rewrite every matching path from FROM to TO",
    ),
    flag(
        "",
        "--regex",
        "",
        "Treat --replace FROM as a regular expression",
    ),
    flag(
        "",
        "--unsaved",
        "",
        "Only consider unsaved Workspaces/<ts> sessions of Cursor",
    ),
    flag(
        "",
        "--project",
        "",
        "With mv or cp, also move or copy the real project folder",
    ),
    flag(
        "",
        "--move",
        "",
        "With split or combine, move chats instead of copying them",
    ),
    flag(
        "",
        "--copy",
        "",
        "With combine, keep chats in the sources (the default)",
    ),
    flag(
        "",
        "--overwrite",
        "",
        "With import, replace chats that already exist",
    ),
    flag(
        "",
        "--fresh",
        "",
        "With ls or stats, skip the index and read the tools live",
    ),
    flag(
        "",
        "--full",
        "",
        "With cursor cache scan, rebuild the index; with queue list, exact commands",
    ),
    flag(
        "",
        "--purge",
        "",
        "With uninstall, also delete history, index, and backups",
    ),
    flag(
        "",
        "--color",
        "WHEN",
        "Color output: auto, always, or never",
    ),
    flag("-h", "--help", "", "Show this help"),
    flag("-V", "--version", "", "Print the version"),
];

pub const EXAMPLES: &[(&str, &str)] = &[
    ("chatkeep ls", "Every project, missing folders on top"),
    ("chatkeep ls ~/code/app", "One project and all of its chats"),
    (
        "chatkeep mv ~/old/app ~/new/app",
        "Repath after moving a project folder",
    ),
    (
        "chatkeep mv -n --replace ~/a ~/b",
        "Preview a bulk rename without changing anything",
    ),
    (
        "chatkeep cp ~/app ~/app2 --project",
        "Copy the folder together with its chats",
    ),
    (
        "chatkeep split ~/mono ~/mono/api",
        "Spread monorepo chats over sub-projects",
    ),
    (
        "chatkeep export ~/app app.chatkeep",
        "Write a portable backup archive",
    ),
    (
        "chatkeep queue add -- mv ~/a ~/b",
        "Queue a move while the tool is still open",
    ),
    (
        "chatkeep claude sync set work A B",
        "Let accounts A and B share their chat lists",
    ),
    (
        "chatkeep claude sync auto on",
        "Sync those lists in the background from now on",
    ),
    ("chatkeep help mv", "Every option of one command"),
];

struct Row {
    cells: Vec<String>,
    about: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Commands,
    Flags,
    Examples,
}

struct Section {
    title: &'static str,
    kind: Kind,
    rows: Vec<Row>,
}

fn command_rows(theme: Theme, group: &Group) -> Vec<Row> {
    let style = (group.style)();
    group
        .entries
        .iter()
        .map(|entry| {
            let icon = if theme.unicode() {
                entry.icon.0
            } else {
                entry.icon.1
            };
            let mut name = theme.paint(entry.name, style.bold());
            for alias in entry.aliases {
                name.push_str(&theme.dim(format!(", {alias}")));
            }
            Row {
                cells: vec![theme.paint(icon, style), name, theme.dim(entry.args)],
                about: entry.about,
            }
        })
        .collect()
}

fn flag_rows(theme: Theme) -> Vec<Row> {
    FLAGS
        .iter()
        .map(|flag| {
            let name = if flag.short.is_empty() {
                format!("    {}", theme.flag(flag.long))
            } else {
                format!(
                    "{}{} {}",
                    theme.flag(flag.short),
                    theme.dim(","),
                    theme.flag(flag.long)
                )
            };
            Row {
                cells: vec![name, theme.dim(flag.value)],
                about: flag.about,
            }
        })
        .collect()
}

fn example_rows(theme: Theme) -> Vec<Row> {
    EXAMPLES
        .iter()
        .map(|(command, about)| Row {
            cells: vec![theme.dim("$"), highlight(theme, command)],
            about,
        })
        .collect()
}

fn sections(theme: Theme) -> Vec<Section> {
    let mut out: Vec<Section> = GROUPS
        .iter()
        .map(|group| Section {
            title: group.title,
            kind: Kind::Commands,
            rows: command_rows(theme, group),
        })
        .collect();
    out.push(Section {
        title: "Flags",
        kind: Kind::Flags,
        rows: flag_rows(theme),
    });
    out.push(Section {
        title: "Examples",
        kind: Kind::Examples,
        rows: example_rows(theme),
    });
    out
}

fn fit(sections: &[Section], kind: Kind) -> Grid {
    Grid::fit(
        INDENT,
        sections
            .iter()
            .filter(|section| section.kind == kind)
            .flat_map(|section| section.rows.iter().map(|row| row.cells.as_slice())),
    )
}

pub fn help_text(theme: Theme, total: usize) -> String {
    let sections = sections(theme);
    let mut commands = fit(&sections, Kind::Commands);
    let mut flags = fit(&sections, Kind::Flags);
    let mut examples = fit(&sections, Kind::Examples);
    let values = commands.column_start(2).max(flags.column_start(1));
    commands.align_column_to(2, values);
    flags.align_column_to(1, values);
    let column = [&commands, &flags, &examples]
        .iter()
        .map(|grid| grid.text_column())
        .max()
        .unwrap_or(0);
    for grid in [&mut commands, &mut flags, &mut examples] {
        grid.align_text_to(column);
    }
    let mut out = banner(theme, total);
    for section in &sections {
        let grid = match section.kind {
            Kind::Commands => &commands,
            Kind::Flags => &flags,
            Kind::Examples => &examples,
        };
        out.push('\n');
        out.push_str(&section_line(theme, section.title));
        out.push('\n');
        for row in &section.rows {
            out.push_str(
                &grid.render(&row.cells, row.about, total, |line| match section.kind {
                    Kind::Examples => theme.dim(line),
                    Kind::Commands | Kind::Flags => line.to_string(),
                }),
            );
        }
    }
    out.push('\n');
    let star = if theme.unicode() { "★" } else { "*" };
    out.push_str(&wrap_indented(
        &format!("{star} {}", crate::update::REPO_URL),
        INDENT,
        total,
        |line| match line.split_once(' ') {
            Some((icon, rest)) if icon == star => {
                format!(
                    "{} {}",
                    theme.paint(icon, Style::new().yellow()),
                    theme.paint(rest, Style::new().cyan().underline())
                )
            }
            _ => theme.paint(line, Style::new().cyan().underline()),
        },
    ));
    out
}

pub fn print_help() {
    let mut stdout = io::stdout().lock();
    let _ = write!(stdout, "{}", help_text(Theme::stdout(), term::width()));
}

/// Grid cells plus the wrapped description of one help row.
type HelpRow = (Vec<String>, String);

/// One command's help (`chatkeep help mv`, `chatkeep mv --help`) in the same grid as the main
/// screen, wrapped to the terminal. `path` is the command path, e.g. `["queue", "add"]`.
pub fn command_help(theme: Theme, command: &clap::Command, path: &[&str], total: usize) -> String {
    let full = format!("chatkeep {}", path.join(" "));
    let mut out = String::new();
    let mut title = theme.command(&full);
    let aliases: Vec<&str> = command.get_visible_aliases().collect();
    if !aliases.is_empty() {
        title.push_str(&theme.dim(format!(", {}", aliases.join(", "))));
    }
    out.push_str(&format!(
        "{} {title}\n",
        theme.heading(theme.icons().header)
    ));
    // A namespace has one row per command on the main screen: those rows describe its
    // commands, not the namespace.
    let listed = GROUPS
        .iter()
        .flat_map(|group| group.entries.iter())
        .find(|entry| {
            let first = entry.args.split(' ').next().unwrap_or_default();
            path.len() == 1 && entry.name == path[0] && command.find_subcommand(first).is_none()
        })
        .map(|entry| entry.about.to_string());
    if let Some(about) = listed.or_else(|| command.get_about().map(|about| about.to_string())) {
        let about = about.split(" Alias:").next().unwrap_or(&about).trim_end();
        let about = about.trim_end_matches('.');
        out.push_str(&wrap_indented(about, INDENT, total, |line| {
            line.to_string()
        }));
    }

    let mut usage = command.clone();
    let usage = super::layout::strip_ansi(&usage.render_usage().to_string());
    let usage = usage.trim().trim_start_matches("Usage:").trim();
    let usage = if usage.starts_with("chatkeep ") {
        usage.to_string()
    } else {
        format!("chatkeep {usage}")
    };
    out.push('\n');
    out.push_str(&section_line(theme, "Usage"));
    out.push('\n');
    let lead = INDENT + 2;
    for (index, line) in super::layout::wrap(&usage, total.saturating_sub(lead).max(16))
        .iter()
        .enumerate()
    {
        if index == 0 {
            out.push_str(&format!(
                "{}{} {}\n",
                " ".repeat(INDENT),
                theme.dim("$"),
                highlight(theme, line)
            ));
        } else {
            out.push_str(&format!("{}{}\n", " ".repeat(lead), theme.dim(line)));
        }
    }

    let mut sections: Vec<(&str, Vec<HelpRow>)> = Vec::new();
    let actions: Vec<HelpRow> = command
        .get_subcommands()
        .filter(|sub| !sub.is_hide_set())
        .map(|sub| {
            let mut name = theme.command(sub.get_name());
            for alias in sub.get_visible_aliases() {
                name.push_str(&theme.dim(format!(", {alias}")));
            }
            let about = sub
                .get_about()
                .map(|about| about.to_string())
                .unwrap_or_default();
            let about = about.split(" Alias:").next().unwrap_or(&about).to_string();
            (vec![name], about.trim_end_matches('.').to_string())
        })
        .collect();
    if !actions.is_empty() {
        sections.push(("Actions", actions));
    }
    let arguments: Vec<HelpRow> = command
        .get_positionals()
        .filter(|arg| !arg.is_hide_set())
        .map(|arg| {
            let name = arg
                .get_value_names()
                .and_then(|names| names.first())
                .map(|name| name.to_string())
                .unwrap_or_else(|| arg.get_id().to_string().to_uppercase());
            let many = arg
                .get_num_args()
                .is_some_and(|range| range.max_values() > 1);
            let name = if many { format!("{name}...") } else { name };
            (vec![theme.dim(name)], help_of(arg))
        })
        .collect();
    if !arguments.is_empty() {
        sections.push(("Arguments", arguments));
    }
    let mut options: Vec<&clap::Arg> = command
        .get_arguments()
        .filter(|arg| !arg.is_positional() && !arg.is_hide_set())
        .collect();
    options.sort_by_key(|arg| arg.is_global_set());
    let options: Vec<HelpRow> = options
        .into_iter()
        .map(|arg| {
            let long = arg
                .get_long()
                .map(|long| theme.flag(format!("--{long}")))
                .unwrap_or_default();
            let flag = match arg.get_short() {
                Some(short) => format!(
                    "{}{} {long}",
                    theme.flag(format!("-{short}")),
                    theme.dim(",")
                ),
                None => format!("    {long}"),
            };
            let takes = arg.get_num_args().is_some_and(|range| range.takes_values());
            let value = if takes {
                arg.get_value_names()
                    .map(|names| {
                        names
                            .iter()
                            .map(|name| name.to_string())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_else(|| arg.get_id().to_string().to_uppercase())
            } else {
                String::new()
            };
            (vec![flag, theme.dim(value)], help_of(arg))
        })
        .collect();
    if !options.is_empty() {
        sections.push(("Options", options));
    }
    let examples: Vec<HelpRow> = EXAMPLES
        .iter()
        .filter(|(example, _)| {
            example
                .strip_prefix(&full)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
        })
        .map(|(example, about)| {
            (
                vec![theme.dim("$"), highlight(theme, example)],
                about.to_string(),
            )
        })
        .collect();
    if !examples.is_empty() {
        sections.push(("Examples", examples));
    }

    let grid = Grid::fit(
        INDENT,
        sections
            .iter()
            .filter(|(title, _)| *title != "Examples")
            .flat_map(|(_, rows)| rows.iter().map(|(cells, _)| cells.as_slice())),
    );
    let sample = Grid::fit(
        INDENT,
        sections
            .iter()
            .filter(|(title, _)| *title == "Examples")
            .flat_map(|(_, rows)| rows.iter().map(|(cells, _)| cells.as_slice())),
    );
    for (title, rows) in &sections {
        out.push('\n');
        out.push_str(&section_line(theme, title));
        out.push('\n');
        let grid = if *title == "Examples" { &sample } else { &grid };
        for (cells, about) in rows {
            out.push_str(&grid.render(cells, about, total, |line| {
                if *title == "Examples" {
                    theme.dim(line)
                } else {
                    line.to_string()
                }
            }));
        }
    }
    out
}

fn help_of(arg: &clap::Arg) -> String {
    let text = arg
        .get_help()
        .map(|help| help.to_string())
        .unwrap_or_default();
    text.trim_end_matches('.').to_string()
}

fn highlight(theme: Theme, command: &str) -> String {
    command
        .split(' ')
        .enumerate()
        .map(|(index, word)| match index {
            0 => theme.paint(word, Style::new().bold().green()),
            1 => theme.command(word),
            _ if word.starts_with('-') => theme.flag(word),
            _ => theme.token(word),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::{strip_ansi, width};
    use crate::ui::term::Depth;

    fn themes() -> [Theme; 4] {
        [
            Theme::plain(),
            Theme::colored(),
            Theme::colored().with_depth(Depth::TrueColor),
            Theme::colored().with_depth(Depth::Ansi256),
        ]
    }

    fn offset(line: &str, needle: &str) -> usize {
        let index = line
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} not in {line:?}"));
        width(&line[..index])
    }

    fn first_word(text: &str) -> &str {
        text.split(' ').next().unwrap()
    }

    /// The row of `entry`. A namespace has one row per command, told apart by its arguments.
    fn command_line<'a>(lines: &'a [&'a str], entry: &Entry) -> &'a str {
        lines
            .iter()
            .find(|line| {
                let cells: Vec<&str> = line.split_whitespace().collect();
                cells.len() > 1
                    && cells[1].trim_end_matches(',') == entry.name
                    && line.contains(entry.args)
            })
            .unwrap_or_else(|| panic!("no row for {} {}", entry.name, entry.args))
    }

    fn assert_logo_gap(text: &str) {
        let lines: Vec<&str> = text.lines().collect();
        let tag = lines
            .iter()
            .position(|line| line.contains("Keep AI chat history attached to your projects."))
            .expect("tagline");
        assert!(tag >= 3, "logo, two blank lines, tagline");
        assert_eq!(lines[tag - 1], "", "blank line under the logo");
        assert_eq!(lines[tag - 2], "", "blank line under the logo");
        let logo = lines[tag - 3];
        assert!(
            logo.contains('╚') || logo.contains('|') || logo.contains('_'),
            "logo line above the blank lines: {logo:?}"
        );
    }

    #[test]
    fn every_row_shares_one_description_column() {
        for total in [80, 100, 120] {
            let plain = help_text(Theme::plain(), total);
            assert_logo_gap(&plain);
            for theme in themes() {
                let text = strip_ansi(&help_text(theme, total));
                assert_eq!(text, plain, "colors changed the layout at {total}");
                let lines: Vec<&str> = text.lines().collect();
                let mut columns = Vec::new();
                for group in GROUPS {
                    for entry in group.entries {
                        let line = command_line(&lines, entry);
                        assert_eq!(offset(line, entry.name), INDENT + 1 + 2);
                        if !entry.args.is_empty() {
                            columns.push(("args", offset(line, entry.args)));
                        }
                        columns.push(("about", offset(line, first_word(entry.about))));
                    }
                }
                for flag in FLAGS {
                    let line = lines
                        .iter()
                        .find(|line| {
                            line.contains(&format!(" {} ", flag.long)) || line.ends_with(flag.long)
                        })
                        .filter(|line| line.trim_start().starts_with('-'))
                        .unwrap_or_else(|| panic!("no row for {}", flag.long));
                    if !flag.value.is_empty() {
                        columns.push(("args", offset(line, flag.value)));
                    }
                    columns.push(("about", offset(line, first_word(flag.about))));
                }
                for (command, about) in EXAMPLES {
                    let line = lines.iter().find(|line| line.contains(command)).unwrap();
                    columns.push(("about", offset(line, first_word(about))));
                }
                let about: Vec<usize> = columns
                    .iter()
                    .filter(|(kind, _)| *kind == "about")
                    .map(|(_, column)| *column)
                    .collect();
                assert!(about.iter().all(|column| *column == about[0]), "{about:?}");
                let args: Vec<usize> = columns
                    .iter()
                    .filter(|(kind, _)| *kind == "args")
                    .map(|(_, column)| *column)
                    .collect();
                assert!(args.iter().all(|column| *column == args[0]), "{args:?}");
                for line in &lines {
                    assert!(width(line) <= total, "{} > {total}: {line:?}", width(line));
                }
                let grid_start = lines
                    .iter()
                    .position(|line| line.starts_with("==> "))
                    .unwrap();
                for line in &lines[grid_start..] {
                    let lead = line.len() - line.trim_start().len();
                    if lead > INDENT && !line.trim_start().starts_with("--") {
                        assert_eq!(lead, about[0], "ragged continuation: {line:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn sections_are_separated_by_one_blank_line() {
        let text = help_text(Theme::plain(), 100);
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if line.starts_with("==> ") {
                assert_eq!(lines[index - 1], "", "{line}");
                assert_ne!(lines[index - 2], "", "{line}");
            }
        }
        for title in [
            "Migrate",
            "Chats",
            "Inspect",
            "Archive",
            "Queue",
            "Cursor only",
            "Claude Code only",
            "Maintenance",
            "Flags",
            "Examples",
        ] {
            assert!(lines.contains(&format!("==> {title}").as_str()), "{title}");
        }
        let tagline_at = text
            .find("Keep AI chat history attached to your projects.")
            .expect("tagline");
        assert_eq!(text[..tagline_at].matches("\n\n\n").count(), 1);
        assert!(!text[tagline_at..].contains("\n\n\n"));
    }

    #[test]
    fn help_lists_every_command_alias_and_flag() {
        let text = help_text(Theme::plain(), 100);
        for group in GROUPS {
            for entry in group.entries {
                assert!(text.contains(entry.about), "{}", entry.name);
                for alias in entry.aliases {
                    assert!(text.contains(&format!("{}, {alias}", entry.name)));
                }
            }
        }
        for flag in FLAGS {
            assert!(text.contains(flag.long), "{}", flag.long);
        }
        assert!(!text.contains("--move-chats"));
        assert!(text.contains(crate::update::REPO_URL));
        assert!(!text.contains('\u{1b}'));
        assert!(help_text(Theme::colored(), 100).contains('\u{1b}'));
    }

    #[test]
    fn ascii_help_keeps_the_grid() {
        let text = help_text(Theme::ascii(), 80);
        assert_logo_gap(&text);
        assert!(text.is_ascii());
        let lines: Vec<&str> = text.lines().collect();
        let columns: Vec<usize> = GROUPS
            .iter()
            .flat_map(|group| group.entries.iter())
            .map(|entry| offset(command_line(&lines, entry), first_word(entry.about)))
            .collect();
        assert!(columns.iter().all(|column| *column == columns[0]));
    }
}
