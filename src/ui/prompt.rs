//! Interactive prompts on stderr, themed to match the status lines.

use owo_colors::Style;
use std::fmt;
use std::io::{self, IsTerminal};

use super::signal::PromptGuard;
use super::{Theme, hinted};

struct ConfirmTheme(Theme);

impl dialoguer::theme::Theme for ConfirmTheme {
    fn format_confirm_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<bool>,
    ) -> fmt::Result {
        let theme = self.0;
        let choices = match default {
            Some(true) => format!("[{}/n]", theme.bold("Y")),
            Some(false) => format!("[y/{}]", theme.bold("N")),
            None => "[y/n]".to_string(),
        };
        write!(
            f,
            "{} {} {} ",
            theme.paint("?", Style::new().bold().yellow()),
            theme.bold(prompt),
            theme.dim(choices)
        )
    }

    fn format_confirm_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: Option<bool>,
    ) -> fmt::Result {
        let theme = self.0;
        let icons = theme.icons();
        match selection {
            Some(true) => write!(
                f,
                "{} {}{}{}",
                theme.good(icons.check),
                theme.bold(prompt),
                theme.sep(),
                theme.good("yes")
            ),
            Some(false) => write!(
                f,
                "{} {}{}{}",
                theme.bad(icons.cross),
                theme.bold(prompt),
                theme.sep(),
                theme.bad("no")
            ),
            None => write!(f, "{} {}", theme.dim("?"), theme.bold(prompt)),
        }
    }
}

fn with_prompt_theme<R>(run: impl FnOnce(&dyn dialoguer::theme::Theme) -> R) -> R {
    let theme = Theme::stderr();
    if theme.color() && theme.unicode() {
        run(&dialoguer::theme::ColorfulTheme::default())
    } else {
        run(&dialoguer::theme::SimpleTheme)
    }
}

/// Picker rows must stay on one terminal row: dialoguer measures a row by its byte length when it
/// redraws, so a longer (or colored) row makes it erase lines above the prompt. `lead` is plain
/// text kept as is; `path` is shortened in the middle until the row fits `total` columns.
pub fn picker_label(lead: &str, path: &str, total: usize, ellipsis: &str) -> String {
    let budget = total.saturating_sub(6);
    let mut max = budget.saturating_sub(lead.len());
    loop {
        let label = format!("{lead}{}", super::layout::shorten_path(path, max, ellipsis));
        if label.len() <= budget || max <= 8 {
            return label;
        }
        max -= 1;
    }
}

fn no_terminal(prompt: &str) -> anyhow::Error {
    let prompt = prompt.trim_end_matches(['?', ':']);
    hinted(
        format!("cannot ask \"{prompt}\" without a terminal"),
        "Pass it on the command line, or run the command in a terminal.",
    )
}

/// Whether prompts can ask on this process's stdin.
pub fn interactive() -> bool {
    io::stdin().is_terminal()
}

pub fn confirm(prompt: &str, yes: bool, interactive: bool) -> anyhow::Result<bool> {
    if yes {
        return Ok(true);
    }
    if !interactive {
        return Err(hinted(prompt, "Re-run with -y to continue."));
    }
    let theme = ConfirmTheme(Theme::stderr());
    let _guard = PromptGuard::new();
    let choice = dialoguer::Confirm::with_theme(&theme)
        .with_prompt(prompt)
        .default(false)
        .interact()?;
    Ok(choice)
}

pub fn multi_select(
    prompt: &str,
    items: &[String],
    interactive: bool,
) -> anyhow::Result<Vec<usize>> {
    if !interactive {
        return Err(no_terminal(prompt));
    }
    let _guard = PromptGuard::new();
    let picked = with_prompt_theme(|theme| {
        dialoguer::MultiSelect::with_theme(theme)
            .with_prompt(prompt)
            .items(items)
            .interact()
    })?;
    Ok(picked)
}

pub fn input(prompt: &str, interactive: bool) -> anyhow::Result<String> {
    if !interactive {
        return Err(no_terminal(prompt));
    }
    let _guard = PromptGuard::new();
    Ok(with_prompt_theme(|theme| {
        dialoguer::Input::with_theme(theme)
            .with_prompt(prompt)
            .interact_text()
    })?)
}

pub fn select(prompt: &str, items: &[&str], interactive: bool) -> anyhow::Result<usize> {
    if !interactive {
        return Err(no_terminal(prompt));
    }
    let _guard = PromptGuard::new();
    Ok(with_prompt_theme(|theme| {
        dialoguer::Select::with_theme(theme)
            .with_prompt(prompt)
            .items(items)
            .default(0)
            .interact()
    })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_labels_fit_one_row_in_bytes() {
        let path = "/Users/me/Machines/upgraded/home/me/projects/workshop/api-demo-isolated";
        assert_eq!(
            picker_label("1f2e3d4c  ", path, 200, "…"),
            format!("1f2e3d4c  {path}")
        );
        for total in [100, 80, 60, 40] {
            let label = picker_label("✖ 1f2e3d4c  workshop  ", path, total, "…");
            assert!(label.len() <= total - 6, "{total}: {label}");
            assert!(label.ends_with("solated"), "{label}");
        }
    }

    #[test]
    fn prompts_without_a_terminal_say_what_they_needed() {
        let err = select("Reindex which workspace?", &["a"], false).unwrap_err();
        let text = crate::ui::error_text(Theme::plain(), &err);
        assert_eq!(
            text,
            "✖ cannot ask \"Reindex which workspace\" without a terminal\n  Pass it on the command line, or run the command in a terminal."
        );
        assert!(input("Destination folder", false).is_err());
        assert!(multi_select("Targets", &["a".to_string()], false).is_err());
        assert!(confirm("Go?", true, false).unwrap());
    }
}
