//! Request envelopes (`SCHEMA.md §4.1`).
//!
//! Requests share a versioned envelope (`"v"`, `"id"`, `"op"`) and are
//! distinguished by the `op` discriminator. We model this as an internally
//! tagged enum so each variant carries exactly its operation's fields.
//!
//! Unknown fields are ignored for forward-compatibility (`SCHEMA.md §4.3`); we do
//! not enable `deny_unknown_fields`.

use serde::{Deserialize, Serialize};

use crate::history::HistoryWindow;

/// A protocol request, tagged by its `op` (`SCHEMA.md §4.1`).
///
/// The envelope fields `v` and `id` are flattened into each variant so the JSON
/// shape matches the schema exactly (a single flat object per line).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// As-you-type completion request.
    Complete(CompleteRequest),
    /// History ghost-text request.
    Autosuggest(AutosuggestRequest),
    /// Failed-command correction request.
    Correct(CorrectRequest),
}

/// `complete` request body (`SCHEMA.md §4.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteRequest {
    /// Protocol version (`1`).
    pub v: u32,
    /// Request id echoed in the response.
    pub id: i64,
    /// The full input line.
    pub line: String,
    /// Byte offset of the cursor into `line`. Defaults to end of `line` if
    /// omitted (`SCHEMA.md §4.1`); resolve via [`CompleteRequest::cursor_or_end`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<usize>,
    /// Working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Environment hints (e.g. `{"SHELL":"zsh"}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

impl CompleteRequest {
    /// The cursor offset, defaulting to the byte length of `line` when omitted
    /// (`SCHEMA.md §4.1`).
    pub fn cursor_or_end(&self) -> usize {
        self.cursor.unwrap_or(self.line.len())
    }
}

/// `autosuggest` request body (`SCHEMA.md §4.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutosuggestRequest {
    /// Protocol version (`1`).
    pub v: u32,
    /// Request id echoed in the response.
    pub id: i64,
    /// The current input prefix to continue.
    pub prefix: String,
    /// Working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Recent-command window for weighting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<HistoryWindow>,
}

/// `correct` request body (`SCHEMA.md §4.1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectRequest {
    /// Protocol version (`1`).
    pub v: u32,
    /// Request id echoed in the response.
    pub id: i64,
    /// The script/command line that failed.
    pub script: String,
    /// Captured stderr from the failed command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    /// Exit code of the failed command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Working directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(json: &str) -> Request {
        let req: Request = serde_json::from_str(json).expect("deserialize request");
        let out = serde_json::to_string(&req).expect("serialize request");
        let back: Request = serde_json::from_str(&out).expect("re-deserialize request");
        assert_eq!(req, back);
        req
    }

    #[test]
    fn complete_request_example() {
        // Mirrors the SCHEMA.md §4.1 complete example.
        let json = r#"{ "v": 1, "id": 7, "op": "complete",
            "line": "git ch", "cursor": 6, "cwd": "/repo", "env": { "SHELL": "zsh" } }"#;
        let req = roundtrip(json);
        match req {
            Request::Complete(c) => {
                assert_eq!(c.id, 7);
                assert_eq!(c.line, "git ch");
                assert_eq!(c.cursor_or_end(), 6);
                assert_eq!(
                    c.env
                        .as_ref()
                        .and_then(|e| e.get("SHELL"))
                        .map(String::as_str),
                    Some("zsh")
                );
            }
            other => panic!("expected complete, got {other:?}"),
        }
    }

    #[test]
    fn complete_cursor_defaults_to_line_end() {
        let json = r#"{ "v": 1, "id": 1, "op": "complete", "line": "git ch" }"#;
        let req: Request = serde_json::from_str(json).expect("deserialize");
        match req {
            Request::Complete(c) => {
                assert_eq!(c.cursor, None);
                assert_eq!(c.cursor_or_end(), "git ch".len());
            }
            other => panic!("expected complete, got {other:?}"),
        }
    }

    #[test]
    fn autosuggest_request_example() {
        // Mirrors the SCHEMA.md §4.1 autosuggest example.
        let json = r#"{ "v": 1, "id": 8, "op": "autosuggest",
            "prefix": "git pu", "cwd": "/repo",
            "history": { "entries": [ { "command": "git push origin main" } ] } }"#;
        let req = roundtrip(json);
        match req {
            Request::Autosuggest(a) => {
                assert_eq!(a.prefix, "git pu");
                assert_eq!(a.history.as_ref().map(|h| h.entries.len()), Some(1));
            }
            other => panic!("expected autosuggest, got {other:?}"),
        }
    }

    #[test]
    fn correct_request_example() {
        // Mirrors the SCHEMA.md §4.1 correct example.
        let json = r#"{ "v": 1, "id": 9, "op": "correct",
            "script": "mkdir a/b", "stderr": "mkdir: a: No such file or directory",
            "exitCode": 1, "cwd": "/repo" }"#;
        let req = roundtrip(json);
        match req {
            Request::Correct(c) => {
                assert_eq!(c.script, "mkdir a/b");
                assert_eq!(c.exit_code, Some(1));
                assert_eq!(
                    c.stderr.as_deref(),
                    Some("mkdir: a: No such file or directory")
                );
            }
            other => panic!("expected correct, got {other:?}"),
        }
    }

    #[test]
    fn correct_request_serializes_exit_code_camelcase() {
        let req = Request::Correct(CorrectRequest {
            v: 1,
            id: 9,
            script: "x".to_string(),
            stderr: None,
            exit_code: Some(1),
            cwd: None,
        });
        let out = serde_json::to_string(&req).expect("serialize");
        assert!(out.contains("\"exitCode\""));
        assert!(out.contains("\"op\":\"correct\""));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{ "v": 1, "id": 1, "op": "complete", "line": "ls", "futureField": true }"#;
        let req: Request = serde_json::from_str(json).expect("ignore unknown");
        assert!(matches!(req, Request::Complete(_)));
    }
}
