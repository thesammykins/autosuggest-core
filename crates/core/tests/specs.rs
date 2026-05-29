//! Validates that every authored spec under `specs/` deserializes into the
//! [`Subcommand`] model (`SCHEMA.md §1`).
//!
//! This is the M0 acceptance test for the seed dataset: each file must parse,
//! carry a non-empty canonical name, and survive a serde round-trip.

use std::fs;
use std::path::{Path, PathBuf};

use autosuggest_core::types::Subcommand;

fn specs_dir() -> PathBuf {
    // crates/core -> workspace root -> specs
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("specs")
}

fn spec_files() -> Vec<PathBuf> {
    let dir = specs_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read specs dir {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".spec.json"))
        })
        .collect();
    files.sort();
    files
}

#[test]
fn all_specs_deserialize_into_subcommand() {
    let files = spec_files();
    assert!(
        files.len() >= 5,
        "expected at least 5 seed specs, found {}",
        files.len()
    );

    for path in files {
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        let spec: Subcommand = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("deserialize {} into Subcommand: {e}", path.display()));

        assert!(
            !spec.name.canonical().is_empty(),
            "{} has an empty canonical name",
            path.display()
        );

        // Round-trip: re-serialize and re-parse must be stable.
        let reserialized = serde_json::to_string(&spec)
            .unwrap_or_else(|e| panic!("serialize {}: {e}", path.display()));
        let back: Subcommand = serde_json::from_str(&reserialized)
            .unwrap_or_else(|e| panic!("re-deserialize {}: {e}", path.display()));
        assert_eq!(back, spec, "round-trip mismatch for {}", path.display());
    }
}

#[test]
fn expected_seed_specs_exist() {
    let names: Vec<String> = spec_files()
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    for expected in ["ls", "cd", "mkdir", "echo", "git"] {
        let file = format!("{expected}.spec.json");
        assert!(
            names.contains(&file),
            "missing seed spec {file}; found {names:?}"
        );
    }
}
