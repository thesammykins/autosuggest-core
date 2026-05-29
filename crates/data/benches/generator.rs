//! Generator completion latency benchmark (`PRODUCT.md` NFR1: warm < 15 ms).
//!
//! Measures end-to-end [`autosuggest_core::complete_line_with_generators`] for a
//! generator-backed argument (`git checkout <branch>`) when the generator result
//! is *warm* in the [`SandboxedRunner`] TTL cache — the steady-state keystroke
//! path. A cold run (which spawns a process) is measured separately for context
//! but is not the NFR1 target.
//!
//! To keep the benchmark hermetic and fast, the generator invokes `echo` (on the
//! allow-list) to emit a fixed branch list; the cache then serves every
//! subsequent iteration without re-executing.

use std::hint::black_box;
use std::path::Path;

use autosuggest_core::complete_line_with_generators;
use autosuggest_core::types::{Generator, GeneratorCache, Subcommand};
use autosuggest_data::SandboxedRunner;
use criterion::{criterion_group, criterion_main, Criterion};

/// A `git`-like spec with a `checkout <branch>` whose `branch` arg is generated
/// by `echo`-ing a branch list. Authored inline so the bench does not depend on
/// the on-disk spec's generator wiring.
fn checkout_spec(echo: &str) -> Subcommand {
    let generator = Generator {
        run: vec![
            echo.to_string(),
            "main\nfeature/login\nfeature/cache\nrelease/1.0\nhotfix/crash".to_string(),
        ],
        split_on: None,
        trim: None,
        extract: None,
        priority: Some(80),
        cache: Some(GeneratorCache { ttl_ms: 60_000 }),
    };

    let json = serde_json::json!({
        "name": "git",
        "subcommands": [{
            "name": "checkout",
            "args": [{ "name": "branch", "generator": generator }]
        }]
    });
    serde_json::from_value(json).expect("valid inline checkout spec")
}

fn find_echo() -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join("echo"))
        .find(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
}

fn bench_generator(c: &mut Criterion) {
    let Some(echo) = find_echo() else {
        eprintln!("skipping generator bench: no echo on PATH");
        return;
    };

    let spec = checkout_spec(&echo);
    let runner = SandboxedRunner::with_allow_list([echo]);
    let cwd = Path::new(".");
    let line = "git checkout ";
    let cursor = line.len();

    // Warm the cache once so the measured iterations are pure cache hits.
    let warm = complete_line_with_generators(&spec, line, cursor, cwd, &runner);
    assert!(
        warm.iter().any(|i| i.insert == "feature/login"),
        "generator should surface branches: {warm:?}"
    );

    let mut group = c.benchmark_group("complete_generator");
    group.bench_function("git_checkout_branch_warm", |b| {
        b.iter(|| {
            let out = complete_line_with_generators(
                black_box(&spec),
                black_box(line),
                black_box(cursor),
                black_box(cwd),
                black_box(&runner),
            );
            black_box(out);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_generator);
criterion_main!(benches);
