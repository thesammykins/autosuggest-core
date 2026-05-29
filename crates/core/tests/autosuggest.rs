//! Golden tests for the history autosuggester (`TECH.md §5`, `ROADMAP.md` M2).
//!
//! Each fixture under `tests/fixtures/autosuggest/<case>/` is a
//! `(request.json, expected.json)` pair where:
//! - `request.json` is an `autosuggest` request envelope (`SCHEMA.md §4.1`);
//! - `expected.json` is the `autosuggest` response envelope (`SCHEMA.md §4.2`,
//!   `{ "v":1, "id":N, "suggestion": <string|null> }`).
//!
//! The runner parses the request, drives it through [`history::autosuggest`],
//! builds the protocol [`Response`], and asserts the serialized result equals the
//! expected JSON value (order-insensitive object comparison via `serde_json`).

use std::fs;
use std::path::{Path, PathBuf};

use autosuggest_core::history;
use autosuggest_protocol::{Request, Response};
use serde_json::Value;

/// Workspace root: `crates/core` -> `../..`.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("workspace root")
}

fn autosuggest_fixtures_dir() -> PathBuf {
    workspace_root()
        .join("tests")
        .join("fixtures")
        .join("autosuggest")
}

/// A discovered `(request, expected)` fixture pair.
struct Case {
    name: String,
    request: Value,
    expected: Value,
}

fn load_cases(dir: &Path) -> Vec<Case> {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("read fixtures dir {}: {e}", dir.display()));

    let mut cases = Vec::new();
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if !path.is_dir() {
            continue;
        }
        let request_path = path.join("request.json");
        let expected_path = path.join("expected.json");
        if !request_path.exists() || !expected_path.exists() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        cases.push(Case {
            name,
            request: read_json(&request_path),
            expected: read_json(&expected_path),
        });
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Run a single `autosuggest` request fixture through the engine, returning the
/// protocol response as a JSON value.
fn run_autosuggest(request: &Value) -> Value {
    let req: Request =
        serde_json::from_value(request.clone()).expect("request is a valid protocol envelope");

    let Request::Autosuggest(a) = req else {
        panic!("autosuggest fixtures must use op=autosuggest");
    };

    let window = a.history.unwrap_or_default();
    let suggestion = history::autosuggest(&a.prefix, &window, a.cwd.as_deref());
    let resp = Response::suggestion(a.id, suggestion);
    serde_json::to_value(&resp).expect("serialize response")
}

#[test]
fn autosuggest_golden_fixtures_pass() {
    let dir = autosuggest_fixtures_dir();
    let cases = load_cases(&dir);
    assert!(
        !cases.is_empty(),
        "expected at least one autosuggest golden fixture under {}",
        dir.display()
    );

    for case in &cases {
        let actual = run_autosuggest(&case.request);
        assert_eq!(
            actual,
            case.expected,
            "golden mismatch in `{}`:\n  expected: {}\n  actual:   {}",
            case.name,
            serde_json::to_string(&case.expected).unwrap_or_default(),
            serde_json::to_string(&actual).unwrap_or_default(),
        );
    }
}

#[test]
fn recorded_history_fixture_present() {
    let cases = load_cases(&autosuggest_fixtures_dir());
    assert!(
        cases.iter().any(|c| c.name == "git_push_recorded"),
        "the recorded-history fixture `git_push_recorded` must exist"
    );
}
