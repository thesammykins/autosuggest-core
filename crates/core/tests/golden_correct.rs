//! Golden tests for the command-correction engine (`TECH.md §5`).
//!
//! Each fixture under `tests/fixtures/correct/<case>/` is a directory with:
//!   - `request.json`: a protocol `correct` request (`SCHEMA.md §4.1`):
//!     `{ "v":1, "op":"correct", "id":N, "script":"...", "stderr"?, "exitCode"? }`.
//!   - `expected.json`: the expected `items` array (`SCHEMA.md §4.2/§4.3`),
//!     already ordered as the engine ranks them (priority desc).
//!   - `path.json` (optional): a JSON array of command names that the
//!     deterministic [`MockCommandResolver`] should report as present on
//!     `$PATH`. Defaults to empty so resolver-driven rules are reproducible
//!     regardless of the host environment.
//!
//! The harness loads each request, drives it through
//! [`autosuggest_core::correct::correct`] with the engine's shipped rules and a
//! seeded mock resolver, maps results to the protocol `Item` shape exactly as
//! the daemon does, and asserts an exact, order-sensitive match.
//!
//! Authoring/refresh: run with `ASC_DUMP_GOLDEN=1` to print produced output
//! instead of asserting.

use std::fs;
use std::path::{Path, PathBuf};

use autosuggest_core::correct::rule::Rule;
use autosuggest_core::correct::{self, CorrectContext, CorrectedCommand, MockCommandResolver};
use autosuggest_core::types::Subcommand;
use autosuggest_protocol::request::{CorrectRequest, Request};
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
        .join("correct")
}

/// Load every shipped spec so spec-driven rules (e.g. `subcommand_typo`) see
/// the same data the daemon does.
fn load_specs() -> Vec<Subcommand> {
    let dir = workspace_root().join("specs");
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("read specs dir {}: {e}", dir.display()));
    let mut specs = Vec::new();
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let spec: Subcommand =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        specs.push(spec);
    }
    specs
}

/// Load every shipped correction rule from `rules/*.rule.json`, exactly as the
/// daemon does at startup.
fn load_rules() -> Vec<Rule> {
    let dir = workspace_root().join("rules");
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("read rules dir {}: {e}", dir.display()));
    let mut rules = Vec::new();
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let rule: Rule =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        rules.push(rule);
    }
    rules
}

/// Map a core [`CorrectedCommand`] to a protocol [`Item`], mirroring the
/// daemon's wire mapping (`crates/daemon/src/lib.rs`): the corrected command is
/// the `insert`, and `score` is `priority / 100` so the contract holds.
fn to_protocol_item(c: &CorrectedCommand) -> Item {
    Item {
        insert: c.command.clone(),
        display: None,
        desc: c.description.clone(),
        score: f64::from(c.priority) / 100.0,
        dangerous: None,
        deprecated: None,
    }
}

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

/// Read an optional `path.json` array of command names for the mock resolver.
fn load_path(dir: &Path) -> Vec<String> {
    let path = dir.join("path.json");
    if !path.exists() {
        return Vec::new();
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn run_case(dir: &Path, specs: &[Subcommand]) {
    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("?");

    let request_text = fs::read_to_string(dir.join("request.json"))
        .unwrap_or_else(|e| panic!("[{name}] read request.json: {e}"));
    let request: Request = serde_json::from_str(&request_text)
        .unwrap_or_else(|e| panic!("[{name}] parse request.json: {e}"));

    let req: CorrectRequest = match request {
        Request::Correct(c) => c,
        other => panic!("[{name}] expected a correct request, got {other:?}"),
    };

    let stderr = req.stderr.as_deref().unwrap_or("");
    let mut ctx = CorrectContext::new(&req.script, stderr, req.exit_code);
    ctx.cwd = req.cwd.as_deref();
    ctx.specs = specs;

    let resolver = MockCommandResolver::new(load_path(dir));
    let rules = load_rules();
    let corrections = correct::correct(&ctx, &rules, &resolver)
        .unwrap_or_else(|e| panic!("[{name}] correct: {e}"));
    let items: Vec<Item> = corrections.iter().map(to_protocol_item).collect();

    let mut actual =
        serde_json::to_value(&items).unwrap_or_else(|e| panic!("[{name}] serialize actual: {e}"));
    round_score(&mut actual);

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
fn all_correction_fixtures_match() {
    let specs = load_specs();
    let dir = fixtures_dir();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("read fixtures dir {}: {e}", dir.display()));

    let mut cases: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir() && p.join("request.json").exists())
        .collect();
    cases.sort();

    assert!(
        cases.len() >= 7,
        "expected at least 7 correction fixtures, found {}",
        cases.len()
    );

    for case in cases {
        run_case(&case, &specs);
    }
}
