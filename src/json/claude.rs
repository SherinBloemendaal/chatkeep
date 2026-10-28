//! The Claude Code side of `--json`.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::PathBuf;

use super::{finished, settle, shape, split_value};
use crate::claude::identity::{Book, Seen};
use crate::claude::stats::Tokens;
use crate::claude::store::{self, Project, Session};
use crate::claude::{
    self, accounts, autosync, doctor, identity, index, stats, sync, transfer, view,
};
use crate::claude_cmd;
use crate::cli::{
    self, AccountsAction, ClaudeCacheAction, ClaudeCommand, Command, SplitArgs, SyncAction,
    SyncAutoAction,
};
use crate::engine::Report;

pub fn perform(rt: &claude::Runtime, command: Command) -> Result<Value> {
    match command {
        Command::Ls(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            match args.id {
                Some(id) => detail(&rt, &id),
                None => list(&rt),
            }
        }
        Command::Stats(args) => {
            let rt = if args.fresh { rt.fresh() } else { rt.clone() };
            Ok(stats_value(&stats::collect(&rt)?))
        }
        Command::Split(args) => split(rt, args),
        Command::Claude(cli::ClaudeArgs { command }) => match command {
            ClaudeCommand::Accounts(cli::AccountsArgs {
                action: AccountsAction::Ls,
            }) => account_list(rt),
            ClaudeCommand::Accounts(cli::AccountsArgs {
                action: AccountsAction::Doctor,
            }) => checkup(rt),
            ClaudeCommand::Cache(cli::ClaudeCacheArgs {
                action: ClaudeCacheAction::Stats,
            }) => cache_stats(rt),
            ClaudeCommand::Sync(cli::SyncArgs {
                action: Some(SyncAction::Profiles),
                ..
            }) => profiles(rt),
            ClaudeCommand::Sync(cli::SyncArgs {
                action: Some(SyncAction::Log(args)),
                ..
            }) => log(rt, args.last),
            ClaudeCommand::Sync(cli::SyncArgs {
                action: Some(SyncAction::Auto(args)),
                ..
            }) => auto(rt, &args.action),
            command => finished(claude_cmd::perform(
                rt,
                Command::Claude(cli::ClaudeArgs { command }),
            )?),
        },
        command => finished(claude_cmd::perform(rt, command)?),
    }
}

fn project(project: &Project) -> Value {
    let destination = match &project.path {
        None => "none",
        Some(_) if project.destination_missing() => "missing",
        Some(_) => "present",
    };
    json!({
        "tool": "claude",
        "id": project.slug,
        "kind": "project",
        "name": view::name(project),
        "path": project.path.as_deref().map(shape::path),
        "uri": Value::Null,
        "storage_dir": shape::path(&project.dir),
        "install": Value::Null,
        "profile": Value::Null,
        "profile_label": Value::Null,
        "chats": project.sessions.len(),
        "subagents": project.subagents(),
        "archived": Value::Null,
        "size_bytes": project.size,
        "last_active": project
            .sessions
            .iter()
            .filter_map(|session| session.modified)
            .max()
            .map(shape::system_time),
        "destination": destination,
        "destination_missing": project.destination_missing(),
        "memory_files": project.memory_files,
    })
}

fn list(rt: &claude::Runtime) -> Result<Value> {
    let mut projects = store::discover(rt)?;
    view::sort(&mut projects);
    Ok(json!({
        "index_as_of": Value::Null,
        "index_refreshing": false,
        "projects": projects.iter().map(project).collect::<Vec<_>>(),
    }))
}

fn tokens(tokens: &Tokens) -> Value {
    json!({
        "input": tokens.input,
        "output": tokens.output,
        "cache_write": tokens.cache_write,
        "cache_read": tokens.cache_read,
    })
}

fn session(found: &Project, session: &Session) -> Result<Value> {
    let usage = match &session.facts {
        Some(facts) => facts.usage.clone(),
        None => stats::read_usage(&session.transcript)?,
    };
    Ok(json!({
        "id": session.id,
        "title": store::title(found, session),
        "created_at": Value::Null,
        "updated_at": session.modified.map(shape::system_time),
        "archived": Value::Null,
        "subagent": false,
        "model": usage.model(),
        "tokens": tokens(&usage.tokens),
        "subagents": session.subagents,
        "size_bytes": session.size,
        "cwd": session.cwd,
        "transcript": shape::path(&session.transcript),
    }))
}

fn detail(rt: &claude::Runtime, id: &str) -> Result<Value> {
    let projects = store::discover(rt)?;
    let found = store::find(&projects, rt, id)
        .with_context(|| format!("no Claude Code project matches {id}"))?;
    let chats = found
        .sessions
        .iter()
        .map(|item| session(found, item))
        .collect::<Result<Vec<_>>>()?;
    let last_active = found
        .sessions
        .iter()
        .filter_map(|session| session.modified)
        .max()
        .map(shape::system_time);
    Ok(json!({
        "index_as_of": Value::Null,
        "index_refreshing": false,
        "details": [{
            "project": project(found),
            "last_active": last_active,
            "chats": chats,
        }],
    }))
}

fn stats_value(found: &stats::Stats) -> Value {
    let named = |items: &[stats::Named], unit: &str| -> Vec<Value> {
        items
            .iter()
            .map(|item| json!({"name": item.name, unit: item.value}))
            .collect()
    };
    json!({
        "projects": found.projects,
        "missing": found.missing,
        "sessions": found.sessions,
        "subagents": found.subagents,
        "memory_files": found.memory_files,
        "size_bytes": found.size,
        "tokens": tokens(&found.tokens),
        "subagent_tokens": tokens(&found.subagent_tokens),
        "models": shape::counted(&found.models, "model", "sessions"),
        "per_month": found.per_month.iter().map(|(month, count)| json!({
            "month": month,
            "sessions": count,
        })).collect::<Vec<_>>(),
        "busiest": named(&found.busiest, "sessions"),
        "largest": named(&found.largest, "size_bytes"),
    })
}

fn split(rt: &claude::Runtime, args: SplitArgs) -> Result<Value> {
    let source = args.source.context("split needs the project to split")?;
    let targets: Vec<PathBuf> = args.targets.iter().map(PathBuf::from).collect();
    let projects = store::discover(rt)?;
    let found = store::find(&projects, rt, &source)
        .with_context(|| format!("no Claude Code project matches {source}"))?;
    let suggested = transfer::suggest_split(rt, found, &targets)?;
    let suggestion = cli::with_assignments(suggested.clone(), &args.assign, &targets, |path| {
        rt.resolve_path(path)
    })?;
    let (assigned, warnings) = settle(&suggestion, rt.dry_run)?;
    let mut report = transfer::split(rt, found, &targets, &assigned, args.move_chats)?;
    report.warnings.splice(0..0, warnings);
    Ok(split_value(
        "Split (Claude Code)",
        &report,
        &suggested,
        &assigned,
        |path| rt.resolve_path(path),
    ))
}

/// Who an account is, as far as chatkeep knows.
fn person(book: &Book, account: &str) -> Value {
    json!({
        "account": account,
        "label": book.label(account),
        "name": book.name(account),
        "email": book.email(account),
    })
}

fn account_list(rt: &claude::Runtime) -> Result<Value> {
    let listings = accounts::listings(&rt.layout)?;
    let known = accounts::known_sessions(rt)?;
    let book = identity::book(rt)?;
    Ok(json!({
        "accounts": listings.iter().map(|listing| {
            let mut value = person(&book, &listing.account);
            if let Some(object) = value.as_object_mut() {
                object.insert("organization".to_string(), json!(listing.organization));
                object.insert("member".to_string(), json!(listing.label()));
                object.insert("chats".to_string(), json!(listing.sessions.len()));
                object.insert("on_disk".to_string(), json!(accounts::on_disk(listing, &known)));
                object.insert("signed_in".to_string(), json!(listing.current));
                object.insert("dir".to_string(), json!(shape::path(&listing.dir)));
            }
            value
        }).collect::<Vec<_>>(),
    }))
}

fn seen(book: &Book, found: Option<&Seen>) -> Value {
    let Some(found) = found else {
        return Value::Null;
    };
    json!({
        "account": found.account,
        "label": book.label(&found.account),
        "email": found.email,
        "organization": found.organization,
        "organization_name": found.organization_name,
    })
}

fn checkup(rt: &claude::Runtime) -> Result<Value> {
    let found = doctor::checkup(rt, doctor::cli_on_path())?;
    let book = identity::book(rt)?;
    let desktop = found.desktop.as_deref().map(|account| {
        let mut value = person(&book, account);
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "organizations".to_string(),
                json!(found.desktop_organizations),
            );
        }
        value
    });
    Ok(json!({
        "desktop_app": found.desktop_app,
        "desktop": desktop,
        "session": seen(&book, found.session.as_ref()),
        "login": seen(&book, found.login.as_ref()),
        "claude_command": found.cli.as_deref().map(shape::path),
        "stale_caches": found.stale,
        "findings": doctor::conclusions(&found, &book)
            .into_iter()
            .map(|(level, text)| json!({"level": level, "text": text}))
            .collect::<Vec<_>>(),
    }))
}

fn cache_stats(rt: &claude::Runtime) -> Result<Value> {
    let home = &rt.layout.chatkeep_home;
    let path = index::db_path(home);
    let size: u64 = index::files(home)
        .iter()
        .filter_map(|file| std::fs::metadata(file).ok())
        .map(|meta| meta.len())
        .sum();
    let counts = if rt.cache {
        index::Cache::open_existing(home)?
            .map(|cache| cache.counts())
            .transpose()?
    } else {
        None
    };
    Ok(json!({
        "enabled": rt.cache,
        "exists": path.exists(),
        "path": shape::path(&path),
        "size_bytes": size,
        "transcripts": counts.as_ref().map(|counts| counts.transcripts),
        "refreshed_at": counts.as_ref().and_then(|counts| counts.refreshed_at).and_then(shape::millis),
        "schema": counts.as_ref().map(|counts| counts.schema),
    }))
}

/// Whether the background sync is set up and running.
fn auto_status(rt: &claude::Runtime) -> Result<Value> {
    let installed = autosync::installed()?;
    let running = autosync::running()?;
    let state = match (installed, running) {
        (true, true) => "on",
        (true, false) => "stopped",
        (false, _) => "off",
    };
    Ok(json!({
        "state": state,
        "installed": installed,
        "running": running,
        "log_path": shape::path(&sync::log_path(&rt.layout.chatkeep_home)),
    }))
}

fn profiles(rt: &claude::Runtime) -> Result<Value> {
    let config = sync::load(&rt.layout.chatkeep_home)?;
    let signed_in = accounts::current(&rt.layout);
    let book = identity::book(rt)?;
    let profiles: Vec<Value> = config
        .profiles
        .iter()
        .map(|(name, profile)| {
            let members: Vec<Value> = profile
                .accounts
                .iter()
                .map(|member| {
                    let (account, organization) = match member.split_once('/') {
                        Some((account, organization)) => (account, Some(organization)),
                        None => (member.as_str(), None),
                    };
                    let mut value = person(&book, account);
                    if let Some(object) = value.as_object_mut() {
                        object.insert("organization".to_string(), json!(organization));
                        object.insert("member".to_string(), json!(member));
                        object.insert(
                            "signed_in".to_string(),
                            json!(
                                signed_in
                                    .as_deref()
                                    .is_some_and(|current| member.starts_with(current))
                            ),
                        );
                    }
                    value
                })
                .collect();
            json!({"name": name, "accounts": members})
        })
        .collect();
    Ok(json!({
        "profiles": profiles,
        "auto": auto_status(rt)?,
    }))
}

fn log(rt: &claude::Runtime, last: usize) -> Result<Value> {
    let mut rows = sync::read_log(&rt.layout.chatkeep_home)?;
    let skip = rows.len().saturating_sub(last.max(1));
    let rows = rows.split_off(skip);
    let text = |value: &String| (!value.is_empty()).then(|| value.clone());
    Ok(json!({
        "entries": rows.iter().map(|entry| json!({
            "at": shape::rfc3339(&entry.at),
            "profile": text(&entry.profile),
            "action": entry.action,
            "chat": text(&entry.chat),
            "title": text(&entry.title),
            "account": text(&entry.account),
            "source": entry.source,
            "changed": entry.changed,
            "times": entry.times,
        })).collect::<Vec<_>>(),
    }))
}

fn auto(rt: &claude::Runtime, action: &SyncAutoAction) -> Result<Value> {
    let mut report = Report::default();
    let title = match action {
        SyncAutoAction::Status => return auto_status(rt),
        SyncAutoAction::On => {
            let config = sync::load(&rt.layout.chatkeep_home)?;
            sync::selected(&config, None)?;
            let path = autosync::enable()?;
            report
                .applied
                .push(format!("background sync is on ({})", shape::path(&path)));
            "Background sync on"
        }
        SyncAutoAction::Off => {
            if autosync::disable()? {
                report.applied.push("background sync is off".to_string());
            } else {
                report
                    .skipped
                    .push("background sync was not on".to_string());
            }
            "Background sync off"
        }
    };
    let mut value = shape::report(title, &report);
    if let Some(object) = value.as_object_mut() {
        object.insert("auto".to_string(), auto_status(rt)?);
    }
    Ok(value)
}
