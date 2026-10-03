//! Dual progress bars (step and operation), a per-item checklist, and a single-line spinner,
//! drawn on stderr. Every live line carries a ticking `{time}` in the shared duration format.

use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, IsTerminal};
use std::time::{Duration, Instant};

use super::Theme;
use super::signal::LinesGuard;
use std::sync::Mutex;

const UNICODE_TICKS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ";
const ASCII_TICKS: &str = "|/-\\ ";
const UNICODE_BAR: &str = "━╸─";
const ASCII_BAR: &str = "=> ";

fn style(theme: Theme, template: &str) -> ProgressStyle {
    let (ticks, bar) = if theme.unicode() {
        (UNICODE_TICKS, UNICODE_BAR)
    } else {
        (ASCII_TICKS, ASCII_BAR)
    };
    ProgressStyle::with_template(template)
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .with_key(
            "time",
            |state: &ProgressState, out: &mut dyn std::fmt::Write| {
                let _ = out.write_str(&super::elapsed(state.elapsed()));
            },
        )
        .with_key(
            "left",
            |state: &ProgressState, out: &mut dyn std::fmt::Write| {
                let _ = out.write_str(&super::elapsed(state.eta()));
            },
        )
        .tick_chars(ticks)
        .progress_chars(bar)
}

thread_local! {
    /// The live display a [`Checklist`] lends to the work it runs, so bars and warnings that
    /// work creates on this thread draw inside it instead of fighting it for the terminal.
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct Host {
    multi: MultiProgress,
    /// Whether bars and spinners created on this thread join the display.
    nest: bool,
}

/// The display new bars on this thread should join, if one lends itself for that.
fn host() -> Option<MultiProgress> {
    HOST.with(|host| {
        host.borrow()
            .as_ref()
            .filter(|host| host.nest)
            .map(|host| host.multi.clone())
    })
}

/// Prints a stderr line above the live display when one is drawing on this thread.
pub fn eprint_above(line: &str) {
    match HOST.with(|host| host.borrow().as_ref().map(|host| host.multi.clone())) {
        Some(multi) => {
            let _ = multi.println(line);
        }
        None => eprintln!("{line}"),
    }
}

/// Columns a bar line spends outside the bar itself, plus room for a short message.
const BAR_CHROME: usize = 48;
const MAX_BAR: usize = 32;
const MIN_BAR: usize = 6;

/// Bar length for a terminal of `total` columns, so a progress line never wraps (a wrapped line
/// cannot be redrawn in place and leaves copies behind).
pub fn bar_span(total: usize) -> usize {
    total.saturating_sub(BAR_CHROME).clamp(MIN_BAR, MAX_BAR)
}

fn dual_styles(theme: Theme, total: usize) -> (ProgressStyle, ProgressStyle) {
    let span = bar_span(total);
    if theme.color() {
        (
            style(
                theme,
                &format!(
                    "{{spinner:.cyan.bold}} {{prefix:<6.bold.blue}} {{bar:{span}.green/black.bright}} {{pos:>4}}/{{len:<4}} {{time:>6.dim}} {{wide_msg}}"
                ),
            ),
            style(
                theme,
                &format!(
                    "  {{spinner:.magenta}} {{prefix:<4.dim}} {{bar:{span}.cyan/black.bright}} {{pos:>4}}/{{len:<4}} eta {{left:.yellow}} {{wide_msg:.dim}}"
                ),
            ),
        )
    } else {
        (
            style(
                theme,
                &format!(
                    "{{spinner}} {{prefix:<6}} [{{bar:{span}}}] {{pos:>4}}/{{len:<4}} {{time:>6}} {{wide_msg}}"
                ),
            ),
            style(
                theme,
                &format!(
                    "  {{spinner}} {{prefix:<4}} [{{bar:{span}}}] {{pos:>4}}/{{len:<4}} eta {{left}} {{wide_msg}}"
                ),
            ),
        )
    }
}

pub struct DualProgress {
    theme: Theme,
    steps: ProgressBar,
    rows: ProgressBar,
    _multi: Option<MultiProgress>,
    lines: Mutex<Option<LinesGuard>>,
}

/// Lets Ctrl-C erase `lines` drawn lines until the bar is finished.
fn drawn(lines: usize) -> Mutex<Option<LinesGuard>> {
    Mutex::new(Some(LinesGuard::new(lines)))
}

fn release(lines: &Mutex<Option<LinesGuard>>) {
    if let Ok(mut guard) = lines.lock() {
        guard.take();
    }
}

impl DualProgress {
    pub fn new(label: &str, steps: u64, quiet: bool) -> Self {
        let theme = Theme::stderr();
        let hosted = host();
        if hosted.is_none() && (quiet || !io::stderr().is_terminal()) {
            let steps =
                ProgressBar::with_draw_target(Some(steps.max(1)), ProgressDrawTarget::hidden());
            let rows = ProgressBar::with_draw_target(Some(1), ProgressDrawTarget::hidden());
            return Self {
                theme,
                steps,
                rows,
                _multi: None,
                lines: Mutex::new(None),
            };
        }
        let (style_steps, style_rows) = dual_styles(theme, super::term::stderr_width());
        let multi =
            hosted.unwrap_or_else(|| MultiProgress::with_draw_target(ProgressDrawTarget::stderr()));
        let steps_bar = multi.add(ProgressBar::new(steps.max(1)));
        let rows_bar = multi.add(ProgressBar::new(1));
        steps_bar.set_style(style_steps);
        steps_bar.set_prefix(label.to_string());
        rows_bar.set_style(style_rows);
        rows_bar.set_prefix("rows");
        steps_bar.enable_steady_tick(Duration::from_millis(100));
        rows_bar.enable_steady_tick(Duration::from_millis(100));
        Self {
            theme,
            steps: steps_bar,
            rows: rows_bar,
            _multi: Some(multi),
            lines: drawn(2),
        }
    }

    pub fn step(&self, index: u64, message: &str) {
        self.steps.set_position(index);
        let styled = message
            .split(' ')
            .map(|word| self.theme.token(word))
            .collect::<Vec<_>>()
            .join(" ");
        self.steps.set_message(styled);
    }

    pub fn rows(&self, done: u64, total: u64, message: &str) {
        self.rows.set_length(total.max(1));
        self.rows.set_position(done.min(total.max(1)));
        self.rows.set_message(message.to_string());
    }

    pub fn finish(&self) {
        self.steps.finish_and_clear();
        self.rows.finish_and_clear();
        if let Some(multi) = &self._multi {
            multi.remove(&self.steps);
            multi.remove(&self.rows);
        }
        release(&self.lines);
    }
}

/// How a checklist item ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Ok,
    Warn,
    Skip,
    Fail,
}

/// A finished-item line: `✔ label  840ms`, with `⚠`, `↷`/`-` or `✖` for the other marks.
pub fn done_line(theme: Theme, mark: Mark, label: &str, elapsed: Duration) -> String {
    let icons = theme.icons();
    let icon = match mark {
        Mark::Ok => theme.good(icons.check),
        Mark::Warn => theme.caution(icons.warn),
        Mark::Skip => theme.dim(icons.hollow),
        Mark::Fail => theme.paint(icons.cross, owo_colors::Style::new().bold().red()),
    };
    format!("{icon} {label}  {}", theme.dim(super::elapsed(elapsed)))
}

/// Columns a checklist line spends outside its label: glyph, gaps and a timer.
const ITEM_CHROME: usize = 12;

/// One line per item under an overall bar. A started item spins with a live timer; a finished
/// one turns into [`done_line`]. `persist` prints finished lines above the display in item order
/// so they stay in the scrollback; otherwise they stay inside it (oldest dropped once the
/// terminal is full) and vanish with it.
pub struct Checklist {
    theme: Theme,
    labels: Vec<String>,
    persist: bool,
    multi: Option<MultiProgress>,
    overall: ProgressBar,
    window: usize,
    state: Mutex<ChecklistState>,
    _overall_line: Option<LinesGuard>,
}

struct ChecklistState {
    running: Vec<Running>,
    shown: VecDeque<(ProgressBar, LinesGuard)>,
    finished: Vec<Option<(Mark, Duration)>>,
    flushed: usize,
}

struct Running {
    index: usize,
    started: Instant,
    drawn: Option<(ProgressBar, LinesGuard)>,
}

impl Checklist {
    pub fn new(prefix: &str, labels: Vec<String>, quiet: bool, persist: bool) -> Self {
        let theme = Theme::stderr();
        let total = labels.len();
        let state = Mutex::new(ChecklistState {
            running: Vec::new(),
            shown: VecDeque::new(),
            finished: vec![None; total],
            flushed: 0,
        });
        if quiet || !io::stderr().is_terminal() {
            return Self {
                theme,
                labels,
                persist: persist && !quiet,
                multi: None,
                overall: ProgressBar::hidden(),
                window: 0,
                state,
                _overall_line: None,
            };
        }
        let width = super::term::stderr_width();
        let labels = labels
            .into_iter()
            .map(|label| {
                super::layout::keep_head(
                    &label,
                    width.saturating_sub(ITEM_CHROME).max(8),
                    theme.icons().ellipsis,
                )
            })
            .collect();
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let overall = multi.add(ProgressBar::new(total.max(1) as u64));
        overall.set_style(checklist_style(theme, width));
        overall.set_prefix(prefix.to_string());
        overall.enable_steady_tick(TICK);
        let height = console::Term::stderr()
            .size_checked()
            .map_or(24, |(rows, _)| rows as usize);
        Self {
            theme,
            labels,
            persist,
            multi: Some(multi),
            overall,
            window: height.saturating_sub(4).max(3),
            state,
            _overall_line: Some(LinesGuard::new(1)),
        }
    }

    /// Marks item `index` as running: a spinner line with its own timer.
    pub fn start(&self, index: usize) {
        let drawn = self.multi.as_ref().map(|multi| {
            let bar = multi.add(ProgressBar::new_spinner());
            bar.set_style(item_style(self.theme));
            bar.set_message(self.labels[index].clone());
            bar.enable_steady_tick(TICK);
            (bar, LinesGuard::new(1))
        });
        let mut state = self.lock();
        state.running.push(Running {
            index,
            started: Instant::now(),
            drawn,
        });
        self.trim(&mut state);
        self.overall.set_message(running_text(state.running.len()));
    }

    /// Marks item `index` as finished and returns how long it ran (zero if it never started).
    pub fn finish(&self, index: usize, mark: Mark) -> Duration {
        let mut state = self.lock();
        let mut elapsed = Duration::ZERO;
        if let Some(position) = state.running.iter().position(|item| item.index == index) {
            let item = state.running.remove(position);
            elapsed = item.started.elapsed();
            if let Some((bar, line)) = item.drawn {
                if self.persist {
                    bar.finish_and_clear();
                    if let Some(multi) = &self.multi {
                        multi.remove(&bar);
                    }
                } else {
                    bar.set_style(style(self.theme, "{msg}"));
                    bar.finish_with_message(done_line(
                        self.theme,
                        mark,
                        &self.labels[index],
                        elapsed,
                    ));
                    state.shown.push_back((bar, line));
                    self.trim(&mut state);
                }
            }
        }
        if let Some(slot) = state.finished.get_mut(index) {
            *slot = Some((mark, elapsed));
        }
        self.overall.inc(1);
        self.overall.set_message(running_text(state.running.len()));
        if self.persist {
            while let Some(Some((mark, took))) = state.finished.get(state.flushed).copied() {
                let line = done_line(self.theme, mark, &self.labels[state.flushed], took);
                match &self.multi {
                    Some(multi) => {
                        let _ = multi.println(line);
                    }
                    None => eprintln!("{line}"),
                }
                state.flushed += 1;
            }
        }
        elapsed
    }

    /// Runs `work` on this thread with this display lent to it: its warnings print above the
    /// display and, with `nest`, the bars and spinners it creates draw under the running item.
    pub fn hosting<R>(&self, nest: bool, work: impl FnOnce() -> R) -> R {
        let Some(multi) = &self.multi else {
            return work();
        };
        let previous = HOST.with(|host| {
            host.borrow_mut().replace(Host {
                multi: multi.clone(),
                nest,
            })
        });
        let result = work();
        HOST.with(|host| *host.borrow_mut() = previous);
        result
    }

    /// Hides the display while `work` prints to the terminal, then draws it again.
    pub fn suspend<R>(&self, work: impl FnOnce() -> R) -> R {
        match &self.multi {
            Some(multi) => multi.suspend(work),
            None => work(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.overall.elapsed()
    }

    pub fn finish_all(&self) {
        let mut state = self.lock();
        for item in state.running.drain(..) {
            if let Some((bar, _line)) = item.drawn {
                bar.finish_and_clear();
            }
        }
        for (bar, _) in state.shown.drain(..) {
            bar.finish_and_clear();
        }
        self.overall.finish_and_clear();
        if let Some(multi) = &self.multi {
            let _ = multi.clear();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ChecklistState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Drops the oldest finished lines once the display would outgrow the terminal.
    fn trim(&self, state: &mut ChecklistState) {
        while state.shown.len() + state.running.len() > self.window {
            let Some((bar, _line)) = state.shown.pop_front() else {
                break;
            };
            bar.finish_and_clear();
            if let Some(multi) = &self.multi {
                multi.remove(&bar);
            }
        }
    }
}

impl Drop for Checklist {
    fn drop(&mut self) {
        self.finish_all();
    }
}

fn running_text(running: usize) -> String {
    match running {
        0 => String::new(),
        1 => "1 running".to_string(),
        n => format!("{n} running"),
    }
}

const TICK: Duration = Duration::from_millis(80);

fn item_style(theme: Theme) -> ProgressStyle {
    if theme.color() {
        style(theme, "{spinner:.cyan.bold} {msg}  {time:.dim}")
    } else {
        style(theme, "{spinner} {msg}  {time}")
    }
}

fn checklist_style(theme: Theme, total: usize) -> ProgressStyle {
    let span = bar_span(total);
    let template = if theme.color() {
        format!(
            "{{spinner:.yellow.bold}} {{prefix:<6.bold.yellow}} {{bar:{span}.yellow/black.bright}} {{pos:>4}}/{{len:<4}} {{time:>6.dim}} {{wide_msg:.dim}}"
        )
    } else {
        format!(
            "{{spinner}} {{prefix:<6}} [{{bar:{span}}}] {{pos:>4}}/{{len:<4}} {{time:>6}} {{wide_msg}}"
        )
    };
    style(theme, &template)
}

/// Spinner text clipped so the line (glyph, text, elapsed time) stays on one row.
fn spinner_text(message: &str, total: usize, ellipsis: &str) -> String {
    super::layout::keep_head(message, total.saturating_sub(10).max(8), ellipsis)
}

/// A spinner line, and the display it joined when one was lent to this thread.
pub struct Spinner(
    Option<ProgressBar>,
    Option<LinesGuard>,
    Option<MultiProgress>,
);

impl Spinner {
    pub fn set_message(&self, message: &str) {
        if let Some(bar) = &self.0 {
            let theme = Theme::stderr();
            bar.set_message(spinner_text(
                message,
                super::term::stderr_width(),
                theme.icons().ellipsis,
            ));
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(bar) = self.0.take() {
            bar.finish_and_clear();
            if let Some(multi) = self.2.take() {
                multi.remove(&bar);
            }
        }
        self.1.take();
    }
}

pub fn spinner(message: &str, quiet: bool) -> Spinner {
    let hosted = host();
    if hosted.is_none() && (quiet || !io::stderr().is_terminal()) {
        return Spinner(None, None, None);
    }
    let theme = Theme::stderr();
    let template = if theme.color() {
        "{spinner:.cyan.bold} {msg} {time:.dim}"
    } else {
        "{spinner} {msg} {time}"
    };
    let bar = match &hosted {
        Some(multi) => multi.add(ProgressBar::new_spinner()),
        None => ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr()),
    };
    bar.set_style(style(theme, template));
    bar.set_message(spinner_text(
        message,
        super::term::stderr_width(),
        theme.icons().ellipsis,
    ));
    bar.enable_steady_tick(TICK);
    Spinner(Some(bar), Some(LinesGuard::new(1)), hosted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_and_spinner_text_shrink_with_the_terminal() {
        assert_eq!(bar_span(200), MAX_BAR);
        assert_eq!(bar_span(80), 32);
        assert_eq!(bar_span(70), 22);
        assert_eq!(bar_span(40), MIN_BAR);
        let long = "Downloading chatkeep-aarch64-apple-darwin.tar.gz from GitHub releases";
        assert_eq!(spinner_text(long, 200, "…"), long);
        let short = spinner_text(long, 40, "…");
        assert!(crate::ui::layout::width(&short) <= 30 && short.ends_with('…'));
    }

    #[test]
    fn a_hidden_checklist_still_times_each_item() {
        let list = Checklist::new("plan", vec!["a".into(), "b".into()], true, true);
        assert!(!list.persist);
        list.start(1);
        std::thread::sleep(Duration::from_millis(15));
        assert!(list.finish(1, Mark::Ok) >= Duration::from_millis(15));
        assert_eq!(list.finish(0, Mark::Skip), Duration::ZERO);
        assert!(!list.hosting(true, || host().is_some()));
    }

    #[test]
    fn templates_parse_in_every_theme() {
        for theme in [Theme::plain(), Theme::colored(), Theme::ascii()] {
            for total in [200, 80, 60, 40] {
                let (steps, rows) = dual_styles(theme, total);
                let bar = ProgressBar::hidden();
                bar.set_style(steps);
                bar.set_style(rows);
                bar.set_style(checklist_style(theme, total));
                bar.set_style(item_style(theme));
            }
        }
        assert_eq!(
            done_line(
                Theme::plain(),
                Mark::Ok,
                "abc12345 mv a b",
                Duration::from_millis(840)
            ),
            "✔ abc12345 mv a b  840ms"
        );
        assert_eq!(
            done_line(
                Theme::ascii(),
                Mark::Fail,
                "x",
                Duration::from_millis(12_500)
            ),
            "x x  12s"
        );
        let hidden = DualProgress::new("move", 3, true);
        hidden.step(1, "abc12345 /tmp/app");
        hidden.rows(5, 10, "rows");
        hidden.finish();
    }
}
