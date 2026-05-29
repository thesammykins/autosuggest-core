//! SQLite-backed command history store.
//!
//! Stores command entries with optional context (cwd, exit code, timestamp)
//! and queries recent history for autosuggestion.
//!
//! ```no_run
//! use autosuggest_history_store::HistoryStore;
//!
//! let store = HistoryStore::open(":memory:").expect("create store");
//! store.record("git push origin main", Some("/repo"), Some(0)).unwrap();
//! store.record("git status", Some("/repo"), Some(0)).unwrap();
//!
//! let window = store.recent("git", 10, None).unwrap();
//! assert_eq!(window.entries.len(), 2);
//! ```

#![forbid(unsafe_code)]

use std::path::Path;

use autosuggest_protocol::history::{HistoryEntry, HistoryWindow};

/// A SQLite-backed history store.
pub struct HistoryStore {
    conn: rusqlite::Connection,
}

impl HistoryStore {
    /// Open (or create) a SQLite database at `path`.
    ///
    /// Use `":memory:"` for a temporary in-memory store (useful in tests).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let conn = rusqlite::Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), rusqlite::Error> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS history_entries (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                command   TEXT NOT NULL,
                cwd       TEXT,
                exit_code INTEGER,
                timestamp INTEGER NOT NULL DEFAULT (strftime('%s','now'))
            );
            CREATE INDEX IF NOT EXISTS idx_history_command_prefix
                ON history_entries(command);
            CREATE INDEX IF NOT EXISTS idx_history_timestamp
                ON history_entries(timestamp DESC);",
        )
    }

    /// Record a command entry in history.
    ///
    /// `command` is the full command text.
    /// `cwd` is the working directory (optional).
    /// `exit_code` is the process exit code (optional).
    pub fn record(
        &self,
        command: &str,
        cwd: Option<&str>,
        exit_code: Option<i32>,
    ) -> Result<(), rusqlite::Error> {
        self.conn.execute(
            "INSERT INTO history_entries (command, cwd, exit_code) VALUES (?1, ?2, ?3)",
            rusqlite::params![command, cwd, exit_code],
        )?;
        Ok(())
    }

    /// Return recent command entries matching a prefix.
    ///
    /// Results are most-recent-first. `limit` caps the number of results.
    /// `cwd_filter` optionally narrows to a specific working directory.
    pub fn recent(
        &self,
        prefix: &str,
        limit: usize,
        cwd_filter: Option<&str>,
    ) -> Result<HistoryWindow, rusqlite::Error> {
        let like_pattern = format!("{}%", prefix);

        let query = if cwd_filter.is_some() {
            "SELECT command, cwd, exit_code, timestamp
             FROM history_entries
             WHERE command LIKE ?1
               AND cwd = ?2
             ORDER BY timestamp DESC
             LIMIT ?3"
        } else {
            "SELECT command, cwd, exit_code, timestamp
             FROM history_entries
             WHERE command LIKE ?1
             ORDER BY timestamp DESC
             LIMIT ?2"
        };

        let mut stmt = self.conn.prepare(query)?;

        let rows: Vec<HistoryEntry> = if let Some(cwd) = cwd_filter {
            stmt.query_map(
                rusqlite::params![like_pattern, cwd, limit as i64],
                HistoryStore::row_to_entry,
            )?
            .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(
                rusqlite::params![like_pattern, limit as i64],
                HistoryStore::row_to_entry,
            )?
            .collect::<Result<Vec<_>, _>>()?
        };

        Ok(HistoryWindow { entries: rows })
    }

    /// Map a SQLite row to a [`HistoryEntry`].
    fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            command: row.get(0)?,
            cwd: row.get(1)?,
            exit_code: row.get(2)?,
            ts: row.get(3)?,
        })
    }

    /// Delete all entries from the history store.
    pub fn clear(&self) -> Result<(), rusqlite::Error> {
        self.conn.execute("DELETE FROM history_entries", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> HistoryStore {
        HistoryStore::open(":memory:").expect("create in-memory store")
    }

    #[test]
    fn record_and_retrieve() {
        let store = test_store();

        store.record("git status", Some("/repo"), Some(0)).unwrap();
        store
            .record("git push origin main", Some("/repo"), Some(0))
            .unwrap();

        let window = store.recent("git", 10, None).unwrap();
        assert_eq!(window.entries.len(), 2);

        let second = store.recent("git push", 10, None).unwrap();
        assert_eq!(second.entries.len(), 1);
        assert_eq!(second.entries[0].command, "git push origin main");
    }

    #[test]
    fn limit_results() {
        let store = test_store();

        for i in 0..10 {
            store.record(&format!("echo {i}"), None, Some(0)).unwrap();
        }

        let window = store.recent("echo", 3, None).unwrap();
        assert_eq!(window.entries.len(), 3);
    }

    #[test]
    fn cwd_filter_works() {
        let store = test_store();

        store.record("ls", Some("/repo-a"), Some(0)).unwrap();
        store.record("ls", Some("/repo-b"), Some(0)).unwrap();

        let only_a = store.recent("ls", 10, Some("/repo-a")).unwrap();
        assert_eq!(only_a.entries.len(), 1);
    }

    #[test]
    fn clear_removes_all_entries() {
        let store = test_store();

        store.record("echo hello", None, Some(0)).unwrap();
        store.clear().unwrap();

        let window = store.recent("echo", 10, None).unwrap();
        assert_eq!(window.entries.len(), 0);
    }

    #[test]
    fn empty_store_returns_empty() {
        let store = test_store();
        let window = store.recent("anything", 10, None).unwrap();
        assert_eq!(window.entries.len(), 0);
    }
}
