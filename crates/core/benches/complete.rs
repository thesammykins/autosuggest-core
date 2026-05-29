//! Static-completion latency benchmark (`TECH.md §5`: < 5 ms target).
//!
//! Measures end-to-end [`autosuggest_core::complete_line`] for spec-derived
//! (non-filesystem) completions, which is the hot, allocation-light path that
//! runs on every keystroke. Filesystem and generator paths are intentionally
//! excluded here: they are I/O-bound and covered by their own tests.

use std::hint::black_box;
use std::path::Path;

use autosuggest_core::types::Subcommand;
use criterion::{criterion_group, criterion_main, Criterion};

fn load_spec(command: &str) -> Subcommand {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("specs")
        .join(format!("{command}.spec.json"));
    let text = std::fs::read_to_string(&path).expect("read spec");
    serde_json::from_str(&text).expect("parse spec")
}

fn bench_static(c: &mut Criterion) {
    let git = load_spec("git");
    let grep = load_spec("grep");
    let cwd = Path::new(".");

    let mut group = c.benchmark_group("complete_static");

    group.bench_function("git_subcommands", |b| {
        b.iter(|| {
            let out = autosuggest_core::complete_line(
                black_box(&git),
                black_box("git "),
                black_box(4),
                black_box(cwd),
            );
            black_box(out);
        });
    });

    group.bench_function("git_subcommand_prefix", |b| {
        b.iter(|| {
            let out = autosuggest_core::complete_line(
                black_box(&git),
                black_box("git che"),
                black_box(7),
                black_box(cwd),
            );
            black_box(out);
        });
    });

    group.bench_function("grep_long_options", |b| {
        b.iter(|| {
            let out = autosuggest_core::complete_line(
                black_box(&grep),
                black_box("grep --"),
                black_box(7),
                black_box(cwd),
            );
            black_box(out);
        });
    });

    group.bench_function("grep_option_value", |b| {
        b.iter(|| {
            let out = autosuggest_core::complete_line(
                black_box(&grep),
                black_box("grep --color=a"),
                black_box(14),
                black_box(cwd),
            );
            black_box(out);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_static);
criterion_main!(benches);
