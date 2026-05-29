//! History window input to `autosuggest` (`SCHEMA.md §3`).
//!
//! The host passes recent commands, most-recent-first preferred. Only `command`
//! is required per entry; `cwd`/`exitCode`/`ts` enable weighting in M2.

use serde::{Deserialize, Serialize};

/// A window of recent commands supplied by the host (`SCHEMA.md §3`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryWindow {
    /// Recent command entries, most-recent-first preferred.
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
}

/// A single history entry (`SCHEMA.md §3`).
///
/// Only `command` is required; the rest enable cwd/exit/recency weighting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    /// The full command line as executed.
    pub command: String,

    /// Working directory the command ran in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,

    /// Process exit code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,

    /// Unix timestamp (seconds) when the command ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_window_example_roundtrips() {
        // Mirrors the SCHEMA.md §3 example.
        let json = r#"{
            "entries": [
                { "command": "git push origin main", "cwd": "/repo", "exitCode": 0, "ts": 1730000000 }
            ]
        }"#;
        let w: HistoryWindow = serde_json::from_str(json).expect("deserialize §3");
        assert_eq!(w.entries.len(), 1);
        assert_eq!(w.entries[0].command, "git push origin main");
        assert_eq!(w.entries[0].exit_code, Some(0));
        assert_eq!(w.entries[0].ts, Some(1_730_000_000));

        let out = serde_json::to_string(&w).expect("serialize");
        assert!(out.contains("\"exitCode\""));
        let back: HistoryWindow = serde_json::from_str(&out).expect("re-de");
        assert_eq!(back, w);
    }

    #[test]
    fn command_only_entry_is_valid() {
        let entry: HistoryEntry =
            serde_json::from_str(r#"{ "command": "ls -la" }"#).expect("minimal entry");
        assert_eq!(entry.command, "ls -la");
        assert_eq!(entry.cwd, None);
        // Optional fields are omitted on re-serialize.
        let out = serde_json::to_string(&entry).expect("serialize");
        assert_eq!(out, r#"{"command":"ls -la"}"#);
    }
}
