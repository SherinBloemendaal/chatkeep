//! `ls` for Claude Code: every project, or the sessions of one.

use chrono::{DateTime, Local};
use comfy_table::{Attribute, Cell, Color};
use owo_colors::Style;

use super::store::{self, Project};
use crate::ui::{self, Align, Sheet, Theme};

const LIST_COLUMNS: [(&str, Align); 6] = [
    ("dest", Align::Left),
    ("project", Align::Left),
    ("sessions", Align::Right),
    ("subagents", Align::Right),
    ("memory", Align::Right),
    ("size", Align::Right),
];

const SESSION_COLUMNS: [(&str, Align); 5] = [
    ("session", Align::Left),
    ("title", Align::Left),
    ("last active", Align::Left),
    ("subagents", Align::Right),
    ("size", Align::Right),
];

/// Missing destinations first, then largest first, as the Cursor list sorts.
pub fn sort(projects: &mut [Project]) {
    projects.sort_by(|a, b| {
        b.destination_missing()
            .cmp(&a.destination_missing())
            .then_with(|| b.size.cmp(&a.size))
            .then_with(|| name(a).cmp(&name(b)))
    });
}

pub fn name(project: &Project) -> String {
    project
        .path
        .as_ref()
        .map(|path| ui::home_relative(&path.display().to_string()))
        .unwrap_or_else(|| project.slug.clone())
}

fn status_cell(theme: Theme, project: &Project) -> Cell {
    let icons = theme.icons();
    match &project.path {
        None => theme.cell(icons.hollow, None, &[Attribute::Dim]),
        Some(_) if project.destination_missing() => {
            theme.cell(icons.cross, Some(Color::Red), &[Attribute::Bold])
        }
        Some(_) => theme.cell(icons.check, Some(Color::Green), &[Attribute::Bold]),
    }
}

fn name_cell(theme: Theme, project: &Project, text: String) -> Cell {
    match &project.path {
        None => theme.cell(text, None, &[Attribute::Dim]),
        Some(_) if project.destination_missing() => theme.cell(text, Some(Color::Red), &[]),
        Some(_) => theme.cell(text, Some(Color::Cyan), &[]),
    }
}

pub fn render_list(theme: Theme, projects: &[Project]) -> String {
    render_list_at(theme, projects, ui::table_width())
}

pub fn render_list_at(theme: Theme, projects: &[Project], total: Option<usize>) -> String {
    let heading = ui::section_line(theme, "Claude Code projects");
    if projects.is_empty() {
        return format!(
            "{heading}\n{}\n",
            ui::info_line(theme, "No Claude Code projects found.")
        );
    }
    let mut sheet = Sheet::new(theme, &LIST_COLUMNS)
        .flex(1)
        .min_flex(28)
        .optional(&[4, 3])
        .at(total);
    for project in projects {
        let text = name(project);
        let short = {
            let project = project.clone();
            let text = text.clone();
            move |room: usize| {
                name_cell(
                    theme,
                    &project,
                    ui::layout::shorten_path(&text, room, theme.icons().ellipsis),
                )
            }
        };
        sheet.fitted_row(
            vec![
                status_cell(theme, project),
                name_cell(theme, project, text),
                theme.count_cell(project.sessions.len(), None),
                theme.count_cell(project.subagents(), Some(Color::Magenta)),
                theme.count_cell(project.memory_files, None),
                theme.size_cell(project.size),
            ],
            short,
        );
    }
    format!("{heading}\n{sheet}\n{}", footer(theme, projects, total))
}

fn footer(theme: Theme, projects: &[Project], total: Option<usize>) -> String {
    let icons = theme.icons();
    let missing = projects
        .iter()
        .filter(|project| project.destination_missing())
        .count();
    let sessions: usize = projects.iter().map(|project| project.sessions.len()).sum();
    let subagents: usize = projects.iter().map(Project::subagents).sum();
    let size: u64 = projects.iter().map(|project| project.size).sum();
    let missing = if missing == 0 {
        format!(
            "{} {}",
            theme.good(icons.check),
            theme.good("no missing destinations")
        )
    } else {
        format!(
            "{} {}",
            theme.paint(icons.cross, Style::new().bold().red()),
            theme.bad(ui::plural(
                missing,
                "missing destination",
                "missing destinations"
            ))
        )
    };
    let parts = [
        format!(
            "{} {}",
            theme.paint(icons.square, Style::new().blue()),
            theme.bold(ui::plural(projects.len(), "project", "projects"))
        ),
        missing,
        format!(
            "{} {}, {}",
            theme.paint(icons.diamond, Style::new().cyan()),
            theme.bold(ui::plural(sessions, "session", "sessions")),
            ui::plural(subagents, "subagent", "subagents")
        ),
        format!(
            "{} {} on disk",
            theme.paint(icons.bullet, Style::new().yellow()),
            theme.size(size)
        ),
    ];
    ui::summary_at(theme, &parts, total)
}

pub fn render_detail(theme: Theme, project: &Project) -> String {
    render_detail_at(theme, project, ui::table_width())
}

pub fn render_detail_at(theme: Theme, project: &Project, total: Option<usize>) -> String {
    let mut out = format!("{}\n", ui::section_line(theme, &name(project)));
    let folder = match &project.path {
        None => "unknown: chatkeep found no path for this project".to_string(),
        Some(path) if project.destination_missing() => format!("{} (missing)", path.display()),
        Some(path) => path.display().to_string(),
    };
    out.push_str(&ui::info_line(theme, &format!("folder: {folder}")));
    out.push('\n');
    out.push_str(&ui::info_line(
        theme,
        &format!("stored in: {}", project.dir.display()),
    ));
    out.push('\n');
    if project.sessions.is_empty() {
        out.push_str(&ui::info_line(theme, "No sessions."));
        out.push('\n');
        return out;
    }
    let mut sheet = Sheet::new(theme, &SESSION_COLUMNS)
        .flex(1)
        .min_flex(20)
        .optional(&[3, 2])
        .at(total);
    for session in &project.sessions {
        let title = store::title(project, session).unwrap_or_default();
        let active = session
            .modified
            .map(|time| {
                DateTime::<Local>::from(time)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        sheet.row(vec![
            theme.cell(&session.id, Some(Color::Magenta), &[]),
            theme.cell(title, None, &[]),
            theme.cell(active, None, &[Attribute::Dim]),
            theme.count_cell(session.subagents, Some(Color::Magenta)),
            theme.size_cell(session.size),
        ]);
    }
    out.push_str(&format!("{sheet}\n"));
    out
}
