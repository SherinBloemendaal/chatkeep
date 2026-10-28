//! How values look in the JSON output: times in ISO 8601 UTC, paths absolute, sizes in bytes.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::engine::Report;

/// Milliseconds since the epoch as `2026-10-28T09:30:00Z`.
pub fn millis(value: i64) -> Option<String> {
    Utc.timestamp_millis_opt(value).single().map(stamp)
}

pub fn system_time(value: SystemTime) -> String {
    stamp(DateTime::<Utc>::from(value))
}

/// An RFC 3339 time in any offset, in UTC. Text that is no such time is `None`.
pub fn rfc3339(value: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|parsed| stamp(parsed.with_timezone(&Utc)))
}

fn stamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn path(value: &Path) -> String {
    value.to_string_lossy().into_owned()
}

/// A path the text output shortened to `~/...`, absolute again.
pub fn expanded(value: &str) -> String {
    let rest = match value.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => rest,
        _ => return value.to_string(),
    };
    match dirs::home_dir() {
        Some(home) => {
            let mut full: PathBuf = home;
            let trimmed = rest.trim_start_matches(['/', '\\']);
            if !trimmed.is_empty() {
                full.push(trimmed);
            }
            path(&full)
        }
        None => value.to_string(),
    }
}

/// A write command's result: its title and the report fields.
pub fn report(title: &str, report: &Report) -> Value {
    json!({
        "title": title,
        "warnings": report.warnings,
        "skipped": report.skipped,
        "applied": report.applied,
        "rewritten_keys": report.rewritten_keys,
    })
}

/// A count map as rows sorted by count, largest first, then by name.
pub fn counted<'a>(
    items: impl IntoIterator<Item = (&'a String, &'a usize)>,
    key: &str,
    unit: &str,
) -> Vec<Value> {
    let mut rows: Vec<(&String, usize)> = items.into_iter().map(|(name, n)| (name, *n)).collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    rows.into_iter()
        .map(|(name, count)| json!({ key: name, unit: count }))
        .collect()
}
