//! The Cursor side of `--json`.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::cmp::Reverse;
use std::path::PathBuf;

use super::{finished, settle, shape, split_value};
use crate::cli::{self, CacheAction, Command, CursorCommand, SplitArgs};
use crate::engine::facts::ChatFacts;
use crate::engine::index::Note;
use crate::engine::stats::Stats;
use crate::engine::{self, Kind, ListRow, Runtime, cache};

pub fn perform(rt: &Runtime, command: Command) -> Result<Value> {
    match command {
        Command::Ls(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            match args.id {
                Some(id) => detail(&rt, args.common.unsaved, &id),
                None => list(&rt, args.common.unsaved),
            }
        }
        Command::Stats(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            let (found, note) = engine::stats_noted(&rt)?;
            Ok(stats(&rt, &found, note.as_ref()))
        }
        Command::Split(args) => split(rt, args),
        Command::Cursor(cli::CursorArgs {
            command:
                CursorCommand::Cache(cli::CacheArgs {
                    action: CacheAction::Scan(args),
                }),
        }) => Ok(if rt.dry_run {
            stale(rt, &cache::collect(rt)?)
        } else {
            scan(&cache::scan(rt, args.full)?)
        }),
        Command::Cursor(cli::CursorArgs {
            command:
                CursorCommand::Cache(cli::CacheArgs {
                    action: CacheAction::Stats(_),
                }),
        }) => Ok(cache_stats(rt, &cache::collect(rt)?)),
        command => finished(cli::perform(rt, command)?),
    }
}

/// When the rows were read, if they came from the index, and whether a refresh runs now.
fn noted(note: Option<&Note>) -> (Value, bool) {
    match note {
        Some(note) => (json!(shape::millis(note.as_of)), note.background),
        None => (Value::Null, false),
    }
}

fn list(rt: &Runtime, unsaved: bool) -> Result<Value> {
    let listing = engine::workspace_rows_noted(rt, unsaved)?;
    let (as_of, refreshing) = noted(listing.note.as_ref());
    let projects: Vec<Value> = listing
        .rows
        .iter()
        .map(|row| project(row, listing.last_active(row)))
        .collect();
    Ok(json!({
        "index_as_of": as_of,
        "index_refreshing": refreshing,
        "projects": projects,
    }))
}

fn detail(rt: &Runtime, unsaved: bool, id: &str) -> Result<Value> {
    let (details, note) = engine::workspace_details_noted(rt, unsaved, id, true)?;
    let (as_of, refreshing) = noted(note.as_ref());
    let details: Vec<Value> = details
        .iter()
        .map(|detail| {
            let mut chats: Vec<&ChatFacts> = detail.chats.iter().collect();
            chats.sort_by_key(|chat| {
                (
                    Reverse(chat.last_updated_at.unwrap_or(i64::MIN)),
                    chat.id.clone(),
                )
            });
            let last_active = detail
                .chats
                .iter()
                .filter_map(|chat| chat.last_updated_at.or(chat.created_at))
                .max();
            json!({
                "project": project(&detail.row, last_active),
                "last_active": last_active.and_then(shape::millis),
                "chats": chats.into_iter().map(chat).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({
        "index_as_of": as_of,
        "index_refreshing": refreshing,
        "details": details,
    }))
}

/// `present`, `missing`, or `none` (a kind with no folder of its own).
fn destination(row: &ListRow) -> &'static str {
    match row.workspace.kind {
        Kind::Unsaved | Kind::EmptyWindow | Kind::Remote => "none",
        Kind::Folder | Kind::CodeWorkspace if row.workspace.destination_missing => "missing",
        Kind::Folder | Kind::CodeWorkspace => "present",
    }
}

/// A project row; `last_active` is the newest update of any of its chats (or its creation, for a
/// chat Cursor recorded no update for), in milliseconds.
fn project(row: &ListRow, last_active: Option<i64>) -> Value {
    let workspace = &row.workspace;
    json!({
        "tool": "cursor",
        "id": workspace.id,
        "kind": workspace.kind.label(),
        "name": engine::workspace_name(workspace),
        "path": workspace.path.as_deref().map(shape::path),
        "uri": workspace.uri,
        "storage_dir": shape::path(&workspace.dir),
        "install": workspace.install,
        "profile": workspace.profile,
        "profile_label": workspace.profile_label(),
        "chats": row.chats,
        "subagents": row.subagents,
        "archived": row.archived,
        "size_bytes": row.size,
        "last_active": last_active.and_then(shape::millis),
        "destination": destination(row),
        "destination_missing": workspace.destination_missing,
    })
}

fn chat(chat: &ChatFacts) -> Value {
    let usage = chat.usage.as_ref();
    json!({
        "id": chat.id,
        "title": chat.title,
        "created_at": chat.created_at.and_then(shape::millis),
        "updated_at": chat.last_updated_at.and_then(shape::millis),
        "archived": chat.is_archived,
        "subagent": chat.is_subagent,
        "model": usage.and_then(|usage| usage.model.clone()),
        "tokens": usage.map(|usage| json!({
            "input": usage.input_tokens,
            "output": usage.output_tokens,
            "context": usage.context_tokens,
        })),
        "cost_cents": usage.map(|usage| usage.cost_cents),
        "requests": usage.map(|usage| usage.requests),
    })
}

/// The user data folder of installation `name`, absolute.
fn install_root(rt: &Runtime, name: &str, shown: &str) -> String {
    rt.installs
        .iter()
        .chain(std::iter::once(&rt.layout))
        .find(|layout| layout.name == name)
        .map(|layout| shape::path(&layout.cursor_root))
        .unwrap_or_else(|| shape::expanded(shown))
}

fn stats(rt: &Runtime, found: &Stats, note: Option<&Note>) -> Value {
    let (as_of, refreshing) = noted(note);
    let usage = &found.usage;
    let named = |items: &[engine::stats::Named], unit: &str| -> Vec<Value> {
        items
            .iter()
            .map(|item| {
                json!({
                    "name": item.name,
                    "kind": item.kind.as_ref().map(Kind::label),
                    unit: item.value,
                })
            })
            .collect()
    };
    json!({
        "index_as_of": as_of,
        "index_refreshing": refreshing,
        "installations": found.installations,
        "user_profiles": found.user_profiles,
        "profiles": found.profiles.iter().map(|row| {
            let install = row.name.split('/').next().unwrap_or(&row.name);
            json!({
                "name": row.name,
                "root": install_root(rt, install, &row.root),
                "workspaces": row.workspaces,
                "chats": row.chats,
                "subagents": row.subagents,
                "global_db_bytes": row.global_db,
                "user_profile": row.user_profile,
            })
        }).collect::<Vec<_>>(),
        "workspaces": found.workspaces,
        "by_kind": found.by_kind.iter().map(|(kind, count)| json!({
            "kind": kind.label(),
            "workspaces": count,
        })).collect::<Vec<_>>(),
        "chats": found.chats,
        "subagents": found.subagents,
        "archived": found.archived,
        "usage": {
            "conversations": usage.conversations,
            "cost_cents": usage.cost_cents,
            "requests": usage.requests,
            "cost_chats": usage.cost_chats,
            "context_tokens": usage.context_tokens,
            "context_chats": usage.context_chats,
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
            "token_chats": usage.token_chats,
        },
        "models": shape::counted(&found.models, "model", "chats"),
        "per_month": found.per_month.iter().map(|(month, count)| json!({
            "month": month,
            "chats": count,
        })).collect::<Vec<_>>(),
        "busiest": named(&found.busiest, "chats"),
        "global_db_bytes": found.global_db,
        "workspace_dbs": named(&found.workspace_dbs, "bytes"),
    })
}

fn split(rt: &Runtime, args: SplitArgs) -> Result<Value> {
    let (rt, source) = cli::one_target(rt, args.source, "Split which workspace?")?;
    let targets: Vec<PathBuf> = args.targets.iter().map(PathBuf::from).collect();
    let workspace = engine::find_workspace(&rt, &source)?
        .with_context(|| format!("no workspace matches {source}"))?;
    let suggested = engine::suggest_split(&rt, &workspace, &targets)?;
    let suggestion = cli::with_assignments(suggested.clone(), &args.assign, &targets, |path| {
        rt.resolve_path(path)
    })?;
    let (assigned, warnings) = settle(&suggestion, rt.dry_run)?;
    let mut report = engine::split_workspace(&rt, &source, &targets, &assigned, args.move_chats)?;
    report.warnings.splice(0..0, warnings);
    Ok(split_value(
        "Split",
        &report,
        &suggested,
        &assigned,
        |path| rt.resolve_path(path),
    ))
}

fn holder(found: Option<&engine::index::Holder>) -> Value {
    match found {
        Some(holder) => json!({
            "pid": holder.pid,
            "started_at": shape::millis(holder.started),
            "alive": holder.alive,
        }),
        None => Value::Null,
    }
}

fn cache_stats(rt: &Runtime, found: &cache::CacheStats) -> Value {
    json!({
        "path": shape::path(&found.path),
        "exists": found.exists,
        "size_bytes": found.size,
        "schema": found.version,
        "refreshed_at": found.refreshed_at.and_then(shape::millis),
        "spawned_at": found.spawned_at.and_then(shape::millis),
        "refresh": holder(found.holder.as_ref()),
        "last_log": found.log,
        "installs": found.installs.iter().map(|row| {
            let root = match (&row.scan, row.found) {
                (_, true) => install_root(rt, &row.name, &row.root),
                (Some(scan), false) => scan.root.clone(),
                (None, false) => shape::expanded(&row.root),
            };
            json!({
                "name": row.name,
                "root": root,
                "found": row.found,
                "scanned_at": row.scan.as_ref().and_then(|scan| shape::millis(scan.scanned_at)),
                "scan_duration_ms": row.scan.as_ref().map(|scan| scan.duration_ms),
                "dirty_at": row.scan.as_ref().and_then(|scan| scan.dirty_at).and_then(shape::millis),
                "workspaces": row.counts.workspaces,
                "chats": row.counts.chats,
                "subagents": row.counts.subagents,
                "stale": row.stale.as_ref().map(|stale| json!({
                    "sources": stale.sources,
                    "reasons": stale.reasons,
                })),
            })
        }).collect::<Vec<_>>(),
    })
}

/// `cursor cache scan -n`: what a scan would read again, per installation.
fn stale(rt: &Runtime, found: &cache::CacheStats) -> Value {
    json!({
        "installs": found.installs.iter().filter(|row| row.found).map(|row| json!({
            "name": row.name,
            "root": install_root(rt, &row.name, &row.root),
            "scanned": row.scan.is_some(),
            "stale_sources": row.stale.as_ref().map(|stale| stale.sources),
            "reasons": row.stale.as_ref().map(|stale| stale.reasons.clone()).unwrap_or_default(),
        })).collect::<Vec<_>>(),
    })
}

fn scan(found: &cache::ScanResult) -> Value {
    json!({
        "full": found.full,
        "duration_ms": found.duration_ms,
        "path": shape::path(&found.path),
        "installs": found.outcomes.iter().map(|outcome| json!({
            "name": outcome.install,
            "workspaces": outcome.workspaces,
            "chats": outcome.chats,
            "subagents": outcome.subagents,
            "changed_sources": outcome.changed,
            "chats_read": outcome.chats_read,
            "duration_ms": outcome.duration_ms,
        })).collect::<Vec<_>>(),
    })
}
