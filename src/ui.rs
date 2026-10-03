//! Terminal presentation: brew-style status lines, themed tables, charts, prompts, and progress.

mod banner;
mod chart;
mod help;
pub mod layout;
mod progress;
mod prompt;
mod sheet;
pub mod signal;
pub mod term;

pub use chart::{BAR_SPAN, blocks, share};
pub use help::{command_help, help_text, print_help};
pub use layout::Grid;
pub use progress::{Checklist, DualProgress, Mark, Spinner, spinner};
pub use prompt::{confirm, input, interactive, multi_select, picker_label, select};
pub use sheet::{Align, Plan, Sheet, column_width, is_bare_number};
pub use term::{ColorMode, Depth};

use chrono::{Local, TimeZone};
use comfy_table::modifiers::UTF8_ROUND_CORNERS;
use comfy_table::presets::{ASCII_FULL_CONDENSED, UTF8_FULL_CONDENSED};
use comfy_table::{
    Attribute, Cell, CellAlignment, Color, ContentArrangement, Table, TableComponent,
};
use owo_colors::{OwoColorize, Style};
use std::fmt::{self, Display};
use std::io::{self, IsTerminal};

const KIB: u64 = 1024;
const MIB: u64 = KIB * 1024;
const GIB: u64 = MIB * 1024;
const TIB: u64 = GIB * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icons {
    pub check: &'static str,
    pub cross: &'static str,
    pub warn: &'static str,
    pub info: &'static str,
    pub arrow: &'static str,
    pub bullet: &'static str,
    pub diamond: &'static str,
    pub pointer: &'static str,
    pub hollow: &'static str,
    pub square: &'static str,
    pub sep: &'static str,
    pub ellipsis: &'static str,
    pub header: &'static str,
}

pub const UNICODE: Icons = Icons {
    check: "✔",
    cross: "✖",
    warn: "⚠",
    info: "ℹ",
    arrow: "➜",
    bullet: "●",
    diamond: "◆",
    pointer: "▸",
    hollow: "○",
    square: "■",
    sep: " · ",
    ellipsis: "…",
    header: "==>",
};

pub const ASCII: Icons = Icons {
    check: "+",
    cross: "x",
    warn: "!",
    info: "i",
    arrow: "->",
    bullet: "*",
    diamond: "*",
    pointer: ">",
    hollow: "-",
    square: "#",
    sep: " | ",
    ellipsis: "...",
    header: "==>",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    color: bool,
    depth: Depth,
    unicode: bool,
}

impl Theme {
    pub const fn plain() -> Self {
        Self {
            color: false,
            depth: Depth::Basic,
            unicode: true,
        }
    }

    pub const fn colored() -> Self {
        Self {
            color: true,
            depth: Depth::Basic,
            unicode: true,
        }
    }

    pub const fn ascii() -> Self {
        Self {
            color: false,
            depth: Depth::Basic,
            unicode: false,
        }
    }

    pub const fn with_depth(self, depth: Depth) -> Self {
        Self { depth, ..self }
    }

    pub fn stdout() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stdout,
            depth: settings.depth,
            unicode: settings.unicode,
        }
    }

    pub fn stderr() -> Self {
        let settings = term::settings();
        Self {
            color: settings.stderr,
            depth: settings.depth,
            unicode: settings.unicode,
        }
    }

    pub fn color(self) -> bool {
        self.color
    }

    pub fn depth(self) -> Depth {
        self.depth
    }

    pub fn unicode(self) -> bool {
        self.unicode
    }

    pub fn icons(self) -> &'static Icons {
        if self.unicode { &UNICODE } else { &ASCII }
    }

    pub fn sep(self) -> String {
        self.dim(self.icons().sep)
    }

    pub fn paint(self, text: impl Display, style: Style) -> String {
        if self.color {
            text.style(style).to_string()
        } else {
            text.to_string()
        }
    }

    pub fn bold(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold())
    }

    pub fn dim(self, text: impl Display) -> String {
        self.paint(text, Style::new().dimmed())
    }

    pub fn heading(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold().blue())
    }

    pub fn path(self, text: impl Display) -> String {
        self.paint(text, Style::new().cyan())
    }

    pub fn id(self, text: impl Display) -> String {
        self.paint(text, Style::new().magenta())
    }

    pub fn good(self, text: impl Display) -> String {
        self.paint(text, Style::new().green())
    }

    pub fn bad(self, text: impl Display) -> String {
        self.paint(text, Style::new().red())
    }

    pub fn caution(self, text: impl Display) -> String {
        self.paint(text, Style::new().yellow())
    }

    pub fn command(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold().cyan())
    }

    pub fn flag(self, text: impl Display) -> String {
        self.paint(text, Style::new().yellow())
    }

    pub fn number(self, text: impl Display) -> String {
        self.paint(text, Style::new().bold())
    }

    pub fn arrow(self) -> String {
        self.dim(self.icons().arrow)
    }

    pub fn token(self, text: &str) -> String {
        match classify(text) {
            Token::Path => self.path(text),
            Token::Id => self.id(text),
            Token::Plain => text.to_string(),
        }
    }

    pub fn size(self, bytes: u64) -> String {
        self.paint(format_size(bytes), magnitude(bytes).style())
    }

    pub fn cell(self, text: impl Display, color: Option<Color>, attributes: &[Attribute]) -> Cell {
        let mut cell = Cell::new(text);
        if self.color {
            if let Some(color) = color {
                cell = cell.fg(color);
            }
            for attribute in attributes {
                cell = cell.add_attribute(*attribute);
            }
        }
        cell
    }

    pub fn token_cell(self, text: &str) -> Cell {
        match classify(text) {
            Token::Path => self.cell(home_relative(text), Some(Color::Cyan), &[]),
            Token::Id => self.cell(text, Some(Color::Magenta), &[]),
            Token::Plain => Cell::new(text),
        }
    }

    pub fn size_cell(self, bytes: u64) -> Cell {
        let (color, attributes): (Option<Color>, &[Attribute]) = match magnitude(bytes) {
            Magnitude::Small => (None, &[Attribute::Dim]),
            Magnitude::Medium => (None, &[]),
            Magnitude::Large => (Some(Color::Yellow), &[]),
            Magnitude::Huge => (Some(Color::Red), &[Attribute::Bold]),
        };
        self.cell(format_size(bytes), color, attributes)
            .set_alignment(CellAlignment::Right)
    }

    pub fn count_cell(self, value: usize, color: Option<Color>) -> Cell {
        let cell = if value == 0 {
            self.cell(value, None, &[Attribute::Dim])
        } else {
            self.cell(count(value as u64), color, &[Attribute::Bold])
        };
        cell.set_alignment(CellAlignment::Right)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Path,
    Id,
    Plain,
}

pub fn classify(text: &str) -> Token {
    let is_path = text.starts_with('/')
        || text.starts_with("~/")
        || text.starts_with("./")
        || text.contains('\\')
        || text.ends_with(".chatkeep")
        || text.ends_with(".crepath")
        || text.ends_with(".code-workspace");
    if is_path {
        return Token::Path;
    }
    let hexish = text.len() >= 8
        && text.chars().any(|ch| ch.is_ascii_digit())
        && text.chars().all(|ch| ch.is_ascii_hexdigit() || ch == '-');
    if hexish || (!text.is_empty() && text.chars().all(|ch| ch.is_ascii_digit())) {
        return Token::Id;
    }
    Token::Plain
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Magnitude {
    Small,
    Medium,
    Large,
    Huge,
}

impl Magnitude {
    fn style(self) -> Style {
        match self {
            Self::Small => Style::new().dimmed(),
            Self::Medium => Style::new(),
            Self::Large => Style::new().yellow(),
            Self::Huge => Style::new().bold().red(),
        }
    }
}

pub fn magnitude(bytes: u64) -> Magnitude {
    if bytes < MIB {
        Magnitude::Small
    } else if bytes < 25 * MIB {
        Magnitude::Medium
    } else if bytes < 100 * MIB {
        Magnitude::Large
    } else {
        Magnitude::Huge
    }
}

pub fn format_size(bytes: u64) -> String {
    if bytes >= TIB {
        format!("{:.1} TB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn plural(value: usize, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        count(value as u64),
        if value == 1 { one } else { many }
    )
}

pub fn datetime(millis: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(millis)
        .single()
        .map(|stamp| stamp.format("%Y-%m-%d %H:%M").to_string())
}

pub fn ago(millis: i64, now_millis: i64) -> String {
    let seconds = (now_millis - millis).max(0) / 1000;
    match seconds {
        0..60 => "just now".to_string(),
        60..3_600 => format!("{}m ago", seconds / 60),
        3_600..86_400 => format!("{}h ago", seconds / 3_600),
        86_400..2_592_000 => format!("{}d ago", seconds / 86_400),
        2_592_000..31_536_000 => format!("{}mo ago", seconds / 2_592_000),
        _ => format!("{}y ago", seconds / 31_536_000),
    }
}

/// `340ms`, `4.2s`, `38s`, `2m 05s`, `1h 02m`: precise while short, coarser as it grows.
pub fn duration(millis: u64) -> String {
    if millis < 1_000 {
        format!("{millis}ms")
    } else if millis < 10_000 {
        format!("{:.1}s", (millis / 100) as f64 / 10.0)
    } else if millis < 60_000 {
        format!("{}s", millis / 1_000)
    } else if millis < 3_600_000 {
        format!("{}m {:02}s", millis / 60_000, (millis % 60_000) / 1_000)
    } else {
        format!(
            "{}h {:02}m",
            millis / 3_600_000,
            (millis % 3_600_000) / 60_000
        )
    }
}

pub fn elapsed(elapsed: std::time::Duration) -> String {
    duration(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

pub fn home_relative(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if !home.is_empty()
            && let Some(rest) = path.strip_prefix(home.as_ref())
            && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

/// A dim-separated summary under a table (`3 entries · 1 pending`), wrapped between parts so a
/// narrow terminal never splits one part. Each line is indented two spaces and ends in `\n`.
pub fn summary(theme: Theme, parts: &[String]) -> String {
    summary_at(theme, parts, table_width())
}

pub fn summary_at(theme: Theme, parts: &[String], total: Option<usize>) -> String {
    let sep = theme.sep();
    let sep_width = layout::width(&sep);
    let mut out = String::new();
    let mut used = 0;
    for part in parts {
        let part_width = layout::width(part);
        if used > 0 && total.is_some_and(|total| 2 + used + sep_width + part_width > total) {
            out.push('\n');
            used = 0;
        }
        if used == 0 {
            out.push_str("  ");
        } else {
            out.push_str(&sep);
            used += sep_width;
        }
        out.push_str(part);
        used += part_width;
    }
    out.push('\n');
    out
}

pub fn section_line(theme: Theme, title: &str) -> String {
    format!(
        "{} {}",
        theme.heading(theme.icons().header),
        theme.bold(title)
    )
}

pub fn ok_line(theme: Theme, message: &str) -> String {
    let icon = theme.good(theme.icons().check);
    status(&icon, message, str::to_string, display_width())
}

pub fn warn_line(theme: Theme, message: &str) -> String {
    let icon = theme.caution(theme.icons().warn);
    status(&icon, message, str::to_string, display_width())
}

pub fn err_line(theme: Theme, message: &str) -> String {
    let icon = theme.paint(theme.icons().cross, Style::new().bold().red());
    status(
        &icon,
        message,
        |line| theme.paint(line, Style::new().bold().red()),
        display_width(),
    )
}

pub fn info_line(theme: Theme, message: &str) -> String {
    let icon = theme.paint(theme.icons().info, Style::new().blue());
    status(&icon, message, str::to_string, display_width())
}

/// Hints often carry a command to copy, so they are never wrapped.
pub fn hint_line(theme: Theme, message: &str) -> String {
    format!("  {}", theme.dim(message))
}

/// `icon message`, with a long single-line message wrapped under itself to fit `total`. Words
/// are never split, so paths stay whole and copyable.
pub fn status(
    icon: &str,
    message: &str,
    paint: impl Fn(&str) -> String,
    total: Option<usize>,
) -> String {
    let lead = layout::width(icon) + 1;
    let lines = match total.map(|total| total.saturating_sub(lead).max(24)) {
        Some(room) if !message.contains('\n') && layout::width(message) > room => {
            layout::wrap_words(message, room)
        }
        _ => vec![message.to_string()],
    };
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                format!("{icon} {}", paint(line))
            } else {
                format!("{}{}", " ".repeat(lead), paint(line))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The width status lines wrap to: the terminal's, or none when neither stream is one.
pub fn display_width() -> Option<usize> {
    if stdout_is_tty() {
        Some(term::width())
    } else if io::stderr().is_terminal() {
        Some(term::stderr_width())
    } else {
        None
    }
}

pub fn section(title: &str) {
    println!("{}", section_line(Theme::stdout(), title));
}

pub fn ok(message: &str) {
    println!("{}", ok_line(Theme::stdout(), message));
}

pub fn warn(message: &str) {
    progress::eprint_above(&warn_line(Theme::stderr(), message));
}

pub fn info(message: &str) {
    println!("{}", info_line(Theme::stdout(), message));
}

pub fn hint(message: &str) {
    println!("{}", hint_line(Theme::stdout(), message));
}

pub fn err(message: &str) {
    eprintln!("{}", err_line(Theme::stderr(), message));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub dry: bool,
    pub verb: String,
    pub subject: String,
    pub target: Option<String>,
}

pub fn applied_parts(text: &str) -> Applied {
    let (dry, rest) = match text.strip_prefix("dry-run") {
        Some(rest) => (true, rest.trim_start()),
        None => (false, text),
    };
    let (left, target) = match rest.split_once(" -> ") {
        Some((left, right)) => (left, Some(right.to_string())),
        None => (rest, None),
    };
    let (verb, subject) = split_subject(left);
    Applied {
        dry,
        verb: verb.trim().to_string(),
        subject: subject.trim().to_string(),
        target,
    }
}

pub fn results(
    theme: Theme,
    action: &str,
    dry_run: bool,
    applied: &[String],
    skipped: &[String],
) -> Sheet {
    let icons = theme.icons();
    let mut sheet = Sheet::new(
        theme,
        &[
            ("result", Align::Left),
            ("action", Align::Left),
            ("item", Align::Left),
            ("target", Align::Left),
        ],
    )
    .flex(3)
    .collapsible(&[3]);
    let item = |text: &str| {
        if text.is_empty() {
            theme.cell("-", None, &[Attribute::Dim])
        } else {
            theme.token_cell(text)
        }
    };
    for text in applied {
        let parts = applied_parts(text);
        let result = if parts.dry || dry_run {
            theme.cell(
                format!("{} planned", icons.hollow),
                Some(Color::Yellow),
                &[Attribute::Bold],
            )
        } else {
            theme.cell(
                format!("{} done", icons.check),
                Some(Color::Green),
                &[Attribute::Bold],
            )
        };
        let verb = if parts.verb.is_empty() {
            action.to_string()
        } else {
            parts.verb.clone()
        };
        sheet.row(vec![
            result,
            theme.cell(verb, None, &[Attribute::Bold]),
            item(&parts.subject),
            item(parts.target.as_deref().unwrap_or("")),
        ]);
    }
    for id in skipped {
        sheet.row(vec![
            theme.cell(
                format!("{} skipped", icons.warn),
                Some(Color::Yellow),
                &[Attribute::Bold],
            ),
            theme.cell(action, None, &[Attribute::Dim]),
            item(id),
            item(""),
        ]);
    }
    sheet
}

fn split_subject(text: &str) -> (&str, &str) {
    let mut start = 0;
    for word in text.split(' ') {
        if classify(word) != Token::Plain {
            return (&text[..start], &text[start..]);
        }
        start += word.len() + 1;
    }
    (text, "")
}

#[derive(Debug)]
pub struct Hinted {
    pub message: String,
    pub hint: String,
}

impl Display for Hinted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let joiner = if self.message.ends_with(['.', '?', '!']) {
            " "
        } else {
            ". "
        };
        write!(f, "{}{joiner}{}", self.message, self.hint)
    }
}

impl std::error::Error for Hinted {}

pub fn hinted(message: impl Into<String>, hint: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Hinted {
        message: message.into(),
        hint: hint.into(),
    })
}

pub fn error_text(theme: Theme, err: &anyhow::Error) -> String {
    let mut chain = err.chain();
    let headline = chain
        .next()
        .map(|head| match head.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => head.to_string(),
        })
        .unwrap_or_default();
    let mut out = err_line(theme, &headline);
    for cause in chain {
        let text = match cause.downcast_ref::<Hinted>() {
            Some(hinted) => hinted.message.clone(),
            None => cause.to_string(),
        };
        out.push('\n');
        out.push_str(&hint_line(
            theme,
            &format!("{} {text}", theme.icons().arrow),
        ));
    }
    if let Some(hinted) = err.chain().find_map(|cause| cause.downcast_ref::<Hinted>()) {
        out.push('\n');
        out.push_str(&hint_line(theme, &hinted.hint));
    }
    out
}

pub fn report_error(err: &anyhow::Error) {
    eprintln!("{}", error_text(Theme::stderr(), err));
}

pub fn table(theme: Theme, headers: &[&str]) -> Table {
    let mut table = Table::new();
    if theme.unicode {
        table
            .load_preset(UTF8_FULL_CONDENSED)
            .apply_modifier(UTF8_ROUND_CORNERS)
            .set_style(TableComponent::VerticalLines, '│')
            .set_style(TableComponent::HeaderLines, '─')
            .set_style(TableComponent::LeftHeaderIntersection, '├')
            .set_style(TableComponent::MiddleHeaderIntersections, '┼')
            .set_style(TableComponent::RightHeaderIntersection, '┤');
    } else {
        table.load_preset(ASCII_FULL_CONDENSED);
    }
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if let Some(total) = table_width() {
        table.set_width(u16::try_from(total).unwrap_or(u16::MAX));
    }
    if theme.color {
        table.enforce_styling();
    }
    if !headers.is_empty() {
        table.set_header(
            headers
                .iter()
                .map(|header| theme.cell(header, Some(Color::Cyan), &[Attribute::Bold])),
        );
    }
    table
}

pub fn panel(theme: Theme, rows: Vec<(&str, Cell)>) -> Sheet {
    let mut sheet = Sheet::new(theme, &[("field", Align::Left), ("value", Align::Left)])
        .flex(1)
        .headless();
    for (key, value) in rows {
        sheet.row(vec![
            theme.cell(key, Some(Color::Blue), &[Attribute::Bold]),
            value.set_alignment(CellAlignment::Left),
        ]);
    }
    sheet
}

pub fn validation(
    theme: Theme,
    title: &str,
    headers: &[&str],
    rows: &[Vec<String>],
    warnings: &[String],
) -> String {
    let columns: Vec<(&str, Align)> = std::iter::once(("#", Align::Right))
        .chain(headers.iter().map(|header| (*header, Align::Left)))
        .collect();
    let mut sheet = Sheet::new(theme, &columns).flex(columns.len() - 1);
    for (index, row) in rows.iter().enumerate() {
        sheet.row(
            std::iter::once(theme.cell(index + 1, None, &[Attribute::Dim]))
                .chain(row.iter().map(|text| theme.token_cell(text)))
                .collect(),
        );
    }
    let mut out = format!("{}\n{sheet}\n", section_line(theme, title));
    let mut summary = vec![plural(rows.len(), "item", "items")];
    if !warnings.is_empty() {
        summary.push(plural(warnings.len(), "warning", "warnings"));
    }
    out.push_str(&hint_line(theme, &summary.join(theme.icons().sep)));
    out.push('\n');
    for warning in warnings {
        out.push_str(&warn_line(theme, warning));
        out.push('\n');
    }
    out
}

pub fn stdout_is_tty() -> bool {
    io::stdout().is_terminal()
}

pub fn table_width() -> Option<usize> {
    stdout_is_tty().then(term::width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_emits_no_escape_codes() {
        let theme = Theme::plain();
        assert_eq!(theme.heading("x"), "x");
        assert_eq!(theme.size(2 * GIB), "2.0 GB");
        assert_eq!(section_line(theme, "Title"), "==> Title");
        assert_eq!(ok_line(theme, "done"), "✔ done");
        assert_eq!(warn_line(theme, "careful"), "⚠ careful");
        assert_eq!(err_line(theme, "broke"), "✖ broke");
        assert_eq!(info_line(theme, "note"), "ℹ note");
        assert!(Theme::colored().heading("x").contains('\u{1b}'));
    }

    #[test]
    fn ascii_theme_falls_back_to_plain_glyphs() {
        let theme = Theme::ascii();
        assert_eq!(ok_line(theme, "done"), "+ done");
        assert_eq!(warn_line(theme, "careful"), "! careful");
        assert_eq!(err_line(theme, "broke"), "x broke");
        let out = results(
            theme,
            "move",
            false,
            &["abc12345 -> def67890".to_string()],
            &[],
        )
        .render();
        assert!(out.is_ascii());
        assert!(out.contains("| + done | move   | abc12345 | def67890 |"));
        let mut table = table(theme, &["a"]);
        table.add_row(vec!["b"]);
        assert!(table.to_string().is_ascii());
    }

    #[test]
    fn sizes_and_counts_are_humanized() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(3 * MIB), "3.0 MB");
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1,000");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(magnitude(10 * KIB), Magnitude::Small);
        assert_eq!(magnitude(5 * MIB), Magnitude::Medium);
        assert_eq!(magnitude(50 * MIB), Magnitude::Large);
        assert_eq!(magnitude(200 * MIB), Magnitude::Huge);
    }

    #[test]
    fn tokens_are_classified() {
        assert_eq!(classify("/Users/me/app"), Token::Path);
        assert_eq!(classify("backup.chatkeep"), Token::Path);
        assert_eq!(classify("backup.crepath"), Token::Path);
        assert_eq!(classify("abcdef0123456789abcdef0123456789"), Token::Id);
        assert_eq!(classify("1700000000003"), Token::Id);
        assert_eq!(classify("remove"), Token::Plain);
        assert_eq!(classify("empty-window"), Token::Plain);
    }

    #[test]
    fn applied_results_split_into_columns() {
        let parts = |dry, verb: &str, subject: &str, target: Option<&str>| Applied {
            dry,
            verb: verb.to_string(),
            subject: subject.to_string(),
            target: target.map(str::to_string),
        };
        assert_eq!(
            applied_parts("abc12345 -> def67890"),
            parts(false, "", "abc12345", Some("def67890"))
        );
        assert_eq!(
            applied_parts("dry-run abc12345 -> /tmp/app"),
            parts(true, "", "abc12345", Some("/tmp/app"))
        );
        assert_eq!(
            applied_parts("dry-run remove"),
            parts(true, "remove", "", None)
        );
        assert_eq!(
            applied_parts("dry-run split abc12345"),
            parts(true, "split", "abc12345", None)
        );
        assert_eq!(
            applied_parts("/tmp/My Docs/a.chatkeep"),
            parts(false, "", "/tmp/My Docs/a.chatkeep", None)
        );
        let sheet = results(
            Theme::plain(),
            "move",
            false,
            &[
                "dry-run abc12345 -> /tmp/app".to_string(),
                "dry-run remove".to_string(),
            ],
            &["def67890".to_string()],
        );
        assert_eq!(sheet.headers(), ["result", "action", "item", "target"]);
        let out = sheet.render();
        assert!(out.contains("│ ○ planned │ move   │ abc12345 │ /tmp/app │"));
        assert!(out.contains("│ ○ planned │ remove │ -        │ -        │"));
        assert!(out.contains("│ ⚠ skipped │ move   │ def67890 │ -        │"));
    }

    #[test]
    fn dry_runs_never_report_done() {
        let out = results(
            Theme::plain(),
            "combine",
            true,
            &["fedcba98".to_string(), "dry-run copy 2 chats".to_string()],
            &[],
        )
        .render();
        assert!(!out.contains("done"), "{out}");
        assert_eq!(out.matches("planned").count(), 2);
    }

    #[test]
    fn hinted_errors_keep_their_text_and_split_the_hint() {
        let err = hinted("Cursor is running.", "Close Cursor completely and retry.");
        assert_eq!(
            err.to_string(),
            "Cursor is running. Close Cursor completely and retry."
        );
        let rendered = error_text(Theme::plain(), &err);
        assert_eq!(
            rendered,
            "✖ Cursor is running.\n  Close Cursor completely and retry."
        );
        let joined = hinted("warnings need confirmation", "Pass -y to continue.");
        assert_eq!(
            joined.to_string(),
            "warnings need confirmation. Pass -y to continue."
        );
        let wrapped = anyhow::anyhow!("disk full").context("failed to write");
        assert_eq!(
            error_text(Theme::plain(), &wrapped),
            "✖ failed to write\n  ➜ disk full"
        );
    }

    #[test]
    fn validation_lists_rows_and_warnings() {
        let rows = vec![vec!["abc12345".to_string(), "/tmp/app".to_string()]];
        let out = validation(
            Theme::plain(),
            "Move plan",
            &["workspace", "destination"],
            &rows,
            &["skipped x".to_string()],
        );
        assert!(out.starts_with("==> Move plan\n"));
        assert!(out.contains("│ # │ workspace │ destination │"));
        assert!(out.contains("/tmp/app"));
        assert!(out.contains("1 item · 1 warning"));
        assert!(out.contains("⚠ skipped x"));
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn long_status_lines_wrap_under_their_text() {
        let message = "skipped fedcba9876543210fedcba9876543210: destination missing /tmp/a/very/long/path/that/goes/on";
        let out = status("⚠", message, str::to_string, Some(60));
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() > 1);
        assert!(lines[0].starts_with("⚠ skipped"));
        assert!(lines[1..].iter().all(|line| line.starts_with("  ")));
        assert!(lines.iter().all(|line| layout::width(line) <= 60), "{out}");
        assert!(out.contains("/tmp/a/very/long/path/that/goes/on"));
        assert_eq!(
            status("⚠", message, str::to_string, None),
            format!("⚠ {message}")
        );
        assert_eq!(status("⚠", "short", str::to_string, Some(60)), "⚠ short");
    }

    #[test]
    fn summaries_wrap_between_parts() {
        let parts: Vec<String> = [
            "12 workspaces",
            "no missing destinations",
            "34 chats, 8 subagents",
        ]
        .iter()
        .map(|part| part.to_string())
        .collect();
        let theme = Theme::plain();
        assert_eq!(
            summary_at(theme, &parts, None),
            "  12 workspaces · no missing destinations · 34 chats, 8 subagents\n"
        );
        assert_eq!(
            summary_at(theme, &parts, Some(45)),
            "  12 workspaces · no missing destinations\n  34 chats, 8 subagents\n"
        );
        for total in [20, 30, 60] {
            for line in summary_at(theme, &parts, Some(total)).lines() {
                assert!(layout::width(line) <= total.max(2 + 23), "{line}");
            }
        }
    }

    #[test]
    fn relative_ages() {
        let now = 10_000_000_000;
        assert_eq!(ago(now - 5_000, now), "just now");
        assert_eq!(ago(now - 120_000, now), "2m ago");
        assert_eq!(ago(now - 7_200_000, now), "2h ago");
        assert_eq!(ago(now - 3 * 86_400_000, now), "3d ago");
        assert_eq!(duration(340), "340ms");
        assert_eq!(duration(1_500), "1.5s");
        assert_eq!(duration(9_999), "9.9s");
        assert_eq!(duration(38_400), "38s");
        assert_eq!(duration(125_000), "2m 05s");
        assert_eq!(duration(3_725_000), "1h 02m");
    }
}
