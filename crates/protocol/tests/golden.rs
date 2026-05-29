//! Golden-test entrypoint (`TECH.md §5`).
//!
//! Includes the shared [`harness`] module and runs it against the workspace
//! `tests/fixtures/` tree. For M0 this only proves the harness can discover and
//! compare fixture pairs (there is no engine yet); later milestones will route
//! `request.json` through the engine before comparing to `expected.json`.

#[path = "support/harness.rs"]
mod harness;

use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    harness::workspace_root().join("tests").join("fixtures")
}

#[test]
fn harness_discovers_and_compares_fixtures() {
    let dir = fixtures_dir();
    let cases = harness::load_fixtures(&dir).expect("load fixtures");

    // M0 ships at least the smoke fixture; the harness must find it.
    assert!(
        !cases.is_empty(),
        "expected at least one golden fixture under {}",
        dir.display()
    );

    // For M0 the smoke fixture's request == expected, so identity comparison
    // exercises the full discover -> parse -> compare path.
    for case in &cases {
        harness::assert_matches(&case.name, &case.request, &case.expected);
    }
}

#[test]
fn harness_smoke_fixture_present() {
    let cases = harness::load_fixtures(&fixtures_dir()).expect("load fixtures");
    assert!(
        cases.iter().any(|c| c.name == "harness_smoke"),
        "harness_smoke fixture should exist"
    );
}
