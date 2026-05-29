//! Golden-test harness (`TECH.md §5`).
//!
//! A golden fixture is a directory containing a `request.json` and an
//! `expected.json`. A harness run loads each pair, (in later milestones) feeds
//! `request.json` through the engine, and asserts the produced output equals
//! `expected.json` (order-sensitive, since ranking is order-sensitive).
//!
//! # Milestone status (M0)
//!
//! No engine exists yet, so M0 only proves the harness machinery works: it can
//! discover fixture pairs, parse both JSON documents, and compare two JSON
//! values for structural equality. The bundled `harness_smoke` fixture has a
//! `request.json` equal to its `expected.json`, so the comparison passes and the
//! harness is exercised end-to-end.
//!
//! This file is included by integration tests via `#[path = ...] mod harness;`.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// A single discovered golden fixture: a `(request, expected)` JSON pair.
#[derive(Debug, Clone)]
pub struct GoldenCase {
    /// Fixture directory name (used in failure messages).
    pub name: String,
    /// Parsed `request.json`.
    pub request: Value,
    /// Parsed `expected.json`.
    pub expected: Value,
}

/// Absolute path to the workspace root, derived from the test crate's manifest
/// directory (`crates/<crate>` -> `../..`).
pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}

/// Load every fixture pair under `dir`.
///
/// A fixture is any immediate subdirectory of `dir` that contains both
/// `request.json` and `expected.json`. Returns the cases sorted by name for
/// deterministic ordering.
pub fn load_fixtures(dir: &Path) -> Result<Vec<GoldenCase>, String> {
    let mut cases = Vec::new();

    let entries =
        fs::read_dir(dir).map_err(|e| format!("read fixtures dir {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("read dir entry: {e}"))?;
        let path = entry.path();
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

        let request = read_json(&request_path)?;
        let expected = read_json(&expected_path)?;

        cases.push(GoldenCase {
            name,
            request,
            expected,
        });
    }

    cases.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(cases)
}

/// Read and parse a JSON file into a [`Value`].
fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Assert that `actual` equals `expected` for a named case, producing a readable
/// diff-style panic message on mismatch. Comparison is exact (order-sensitive).
pub fn assert_matches(name: &str, actual: &Value, expected: &Value) {
    if actual != expected {
        panic!(
            "golden mismatch in `{name}`:\n  expected: {}\n  actual:   {}",
            serde_json::to_string(expected).unwrap_or_default(),
            serde_json::to_string(actual).unwrap_or_default(),
        );
    }
}
