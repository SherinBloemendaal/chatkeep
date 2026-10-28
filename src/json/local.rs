//! `--json` for what chatkeep keeps itself: the command log and the queue.

use anyhow::Result;
use serde_json::{Value, json};
use std::fs;

use super::{Failed, Part, shape};
use crate::cli::{self, CommonArgs, Outcome, QueueAction, QueueArgs, Tools};
use crate::config;
use crate::engine::{Report, queue};
use crate::queue_cmd::{self, EntryRun};

/// `history`: every line of the command log of the last 30 days, oldest first.
pub fn history(tools: &Tools, common: &CommonArgs) -> Result<Value> {
    let path = match (tools.cursor(common)?, tools.claude(common)) {
        (Some(rt), _) => rt.layout.history_file(),
        (None, Some(claude)) => claude.layout.history_file(),
        (None, None) => config::chatkeep_home()?.join("history.jsonl"),
    };
    let raw = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };
    let mut entries = Vec::new();
    let mut unreadable = 0usize;
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(found) = serde_json::from_str::<Value>(line) else {
            unreadable += 1;
            continue;
        };
        let text = |key: &str| found.get(key).and_then(Value::as_str);
        let args: Vec<&str> = found
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        entries.push(json!({
            "at": text("at").and_then(shape::rfc3339),
            "command": text("command"),
            "args": args,
            "outcome": text("outcome"),
            "duration_ms": found.get("duration_ms").and_then(Value::as_u64),
        }));
    }
    Ok(json!({
        "path": shape::path(&path),
        "entries": entries,
        "unreadable": unreadable,
    }))
}

/// One stored queue entry. `position` counts the pending entries, in run order.
fn entry(found: &queue::Entry, position: Option<usize>) -> Value {
    let tool = queue_cmd::parse_command(&found.argv)
        .ok()
        .and_then(|command| {
            cli::named_tool(&command, &cli::common_of(&command))
                .ok()
                .flatten()
        })
        .map(cli::Tool::flag);
    json!({
        "id": found.id,
        "position": position,
        "status": found.status.as_str(),
        "interrupted": found.status == queue::Status::Failed
            && found.last_error.as_deref() == Some(queue::INTERRUPTED),
        "argv": found.argv,
        "command": queue::shell_join(&found.argv),
        "tool": tool,
        "cwd": shape::path(&found.cwd),
        "profile": found.profile,
        "created_at": shape::rfc3339(&found.created_at),
        "tool_version": found.tool_version,
        "warnings": found.warnings,
        "last_error": found.last_error,
    })
}

fn entries(found: &[queue::Entry]) -> Vec<Value> {
    let mut pending = 0usize;
    found
        .iter()
        .map(|item| {
            let position = (item.status == queue::Status::Pending).then(|| {
                pending += 1;
                pending
            });
            entry(item, position)
        })
        .collect()
}

pub fn queue(tools: &Tools, args: QueueArgs) -> std::result::Result<Vec<Part>, Failed> {
    let (dry_run, yes) = cli::queue_flags(&args.action);
    let session = tools.session(dry_run, yes);
    let home = session.home.clone();
    let done = |data: Value| Ok(vec![Part::of(None, data)]);
    match args.action {
        QueueAction::List(_) => {
            let found = queue::load(&home)?;
            done(json!({"entries": entries(&found)}))
        }
        QueueAction::Add(args) => {
            let (stored, position) = queue_cmd::add_entry(tools, &session, args)?;
            done(json!({"entry": entry(&stored, Some(position))}))
        }
        QueueAction::Rm(args) => {
            let found = queue::load(&home)?;
            let mut report = Report::default();
            for id in &args.ids {
                let Some(item) = found.iter().find(|item| item.id == *id) else {
                    return Err(anyhow::anyhow!("no queue entry {id}").into());
                };
                report
                    .applied
                    .push(format!("remove {id} {}", queue::shell_join(&item.argv)));
            }
            if !dry_run {
                queue::remove(&home, &args.ids)?;
            }
            done(shape::report("Queue remove", &report))
        }
        QueueAction::Clear(_) => {
            let found = queue::load(&home)?;
            let report = Report {
                applied: found
                    .iter()
                    .map(|item| format!("remove {} {}", item.id, queue::shell_join(&item.argv)))
                    .collect(),
                ..Report::default()
            };
            if !dry_run && !found.is_empty() {
                queue::clear_all(&home)?;
            }
            done(shape::report("Queue clear", &report))
        }
        QueueAction::Retry(args) => {
            let found = queue::load(&home)?;
            let retryable = |item: &queue::Entry| {
                matches!(item.status, queue::Status::Failed | queue::Status::Skipped)
            };
            let mut picked: Vec<&queue::Entry> = Vec::new();
            if args.ids.is_empty() {
                picked.extend(found.iter().filter(|item| retryable(item)));
            } else {
                for id in &args.ids {
                    let Some(item) = found.iter().find(|item| item.id == *id) else {
                        return Err(anyhow::anyhow!("no queue entry {id}").into());
                    };
                    if !retryable(item) {
                        return Err(anyhow::anyhow!(
                            "queue entry {id} is {} (only failed or skipped entries can be retried)",
                            item.status.as_str()
                        )
                        .into());
                    }
                    picked.push(item);
                }
            }
            let report = Report {
                applied: picked
                    .iter()
                    .map(|item| format!("retry {} {}", item.id, queue::shell_join(&item.argv)))
                    .collect(),
                ..Report::default()
            };
            if !dry_run {
                let ids = (!args.ids.is_empty()).then_some(args.ids.as_slice());
                queue::retry(&home, ids)?;
            }
            done(shape::report("Queue retry", &report))
        }
        QueueAction::Execute(args) => {
            let mut runs = Vec::new();
            let result = queue_cmd::execute_quietly(tools, &session, &args, &mut runs);
            let data = executed(&runs);
            match result {
                Ok(()) => done(data),
                Err(error) => Err(Failed {
                    error,
                    done: vec![Part::of(None, data)],
                }),
            }
        }
    }
}

/// `queue execute`: the summary report, and per entry its plan, how it ended, and what it did.
fn executed(runs: &[EntryRun]) -> Value {
    let mut report = Report::default();
    for run in runs {
        let line = format!("{} {}", run.entry.id, queue::shell_join(&run.entry.argv));
        if let Ok(warnings) = &run.plan {
            report.warnings.extend(
                warnings
                    .iter()
                    .map(|warning| format!("{}: {warning}", run.entry.id)),
            );
        }
        match run.status {
            "done" | "planned" if run.plan.is_ok() => report.applied.push(line),
            _ => report.skipped.push(line),
        }
    }
    let mut value = shape::report("Queue execute", &report);
    let items: Vec<Value> = runs
        .iter()
        .map(|run| {
            let (plan, warnings, plan_error) = match &run.plan {
                Ok(warnings) if warnings.is_empty() => ("ok", warnings.clone(), None),
                Ok(warnings) => ("warnings", warnings.clone(), None),
                Err(message) => ("skip", Vec::new(), Some(message.clone())),
            };
            json!({
                "id": run.entry.id,
                "argv": run.entry.argv,
                "command": queue::shell_join(&run.entry.argv),
                "plan": plan,
                "plan_warnings": warnings,
                "plan_error": plan_error,
                "status": run.status,
                "error": run.error,
                "results": run.outcomes.iter().filter_map(|outcome| match outcome {
                    Outcome::Finished { dry_run, title, report } => {
                        let mut found = shape::report(title, report);
                        if let Some(object) = found.as_object_mut() {
                            object.insert("dry_run".to_string(), json!(dry_run));
                        }
                        Some(found)
                    }
                    Outcome::Text { .. } | Outcome::Standalone => None,
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    if let Some(object) = value.as_object_mut() {
        object.insert("entries".to_string(), Value::Array(items));
    }
    value
}
