//! Golden completion tests (`TECH.md §5`).
//!
//! Each fixture under `tests/fixtures/complete/<command>/` is a directory with:
//!   - `request.json`: a protocol `complete` request (`SCHEMA.md §4.1`).
//!   - `expected.json`: the expected `items` array (`SCHEMA.md §4.2/§4.3`),
//!     already sorted descending by score.
//!
//! The harness loads each request, resolves the command's spec from `specs/`,
//! routes the line through [`autosuggest_core::complete_line`], maps results to
//! the protocol `Item` shape, and asserts an exact, order-sensitive match.

use std::fs;
use std::path::{Path, PathBuf};

use autosuggest_core::types::Subcommand;
use autosuggest_protocol::request::{CompleteRequest, Request};
use autosuggest_protocol::response::Item;
use serde_json::Value;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .expect("workspace root")
}

fn fixtures_dir() -> PathBuf {
    workspace_root()
        .join("tests")
        .join("fixtures")
        .join("complete")
}

fn load_spec(command: &str) -> Subcommand {
    let path = workspace_root()
        .join("specs")
        .join(format!("{command}.spec.json"));
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// The command name a fixture targets: the first whitespace-delimited token of
/// the request line (e.g. `git status -` -> `git`).
fn command_of(line: &str) -> &str {
    line.split_whitespace().next().unwrap_or("")
}

/// Map a core [`autosuggest_core::CompletionItem`] to a protocol [`Item`].
///
/// `dangerous`/`deprecated` are only emitted when `true`, matching the
/// schema's "omit defaults" convention so fixtures stay terse.
fn to_protocol_item(c: &autosuggest_core::CompletionItem) -> Item {
    Item {
        insert: c.insert.clone(),
        display: if c.display == c.insert {
            None
        } else {
            Some(c.display.clone())
        },
        desc: c.desc.clone(),
        score: c.score,
        dangerous: c.dangerous.then_some(true),
        deprecated: c.deprecated.then_some(true),
    }
}

/// Round a score to a fixed number of decimals so floating-point output is
/// stable across platforms in golden comparisons.
fn round_score(v: &mut Value) {
    if let Some(items) = v.as_array_mut() {
        for item in items {
            if let Some(score) = item.get("score").and_then(Value::as_f64) {
                let rounded = (score * 1000.0).round() / 1000.0;
                item["score"] = serde_json::json!(rounded);
            }
        }
    }
}

fn run_case(dir: &Path) {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    let request_text = fs::read_to_string(dir.join("request.json"))
        .unwrap_or_else(|e| panic!("[{name}] read request.json: {e}"));
    let request: Request = serde_json::from_str(&request_text)
        .unwrap_or_else(|e| panic!("[{name}] parse request.json: {e}"));

    let req: CompleteRequest = match request {
        Request::Complete(c) => c,
        other => panic!("[{name}] expected a complete request, got {other:?}"),
    };

    let cursor = req.cursor_or_end();
    let cwd = req
        .cwd
        .clone()
        .map(PathBuf::from)
        .unwrap_or_else(workspace_root);

    let spec = load_spec(command_of(&req.line));
    let items = autosuggest_core::complete_line(&spec, &req.line, cursor, &cwd);
    let protocol_items: Vec<Item> = items.iter().map(to_protocol_item).collect();

    let mut actual = serde_json::to_value(&protocol_items)
        .unwrap_or_else(|e| panic!("[{name}] serialize actual: {e}"));
    round_score(&mut actual);

    // Optional capture mode: print the produced output instead of asserting,
    // to ease authoring/updating golden files. Enable with `ASC_DUMP_GOLDEN=1`.
    if std::env::var_os("ASC_DUMP_GOLDEN").is_some() {
        println!(
            "@@GOLDEN {name}\n{}",
            serde_json::to_string_pretty(&actual).unwrap_or_default()
        );
        return;
    }

    let expected_text = fs::read_to_string(dir.join("expected.json"))
        .unwrap_or_else(|e| panic!("[{name}] read expected.json: {e}"));
    let mut expected: Value = serde_json::from_str(&expected_text)
        .unwrap_or_else(|e| panic!("[{name}] parse expected.json: {e}"));
    round_score(&mut expected);

    if actual != expected {
        panic!(
            "golden mismatch in `{name}`:\n  expected: {}\n  actual:   {}",
            serde_json::to_string_pretty(&expected).unwrap_or_default(),
            serde_json::to_string_pretty(&actual).unwrap_or_default(),
        );
    }
}

#[test]
fn all_completion_fixtures_match() {
    let dir = fixtures_dir();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("read fixtures dir {}: {e}", dir.display()));

    let mut cases: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("request.json").exists())
        .collect();
    cases.sort();

    assert!(
        cases.len() >= 9,
        "expected at least 9 completion fixtures, found {}",
        cases.len()
    );

    for case in cases {
        run_case(&case);
    }
}
