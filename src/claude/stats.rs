//! `stats` for Claude Code: projects, sessions, tokens, and models, read from the transcripts.

use anyhow::Result;
use comfy_table::{Attribute, Cell, CellAlignment, Color};
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Path;

use super::ops::files_under;
use super::store::{self, Project};
use super::view;
use super::{Runtime, index};
use crate::ui::{self, Align, Sheet, Theme};

const TOP: usize = 10;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Tokens {
    fn absorb(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// What one transcript says about usage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub tokens: Tokens,
    /// Answers per model.
    pub models: BTreeMap<String, usize>,
    /// `YYYY-MM` of the first line that carries a time.
    pub month: Option<String>,
}

impl Usage {
    /// The model that gave the most answers.
    pub fn model(&self) -> Option<&str> {
        self.models
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(name, _)| name.as_str())
    }
}

/// Usage of one transcript, read now.
pub fn read_usage(transcript: &Path) -> Result<Usage> {
    Ok(index::read_facts(transcript)?.usage)
}

/// Usage of one transcript, from the index when it is on.
fn usage_of(cache: Option<&index::Cache>, transcript: &Path) -> Result<Usage> {
    match cache {
        Some(cache) => Ok(cache.facts(transcript)?.usage),
        None => read_usage(transcript),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub name: String,
    pub value: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub projects: usize,
    pub missing: usize,
    pub sessions: usize,
    pub subagents: usize,
    pub memory_files: usize,
    pub size: u64,
    pub tokens: Tokens,
    pub subagent_tokens: Tokens,
    /// Sessions per model, by the model that gave most of a session's answers.
    pub models: BTreeMap<String, usize>,
    pub per_month: BTreeMap<String, usize>,
    pub busiest: Vec<Named>,
    pub largest: Vec<Named>,
}

pub fn collect(rt: &Runtime) -> Result<Stats> {
    let projects = store::discover(rt)?;
    let _spinner = ui::spinner("Reading chat usage", rt.quiet);
    gather(&projects, rt.open_cache().as_ref())
}

pub fn gather(projects: &[Project], cache: Option<&index::Cache>) -> Result<Stats> {
    let mut stats = Stats {
        projects: projects.len(),
        ..Stats::default()
    };
    for project in projects {
        stats.missing += usize::from(project.destination_missing());
        stats.sessions += project.sessions.len();
        stats.subagents += project.subagents();
        stats.memory_files += project.memory_files;
        stats.size += project.size;
        let name = view::name(project);
        for session in &project.sessions {
            let usage = match &session.facts {
                Some(facts) => facts.usage.clone(),
                None => read_usage(&session.transcript)?,
            };
            stats.tokens.absorb(&usage.tokens);
            if let Some(model) = usage.model() {
                *stats.models.entry(model.to_string()).or_default() += 1;
            }
            if let Some(month) = usage.month {
                *stats.per_month.entry(month).or_default() += 1;
            }
            let subagents = project.dir.join(&session.id).join("subagents");
            if subagents.is_dir() {
                for file in files_under(&subagents)? {
                    if file.extension().is_some_and(|ext| ext == "jsonl") {
                        stats
                            .subagent_tokens
                            .absorb(&usage_of(cache, &file)?.tokens);
                    }
                }
            }
        }
        if !project.sessions.is_empty() {
            stats.busiest.push(Named {
                name: name.clone(),
                value: project.sessions.len() as u64,
            });
        }
        stats.largest.push(Named {
            name,
            value: project.size,
        });
    }
    for list in [&mut stats.busiest, &mut stats.largest] {
        list.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.name.cmp(&b.name)));
    }
    Ok(stats)
}

fn left(theme: Theme, text: impl std::fmt::Display) -> Cell {
    theme.cell(text, None, &[])
}

fn dim(theme: Theme, text: impl std::fmt::Display) -> Cell {
    theme.cell(text, None, &[Attribute::Dim])
}

fn number(theme: Theme, value: u64) -> Cell {
    theme
        .cell(ui::count(value), None, &[Attribute::Bold])
        .set_alignment(CellAlignment::Right)
}

fn overview(theme: Theme, stats: &Stats) -> Sheet {
    let mut sheet = Sheet::new(
        theme,
        &[
            ("metric", Align::Left),
            ("value", Align::Right),
            ("detail", Align::Left),
        ],
    )
    .flex(2)
    .min_flex(20)
    .optional(&[2]);
    let metric = |name: &str| theme.cell(name, Some(Color::Blue), &[Attribute::Bold]);
    let value = |text: String, color: Option<Color>| {
        theme
            .cell(text, color, &[Attribute::Bold])
            .set_alignment(CellAlignment::Right)
    };
    let tokens = &stats.tokens;
    let side = &stats.subagent_tokens;
    let mut rows = vec![
        (
            "Projects",
            value(ui::plural(stats.projects, "project", "projects"), None),
            if stats.missing == 0 {
                "every folder still exists".to_string()
            } else {
                ui::plural(stats.missing, "missing folder", "missing folders")
            },
        ),
        (
            "Sessions",
            value(ui::plural(stats.sessions, "session", "sessions"), None),
            "top-level chats, subagents not included".to_string(),
        ),
        (
            "Subagents",
            value(
                ui::plural(stats.subagents, "subagent chat", "subagent chats"),
                Some(Color::Magenta),
            ),
            "chats started by an agent".to_string(),
        ),
        (
            "Memory",
            value(
                ui::plural(stats.memory_files, "memory file", "memory files"),
                None,
            ),
            "notes Claude Code keeps per project".to_string(),
        ),
        (
            "Message tokens",
            value(
                format!(
                    "{} in, {} out",
                    ui::count(tokens.input),
                    ui::count(tokens.output)
                ),
                Some(Color::Cyan),
            ),
            format!(
                "plus {} in, {} out in subagent chats",
                ui::count(side.input),
                ui::count(side.output)
            ),
        ),
        (
            "Cached tokens",
            value(
                format!(
                    "{} read, {} written",
                    ui::count(tokens.cache_read),
                    ui::count(tokens.cache_write)
                ),
                Some(Color::Cyan),
            ),
            format!(
                "plus {} read, {} written in subagent chats",
                ui::count(side.cache_read),
                ui::count(side.cache_write)
            ),
        ),
    ];
    if let Some((model, count)) = stats
        .models
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
    {
        rows.push((
            "Top model",
            value(model.clone(), Some(Color::Cyan)),
            ui::plural(*count, "session", "sessions"),
        ));
    }
    rows.push((
        "On disk",
        value(ui::format_size(stats.size), None),
        "transcripts, tool output, and memory".to_string(),
    ));
    for (name, cell, detail) in rows {
        sheet.row(vec![metric(name), cell, dim(theme, detail)]);
    }
    sheet
}

/// Rows of `(name, value)` with their share and a bar, the top ten spelled out.
fn ranked(
    theme: Theme,
    columns: (&str, &str),
    items: &[(String, u64)],
    others: (&str, &str),
    color: Color,
    format: fn(u64) -> String,
    width: Option<usize>,
) -> Sheet {
    let mut sheet = Sheet::new(
        theme,
        &[
            (columns.0, Align::Left),
            (columns.1, Align::Right),
            ("share", Align::Right),
            ("distribution", Align::Left),
        ],
    )
    .flex(0)
    .min_flex(24)
    .optional(&[3])
    .at(width);
    let total: u64 = items.iter().map(|(_, value)| value).sum();
    let rest: u64 = items.iter().skip(TOP).map(|(_, value)| value).sum();
    let max = items
        .iter()
        .take(TOP)
        .map(|(_, value)| *value)
        .chain([rest])
        .max()
        .unwrap_or(0);
    let value = |amount: u64| {
        theme
            .cell(format(amount), None, &[Attribute::Bold])
            .set_alignment(CellAlignment::Right)
    };
    for (name, amount) in items.iter().take(TOP) {
        let full = name.clone();
        sheet.fitted_row(
            vec![
                theme.cell(name, Some(Color::Cyan), &[]),
                value(*amount),
                theme.share_cell(*amount, total),
                theme.bar_cell(*amount, max, color),
            ],
            move |room| {
                theme.cell(
                    ui::layout::shorten_path(&full, room, theme.icons().ellipsis),
                    Some(Color::Cyan),
                    &[],
                )
            },
        );
    }
    if items.len() > TOP {
        sheet.row(vec![
            dim(theme, ui::plural(items.len() - TOP, others.0, others.1)),
            value(rest),
            theme.share_cell(rest, total),
            theme.bar_cell(rest, max, color),
        ]);
    }
    if !sheet.is_empty() {
        sheet.total(vec![
            left(theme, "all"),
            value(total),
            theme.share_cell(total, total),
            left(theme, ""),
        ]);
    }
    sheet
}

fn months(theme: Theme, stats: &Stats) -> Sheet {
    let mut sheet = Sheet::new(
        theme,
        &[
            ("month", Align::Left),
            ("sessions", Align::Right),
            ("share", Align::Right),
            ("distribution", Align::Left),
        ],
    )
    .optional(&[3]);
    let total: u64 = stats.per_month.values().map(|count| *count as u64).sum();
    let max = stats
        .per_month
        .values()
        .map(|count| *count as u64)
        .max()
        .unwrap_or(0);
    for (month, count) in &stats.per_month {
        sheet.row(vec![
            left(theme, month),
            number(theme, *count as u64),
            theme.share_cell(*count as u64, total),
            theme.bar_cell(*count as u64, max, Color::Blue),
        ]);
    }
    if !sheet.is_empty() {
        sheet.total(vec![
            left(
                theme,
                format!(
                    "all {}",
                    ui::plural(stats.per_month.len(), "month", "months")
                ),
            ),
            number(theme, total),
            theme.share_cell(total, total),
            left(theme, ""),
        ]);
    }
    sheet
}

pub fn sections(theme: Theme, stats: &Stats, width: Option<usize>) -> Vec<(&'static str, Sheet)> {
    let mut models: Vec<(String, u64)> = stats
        .models
        .iter()
        .map(|(name, count)| (name.clone(), *count as u64))
        .collect();
    models.sort_by_key(|(name, count)| (Reverse(*count), name.clone()));
    let pairs = |items: &[Named]| -> Vec<(String, u64)> {
        items
            .iter()
            .map(|item| (item.name.clone(), item.value))
            .collect()
    };
    vec![
        ("Claude Code overview", overview(theme, stats)),
        (
            "Claude Code models",
            ranked(
                theme,
                ("model", "sessions"),
                &models,
                ("other model", "other models"),
                Color::Cyan,
                ui::count,
                width,
            ),
        ),
        (
            "Claude Code projects with the most sessions",
            ranked(
                theme,
                ("project", "sessions"),
                &pairs(&stats.busiest),
                ("other project", "other projects"),
                Color::Green,
                ui::count,
                width,
            ),
        ),
        ("Claude Code sessions per month", months(theme, stats)),
        (
            "Largest Claude Code projects",
            ranked(
                theme,
                ("project", "size"),
                &pairs(&stats.largest),
                ("other project", "other projects"),
                Color::Yellow,
                ui::format_size,
                width,
            ),
        ),
    ]
}

pub fn render(theme: Theme, stats: &Stats) -> String {
    render_at(theme, stats, ui::table_width())
}

pub fn render_at(theme: Theme, stats: &Stats, width: Option<usize>) -> String {
    let mut out = String::new();
    for (index, (title, sheet)) in sections(theme, stats, width).into_iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&ui::section_line(theme, title));
        out.push('\n');
        if sheet.is_empty() {
            out.push_str(&ui::hint_line(theme, "none yet"));
            out.push('\n');
        } else {
            out.push_str(&format!("{sheet}\n"));
        }
    }
    out
}
