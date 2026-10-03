//! The one table shape every command renders: named columns, aligned cells, optional total row.
//!
//! A sheet fits the terminal on its own: one flex column absorbs the slack (callers can hand it
//! a shortened cell for the room it gets), optional columns drop out on narrow terminals, and
//! columns that only hold placeholders can collapse away.

use comfy_table::{Attribute, Cell, CellAlignment, ColumnConstraint, ContentArrangement, Width};
use std::fmt::{self, Display};

use super::layout::width;
use super::{Theme, table};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

type Fit = Box<dyn Fn(usize) -> Cell>;

pub struct Sheet {
    theme: Theme,
    headers: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<Cell>>,
    fits: Vec<Option<Fit>>,
    total: Option<Vec<Cell>>,
    flex: Option<usize>,
    min_flex: usize,
    optional: Vec<usize>,
    collapsible: Vec<usize>,
    width: Option<Option<usize>>,
    headless: bool,
}

/// Which columns a sheet shows at one width, and how wide its flex column may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub shown: Vec<bool>,
    /// Content width of the flex column when it has to shrink below its natural width.
    pub room: Option<usize>,
    /// The table cannot fit even after dropping every optional column.
    pub cramped: bool,
}

impl Sheet {
    pub fn new(theme: Theme, columns: &[(&str, Align)]) -> Self {
        Self {
            theme,
            headers: columns.iter().map(|(name, _)| name.to_string()).collect(),
            aligns: columns.iter().map(|(_, align)| *align).collect(),
            rows: Vec::new(),
            fits: Vec::new(),
            total: None,
            flex: None,
            min_flex: MIN_FLEX,
            optional: Vec::new(),
            collapsible: Vec::new(),
            width: None,
            headless: false,
        }
    }

    /// Key-value panels: no header row, the first column already names each value.
    pub fn headless(mut self) -> Self {
        self.headless = true;
        self
    }

    /// The column that grows and shrinks with the terminal.
    pub fn flex(mut self, index: usize) -> Self {
        self.flex = Some(index);
        self
    }

    /// The narrowest the flex column may get before optional columns start to drop.
    pub fn min_flex(mut self, width: usize) -> Self {
        self.min_flex = width;
        self
    }

    /// Columns that may drop out on narrow terminals, least important first.
    pub fn optional(mut self, order: &[usize]) -> Self {
        self.optional = order.to_vec();
        self
    }

    /// Columns that disappear when no row has a real value in them (only blanks or `-`).
    pub fn collapsible(mut self, columns: &[usize]) -> Self {
        self.collapsible = columns.to_vec();
        self
    }

    /// Render for this many terminal columns (`None`: no limit) instead of detecting the width.
    pub fn at(mut self, total: Option<usize>) -> Self {
        self.width = Some(total);
        self
    }

    pub fn row(&mut self, cells: Vec<Cell>) {
        assert_eq!(cells.len(), self.headers.len(), "{:?}", self.headers);
        assert_plain(&cells);
        self.rows.push(cells);
        self.fits.push(None);
    }

    /// A row whose flex cell has a shorter form: `fit(room)` replaces it when it does not fit.
    pub fn fitted_row(&mut self, cells: Vec<Cell>, fit: impl Fn(usize) -> Cell + 'static) {
        self.row(cells);
        if let Some(slot) = self.fits.last_mut() {
            *slot = Some(Box::new(fit));
        }
    }

    pub fn total(&mut self, cells: Vec<Cell>) {
        assert_eq!(cells.len(), self.headers.len(), "{:?}", self.headers);
        assert_plain(&cells);
        let theme = self.theme;
        self.total = Some(
            cells
                .into_iter()
                .map(|cell| {
                    if theme.color() {
                        cell.add_attribute(Attribute::Bold)
                    } else {
                        cell
                    }
                })
                .collect(),
        );
    }

    pub fn headers(&self) -> &[String] {
        &self.headers
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn is_headless(&self) -> bool {
        self.headless
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn column(&self, index: usize) -> Vec<String> {
        self.rows
            .iter()
            .chain(self.total.iter())
            .map(|row| row[index].content())
            .collect()
    }

    fn natural_widths(&self) -> Vec<usize> {
        (0..self.headers.len())
            .map(|index| {
                self.rows
                    .iter()
                    .chain(self.total.iter())
                    .flat_map(|row| row[index].content().lines().map(width).collect::<Vec<_>>())
                    .chain([if self.headless {
                        0
                    } else {
                        width(&self.headers[index])
                    }])
                    .max()
                    .unwrap_or(0)
            })
            .collect()
    }

    fn blank_columns(&self) -> Vec<bool> {
        (0..self.headers.len())
            .map(|index| {
                self.collapsible.contains(&index)
                    && self.rows.iter().all(|row| is_blank(&row[index].content()))
            })
            .collect()
    }

    /// The layout this sheet gets at `total` columns.
    pub fn plan(&self, total: Option<usize>) -> Plan {
        plan(
            &self.natural_widths(),
            &self.blank_columns(),
            self.flex,
            self.min_flex,
            &self.optional,
            total,
        )
    }

    fn target_width(&self) -> Option<usize> {
        self.width.unwrap_or_else(super::table_width)
    }

    /// Content width caps per column: the flex column's room, or, when even dropping every
    /// optional column is not enough, a share of what is left for every text column.
    fn limits(&self, plan: &Plan, total: Option<usize>) -> Vec<Option<usize>> {
        let mut limits = vec![None; self.headers.len()];
        if let Some(flex) = self.flex {
            limits[flex] = plan.room;
        }
        let Some(total) = total.filter(|_| plan.cramped) else {
            return limits;
        };
        let widths = self.natural_widths();
        let shown: Vec<usize> = (0..widths.len())
            .filter(|index| plan.shown[*index])
            .collect();
        let (texts, numbers): (Vec<usize>, Vec<usize>) = shown
            .iter()
            .partition(|index| self.aligns[**index] == Align::Left);
        let fixed: usize =
            numbers.iter().map(|index| widths[*index]).sum::<usize>() + chrome(shown.len());
        let available = total.saturating_sub(fixed);
        let natural: usize = texts
            .iter()
            .map(|index| widths[*index])
            .sum::<usize>()
            .max(1);
        for index in texts {
            let width = widths[index];
            let share = (available * width / natural).max(LAST_RESORT.min(width));
            limits[index] = Some(share.min(width));
        }
        limits
    }

    pub fn render(&self) -> String {
        let total = self.target_width();
        let plan = self.plan(total);
        let limits = self.limits(&plan, total);
        let headers: Vec<&str> = if self.headless {
            Vec::new()
        } else {
            self.headers.iter().map(String::as_str).collect()
        };
        let mut table = table(self.theme, &headers);
        match total {
            Some(total) => {
                table.set_width(u16::try_from(total).unwrap_or(u16::MAX));
            }
            None => {
                table.set_content_arrangement(ContentArrangement::Disabled);
            }
        }
        let flex_limit = self.flex.and_then(|flex| limits[flex]);
        for (row, fit) in self.rows.iter().zip(&self.fits) {
            let mut row = row.clone();
            if let (Some(flex), Some(room), Some(fit)) = (self.flex, flex_limit, fit)
                && row[flex].content().lines().map(width).max().unwrap_or(0) > room
            {
                row[flex] = fit(room);
            }
            table.add_row(row);
        }
        if let Some(cells) = &self.total {
            table.add_row(cells.clone());
        }
        for (index, column) in table.column_iter_mut().enumerate() {
            column.set_cell_alignment(match self.aligns[index] {
                Align::Left => CellAlignment::Left,
                Align::Right => CellAlignment::Right,
            });
            column.set_constraint(match (plan.shown[index], limits[index]) {
                (false, _) => ColumnConstraint::Hidden,
                (true, Some(limit)) => ColumnConstraint::Absolute(Width::Fixed(fixed(limit + 2))),
                (true, None) => ColumnConstraint::ContentWidth,
            });
        }
        table.to_string()
    }
}

impl Display for Sheet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

fn fixed(width: usize) -> u16 {
    u16::try_from(width).unwrap_or(u16::MAX)
}

fn assert_plain(cells: &[Cell]) {
    debug_assert!(
        cells.iter().all(|cell| !cell.content().contains('\u{1b}')),
        "style table cells with Theme::cell, not pre-painted text: comfy-table counts escape codes as width"
    );
}

fn is_blank(text: &str) -> bool {
    matches!(text.trim(), "" | "-")
}

pub const MIN_FLEX: usize = 16;
/// Narrowest a flex column gets once no optional column is left to drop.
const LAST_RESORT: usize = 8;

/// Border and padding cost of `columns` visible columns, excluding their content.
fn chrome(columns: usize) -> usize {
    columns * 2 + columns + 1
}

pub fn plan(
    widths: &[usize],
    blank: &[bool],
    flex: Option<usize>,
    min_flex: usize,
    optional: &[usize],
    total: Option<usize>,
) -> Plan {
    let mut shown: Vec<bool> = blank.iter().map(|blank| !blank).collect();
    let Some(total) = total else {
        return Plan {
            shown,
            room: None,
            cramped: false,
        };
    };
    loop {
        let flex = flex.filter(|index| shown[*index]);
        let columns = shown.iter().filter(|shown| **shown).count();
        let fixed: usize = widths
            .iter()
            .enumerate()
            .filter(|(index, _)| shown[*index] && Some(*index) != flex)
            .map(|(_, width)| *width)
            .sum::<usize>()
            + chrome(columns);
        let room = total.saturating_sub(fixed);
        let (fits, room) = match flex {
            Some(flex) => {
                let natural = widths[flex];
                if room >= natural {
                    (true, None)
                } else {
                    (room >= min_flex.min(natural), Some(room))
                }
            }
            None => (fixed <= total, None),
        };
        if fits {
            return Plan {
                shown,
                room,
                cramped: false,
            };
        }
        if let Some(next) = optional.iter().copied().find(|index| shown[*index]) {
            shown[next] = false;
            continue;
        }
        // Nothing left to drop: the flex column takes what is left, down to a last-resort floor.
        let floor = |flex: usize| LAST_RESORT.min(min_flex).min(widths[flex]);
        let room = flex.map(|flex| room.unwrap_or(0).max(floor(flex)));
        let cramped = match flex {
            Some(flex) => total.saturating_sub(fixed) < floor(flex),
            None => true,
        };
        return Plan {
            shown,
            room,
            cramped,
        };
    }
}

pub fn column_width<'a>(header: &str, cells: impl IntoIterator<Item = &'a str>) -> usize {
    cells
        .into_iter()
        .map(super::layout::width)
        .chain([super::layout::width(header)])
        .max()
        .unwrap_or(0)
}

pub fn is_bare_number(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, ',' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::layout::{strip_ansi, width};
    use comfy_table::Color;

    fn sample(theme: Theme) -> Sheet {
        let mut sheet = Sheet::new(
            theme,
            &[("kind", Align::Left), ("workspaces", Align::Right)],
        );
        sheet.row(vec![theme.cell("folder", None, &[]), Cell::new("52")]);
        sheet.row(vec![theme.cell("unsaved", None, &[]), Cell::new("4")]);
        sheet.total(vec![Cell::new("all kinds"), Cell::new("56")]);
        sheet
    }

    #[test]
    fn renders_a_header_and_aligned_rows() {
        let plain = sample(Theme::plain()).render();
        let lines: Vec<&str> = plain.lines().collect();
        assert!(lines[1].contains("kind") && lines[1].contains("workspaces"));
        assert!(lines.iter().all(|line| width(line) == width(lines[0])));
        assert!(lines[3].ends_with("52 │"));
        assert!(lines[4].ends_with(" 4 │"));
        assert!(plain.contains("all kinds"));
        assert_eq!(strip_ansi(&sample(Theme::colored()).render()), plain);
    }

    #[test]
    fn exposes_columns_for_checks() {
        let sheet = sample(Theme::plain());
        assert_eq!(sheet.headers(), ["kind", "workspaces"]);
        assert_eq!(sheet.column(1), ["52", "4", "56"]);
        assert_eq!(sheet.len(), 2);
        assert!(is_bare_number("1,092"));
        assert!(!is_bare_number("72.2%"));
        assert!(!is_bare_number("4 chats"));
        assert_eq!(column_width("kind", ["● code-workspace", "x"]), 16);
        assert_eq!(column_width("workspaces", ["52"]), 10);
    }

    fn wide(theme: Theme) -> Sheet {
        let mut sheet = Sheet::new(
            theme,
            &[
                ("id", Align::Left),
                ("path", Align::Left),
                ("when", Align::Left),
                ("size", Align::Right),
            ],
        )
        .flex(1)
        .min_flex(12)
        .optional(&[2]);
        for index in 0..3 {
            let long = format!("/very/long/{}/tail-{index}", "x".repeat(60));
            sheet.fitted_row(
                vec![
                    theme.cell(format!("id{index}"), Some(Color::Cyan), &[]),
                    theme.cell(&long, Some(Color::Cyan), &[]),
                    theme.cell("2026-10-01 12:00", None, &[Attribute::Dim]),
                    theme.cell("1.0 MB", None, &[]),
                ],
                move |room| Cell::new(crate::ui::layout::keep_tail(&long, room, "…")),
            );
        }
        sheet
    }

    #[test]
    fn every_line_fits_the_width_and_the_frame_stays_closed() {
        for theme in [Theme::plain(), Theme::colored(), Theme::ascii()] {
            for total in [200, 120, 80, 60, 40, 30] {
                let out = wide(theme).at(Some(total)).render();
                let lines: Vec<&str> = out.lines().collect();
                for line in &lines {
                    assert!(width(line) <= total, "{total}: {line}");
                    assert_eq!(width(line), width(lines[0]), "{total}: {line}");
                }
                let corners = if theme.unicode() {
                    ['╭', '╰']
                } else {
                    ['+', '+']
                };
                assert!(lines[0].starts_with(corners[0]));
                assert!(lines[lines.len() - 1].starts_with(corners[1]));
            }
        }
    }

    #[test]
    fn narrow_terminals_shorten_the_flex_column_then_drop_optional_columns() {
        let at = |total| wide(Theme::plain()).at(Some(total)).render();
        let roomy = at(200);
        assert!(roomy.contains(&"x".repeat(60)));
        assert!(roomy.contains("when"));
        let tight = at(60);
        assert!(tight.contains('…') && tight.contains("tail-2"));
        assert!(tight.contains("when"));
        let narrow = at(40);
        assert!(!narrow.contains("when"), "{narrow}");
        assert!(narrow.contains("tail-2") && narrow.contains("size"));
        let unbounded = wide(Theme::plain()).at(None).render();
        assert!(unbounded.contains(&"x".repeat(60)) && unbounded.contains("when"));
    }

    #[test]
    fn collapsible_columns_vanish_when_every_row_is_blank() {
        let build = |error: &str| {
            let theme = Theme::plain();
            let mut sheet = Sheet::new(theme, &[("id", Align::Left), ("error", Align::Left)])
                .collapsible(&[1])
                .at(None);
            sheet.row(vec![Cell::new("a"), Cell::new("-")]);
            sheet.row(vec![Cell::new("b"), Cell::new(error)]);
            sheet.render()
        };
        let clean = build("");
        assert!(!clean.contains("error"));
        assert!(clean.lines().all(|line| width(line) == width("│ id │")));
        let failed = build("boom");
        assert!(failed.contains("│ error │") && failed.contains("│ boom  │"));
    }

    #[test]
    fn headless_panels_skip_the_header_row() {
        let mut sheet = Sheet::new(
            Theme::plain(),
            &[("field", Align::Left), ("value", Align::Left)],
        )
        .headless()
        .at(None);
        sheet.row(vec![Cell::new("Name"), Cell::new("app")]);
        let out = sheet.render();
        assert!(!out.contains("field"));
        assert_eq!(out.lines().count(), 3);
        assert!(out.contains("│ Name │ app │"));
    }

    #[test]
    fn cramped_tables_share_the_width_between_text_columns() {
        let theme = Theme::plain();
        let mut sheet = Sheet::new(
            theme,
            &[
                ("#", Align::Right),
                ("source", Align::Left),
                ("target", Align::Left),
            ],
        )
        .flex(2)
        .at(Some(80));
        sheet.row(vec![
            Cell::new("1"),
            Cell::new(format!("~/{}/sample-config", "s".repeat(50))),
            Cell::new(format!("~/{}/tiny", "t".repeat(30))),
        ]);
        let out = sheet.render();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.iter().all(|line| width(line) <= 80), "{out}");
        let header = lines[1];
        let source = header.find("source").unwrap();
        let target = header.find("target").unwrap();
        assert!(
            target - source > 30 && width(header) - width(&header[..target]) > 20,
            "{out}"
        );
    }

    #[test]
    fn plans_follow_the_width() {
        let widths = [4, 40, 16, 6];
        let none = [false; 4];
        let roomy = plan(&widths, &none, Some(1), 12, &[2], Some(200));
        assert_eq!(roomy.room, None);
        assert!(roomy.shown.iter().all(|shown| *shown));
        let tight = plan(&widths, &none, Some(1), 12, &[2], Some(60));
        assert_eq!(tight.room, Some(60 - (4 + 16 + 6) - chrome(4)));
        let narrow = plan(&widths, &none, Some(1), 12, &[2], Some(40));
        assert_eq!(narrow.shown, [true, true, false, true]);
        assert_eq!(narrow.room, Some(40 - (4 + 6) - chrome(3)));
        assert!(!narrow.cramped);
        let squeezed = plan(&widths, &none, Some(1), 12, &[2], Some(30));
        assert!(!squeezed.cramped);
        assert_eq!(squeezed.room, Some(30 - (4 + 6) - chrome(3)));
        let hopeless = plan(&widths, &none, Some(1), 12, &[2], Some(20));
        assert!(hopeless.cramped);
        assert_eq!(hopeless.room, Some(LAST_RESORT));
    }

    #[test]
    #[should_panic]
    fn rows_must_match_the_header() {
        let mut sheet = Sheet::new(Theme::plain(), &[("a", Align::Left)]);
        sheet.row(vec![Cell::new("1"), Cell::new("2")]);
    }

    #[test]
    #[should_panic(expected = "pre-painted")]
    fn painted_text_is_refused() {
        let theme = Theme::colored();
        let mut sheet = Sheet::new(theme, &[("a", Align::Left)]);
        sheet.row(vec![Cell::new(theme.bold("1"))]);
    }
}
