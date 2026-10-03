//! A cache of what every Claude Code transcript says, in `claude-index.db`.
//!
//! Claude Code keeps plain files, so the cache only has to remember one thing per
//! transcript: the folder it runs in, its title, and its usage, keyed by the file's size and
//! modification time. Every read refreshes the rows whose file changed and drops the rows
//! whose file is gone, so it is never out of date and needs no background refresh.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use super::stats::Usage;

pub const SCHEMA: i64 = 1;

const TABLES: &str = r#"
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE transcripts (
    path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    cwd TEXT,
    title TEXT,
    month TEXT,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_write INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    models TEXT NOT NULL
);
"#;

pub fn db_path(home: &Path) -> PathBuf {
    home.join("claude-index.db")
}

/// The files SQLite keeps for the cache.
pub fn files(home: &Path) -> Vec<PathBuf> {
    let db = db_path(home);
    let mut found = vec![db.clone()];
    for suffix in ["-wal", "-shm"] {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        found.push(PathBuf::from(name));
    }
    found.into_iter().filter(|path| path.exists()).collect()
}

/// What one transcript says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    /// The folder the session started in: the `cwd` of its first line that has one.
    pub cwd: Option<String>,
    /// The last custom title it was given.
    pub title: Option<String>,
    pub usage: Usage,
}

/// Size and modification time, the key that tells a changed file from an unchanged one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub mtime_ms: i64,
}

pub fn stamp(path: &Path) -> Result<Stamp> {
    let meta = fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        });
    Ok(Stamp {
        size: meta.len(),
        mtime_ms,
    })
}

/// Everything a transcript says, in one pass over its lines. Claude Code writes one line per
/// part of an answer and repeats the token counts on each, so every answer is counted once,
/// by its message id.
pub fn read_facts(transcript: &Path) -> Result<Facts> {
    let file = fs::File::open(transcript)
        .with_context(|| format!("failed to open {}", transcript.display()))?;
    let mut facts = Facts::default();
    let mut counted = HashSet::new();
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else {
            break;
        };
        let wanted = line.contains("\"usage\"")
            || line.contains("\"custom-title\"")
            || (facts.cwd.is_none() && line.contains("\"cwd\""))
            || (facts.usage.month.is_none() && line.contains("\"timestamp\""));
        if !wanted {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if facts.cwd.is_none()
            && let Some(cwd) = json.get("cwd").and_then(Value::as_str)
        {
            facts.cwd = Some(cwd.to_string());
        }
        if json.get("type").and_then(Value::as_str) == Some("custom-title")
            && let Some(title) = json.get("customTitle").and_then(Value::as_str)
        {
            facts.title = Some(title.to_string());
        }
        if facts.usage.month.is_none()
            && let Some(stamp) = json.get("timestamp").and_then(Value::as_str)
            && let Some(month) = stamp.get(..7)
        {
            facts.usage.month = Some(month.to_string());
        }
        let Some(message) = json.get("message") else {
            continue;
        };
        let Some(counts) = message.get("usage") else {
            continue;
        };
        if let Some(id) = message.get("id").and_then(Value::as_str)
            && !counted.insert(id.to_string())
        {
            continue;
        }
        let field = |key: &str| counts.get(key).and_then(Value::as_u64).unwrap_or(0);
        facts.usage.tokens.input += field("input_tokens");
        facts.usage.tokens.output += field("output_tokens");
        facts.usage.tokens.cache_write += field("cache_creation_input_tokens");
        facts.usage.tokens.cache_read += field("cache_read_input_tokens");
        if let Some(model) = message.get("model").and_then(Value::as_str)
            && !model.starts_with('<')
        {
            *facts.usage.models.entry(model.to_string()).or_default() += 1;
        }
    }
    Ok(facts)
}

/// What the cache knows about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counts {
    pub transcripts: usize,
    pub refreshed_at: Option<i64>,
    pub schema: i64,
}

pub struct Cache {
    conn: Connection,
    path: PathBuf,
    /// Paths looked up since the cache was opened; `prune` drops every other row.
    seen: std::cell::RefCell<HashSet<String>>,
    /// Re-read every transcript, whether it changed or not.
    fresh: bool,
}

impl Cache {
    /// Open or create the cache, rebuilding it when the schema version differs.
    pub fn open(home: &Path, fresh: bool) -> Result<Self> {
        fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;
        let path = db_path(home);
        let conn = Connection::open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        let cache = Self {
            conn,
            path,
            seen: std::cell::RefCell::new(HashSet::new()),
            fresh,
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    pub fn open_existing(home: &Path) -> Result<Option<Self>> {
        if !db_path(home).exists() {
            return Ok(None);
        }
        Self::open(home, false).map(Some)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn version(&self) -> Result<i64> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }

    fn ensure_schema(&self) -> Result<()> {
        if self.version()? == SCHEMA {
            return Ok(());
        }
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let done = (|| -> Result<()> {
            let tables: Vec<String> = self
                .conn
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                )?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            for table in tables {
                self.conn
                    .execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))?;
            }
            self.conn.execute_batch(TABLES)?;
            self.conn.pragma_update(None, "user_version", SCHEMA)?;
            Ok(())
        })();
        match done {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(err) => {
                self.conn.execute_batch("ROLLBACK").ok();
                return Err(err);
            }
        }
        Ok(())
    }

    /// The facts of `transcript`: from the cache when the file did not change, otherwise
    /// read now and remembered.
    pub fn facts(&self, transcript: &Path) -> Result<Facts> {
        let key = transcript.to_string_lossy().into_owned();
        self.seen.borrow_mut().insert(key.clone());
        let stamp = stamp(transcript)?;
        if !self.fresh
            && let Some(found) = self.lookup(&key, stamp)?
        {
            return Ok(found);
        }
        let facts = read_facts(transcript)?;
        self.store(&key, stamp, &facts)?;
        Ok(facts)
    }

    fn lookup(&self, key: &str, stamp: Stamp) -> Result<Option<Facts>> {
        let row = self
            .conn
            .query_row(
                "SELECT cwd, title, month, input, output, cache_write, cache_read, models
                 FROM transcripts WHERE path = ?1 AND size = ?2 AND mtime = ?3",
                params![key, stamp.size as i64, stamp.mtime_ms],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((cwd, title, month, input, output, cache_write, cache_read, models)) = row else {
            return Ok(None);
        };
        let models: BTreeMap<String, usize> = serde_json::from_str(&models).unwrap_or_default();
        Ok(Some(Facts {
            cwd,
            title,
            usage: Usage {
                tokens: super::stats::Tokens {
                    input: input.max(0) as u64,
                    output: output.max(0) as u64,
                    cache_write: cache_write.max(0) as u64,
                    cache_read: cache_read.max(0) as u64,
                },
                models,
                month,
            },
        }))
    }

    fn store(&self, key: &str, stamp: Stamp, facts: &Facts) -> Result<()> {
        let tokens = &facts.usage.tokens;
        self.conn.execute(
            "INSERT INTO transcripts
                (path, size, mtime, cwd, title, month, input, output, cache_write, cache_read, models)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size, mtime = excluded.mtime, cwd = excluded.cwd,
                title = excluded.title, month = excluded.month, input = excluded.input,
                output = excluded.output, cache_write = excluded.cache_write,
                cache_read = excluded.cache_read, models = excluded.models",
            params![
                key,
                stamp.size as i64,
                stamp.mtime_ms,
                facts.cwd,
                facts.title,
                facts.usage.month,
                tokens.input as i64,
                tokens.output as i64,
                tokens.cache_write as i64,
                tokens.cache_read as i64,
                serde_json::to_string(&facts.usage.models)?,
            ],
        )?;
        self.set_meta(
            "refreshed_at",
            &chrono::Utc::now().timestamp_millis().to_string(),
        )
    }

    /// Drop the rows of transcripts that were not looked up since the cache was opened: the
    /// files that are gone or moved. Call it after one complete pass over every project.
    pub fn prune(&self) -> Result<()> {
        let seen = self.seen.borrow();
        let stored: Vec<String> = self
            .conn
            .prepare("SELECT path FROM transcripts")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<_, _>>()?;
        for path in stored.iter().filter(|path| !seen.contains(*path)) {
            self.conn
                .execute("DELETE FROM transcripts WHERE path = ?1", [path])?;
        }
        Ok(())
    }

    fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn counts(&self) -> Result<Counts> {
        let transcripts: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM transcripts", [], |row| row.get(0))?;
        let refreshed_at = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'refreshed_at'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|value| value.parse().ok());
        Ok(Counts {
            transcripts: usize::try_from(transcripts).unwrap_or(0),
            refreshed_at,
            schema: self.version()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, lines: &[Value]) -> PathBuf {
        let path = dir.join(name);
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn facts_come_from_one_pass_over_the_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let answer = |id: &str, out: u64| {
            json!({"type": "assistant", "timestamp": "2026-03-04T10:00:00Z", "message": {
                "id": id, "model": "claude-opus",
                "usage": {"input_tokens": 1, "output_tokens": out}
            }})
        };
        let path = write(
            dir.path(),
            "a.jsonl",
            &[
                json!({"type": "user", "timestamp": "2026-02-01T09:00:00Z", "cwd": "/first"}),
                json!({"type": "custom-title", "customTitle": "Old"}),
                answer("m1", 5),
                answer("m1", 5),
                json!({"type": "user", "cwd": "/second"}),
                json!({"type": "custom-title", "customTitle": "New"}),
            ],
        );
        let facts = read_facts(&path).unwrap();
        assert_eq!(facts.cwd.as_deref(), Some("/first"));
        assert_eq!(facts.title.as_deref(), Some("New"));
        assert_eq!(facts.usage.tokens.output, 5);
        assert_eq!(facts.usage.month.as_deref(), Some("2026-02"));
        assert_eq!(facts.usage.models["claude-opus"], 1);
    }

    #[test]
    fn the_cache_reads_a_file_again_only_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("state");
        let path = write(
            dir.path(),
            "a.jsonl",
            &[json!({"type": "user", "cwd": "/one"})],
        );
        let cache = Cache::open(&home, false).unwrap();
        assert_eq!(cache.facts(&path).unwrap().cwd.as_deref(), Some("/one"));
        assert_eq!(cache.counts().unwrap().transcripts, 1);

        // Same size and time: the row stands in for the file.
        let kept = stamp(&path).unwrap();
        write(
            dir.path(),
            "a.jsonl",
            &[json!({"type": "user", "cwd": "/two"})],
        );
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_millis(kept.mtime_ms as u64))
            .unwrap();
        assert_eq!(stamp(&path).unwrap(), kept);
        assert_eq!(cache.facts(&path).unwrap().cwd.as_deref(), Some("/one"));
        // A fresh run reads the file anyway and remembers what it found.
        let fresh = Cache::open(&home, true).unwrap();
        assert_eq!(fresh.facts(&path).unwrap().cwd.as_deref(), Some("/two"));
        drop(fresh);
        assert_eq!(cache.facts(&path).unwrap().cwd.as_deref(), Some("/two"));
        // A changed size is read again without --fresh.
        write(
            dir.path(),
            "a.jsonl",
            &[json!({"type": "user", "cwd": "/three"})],
        );
        assert_eq!(cache.facts(&path).unwrap().cwd.as_deref(), Some("/three"));

        // Rows of files nobody asked about since the cache was opened are pruned.
        let other = write(dir.path(), "b.jsonl", &[json!({"cwd": "/b"})]);
        cache.facts(&other).unwrap();
        drop(cache);
        let pass = Cache::open(&home, false).unwrap();
        pass.facts(&path).unwrap();
        pass.prune().unwrap();
        assert_eq!(pass.counts().unwrap().transcripts, 1);
    }

    #[test]
    fn a_schema_change_rebuilds_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::open(dir.path(), false).unwrap();
        cache
            .conn
            .pragma_update(None, "user_version", SCHEMA + 1)
            .unwrap();
        cache
            .conn
            .execute_batch("DROP TABLE transcripts; CREATE TABLE junk (x)")
            .unwrap();
        drop(cache);
        let cache = Cache::open(dir.path(), false).unwrap();
        assert_eq!(cache.counts().unwrap().transcripts, 0);
        assert_eq!(cache.counts().unwrap().schema, SCHEMA);
    }
}
