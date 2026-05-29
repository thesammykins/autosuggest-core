//! `autosuggest-data`: the side-effectful half of the engine (`TECH.md §3.4`).
//!
//! The `autosuggest-core` crate is deliberately pure — it performs no process,
//! file, or network I/O. Dynamic argument *generators* (`SCHEMA.md §1.5`) need
//! to actually run programs, so that capability lives here, behind the
//! [`autosuggest_core::GeneratorRunner`] trait, and nowhere else.
//!
//! The entry point is [`SandboxedRunner`], which enforces the `PRODUCT.md` NFR3
//! security model — allow-list only, no shell, hard timeout, output cap — and
//! caches results by `(run, cwd)` for each generator's TTL so warm completions
//! stay inside the NFR1 `< 15 ms` budget.
//!
//! ```no_run
//! use autosuggest_core::complete_line_with_generators;
//! use autosuggest_data::SandboxedRunner;
//! # use autosuggest_core::types::Subcommand;
//! # fn demo(spec: &Subcommand) {
//! let runner = SandboxedRunner::new();
//! let items = complete_line_with_generators(spec, "git checkout ", 13, ".".as_ref(), &runner);
//! # let _ = items;
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cache;
mod parse;
mod runner;

pub use cache::TtlCache;
pub use runner::{SandboxedRunner, DEFAULT_ALLOW_LIST, DEFAULT_MAX_OUTPUT_BYTES, DEFAULT_TIMEOUT};

#[cfg(test)]
mod integration_tests {
    //! Cross-module sanity: a real `echo`-backed generator flows through the
    //! runner and is parsed. (Allow-list/timeout/cache specifics live in the
    //! per-module unit tests.) The pure-engine integration — dynamic
    //! suggestions through ranking with a *mock* runner — lives in `core`.

    use crate::SandboxedRunner;
    use autosuggest_core::types::{Generator, GeneratorCache};
    use autosuggest_core::GeneratorRunner;

    fn find_echo() -> Option<String> {
        let path = std::env::var("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            let full = dir.join("echo");
            if full.is_file() {
                return Some(full.to_string_lossy().into_owned());
            }
        }
        None
    }

    #[test]
    fn echo_generator_end_to_end() {
        let Some(echo) = find_echo() else {
            eprintln!("skipping: no echo on PATH");
            return;
        };
        let runner = SandboxedRunner::with_allow_list([echo.clone()]);
        let g = Generator {
            run: vec![echo, "a\nb\nc".to_string()],
            split_on: None,
            trim: None,
            extract: None,
            priority: Some(80),
            cache: Some(GeneratorCache { ttl_ms: 1000 }),
        };
        let out = runner.run(&g, ".").expect("echo runs");
        assert_eq!(out, vec!["a", "b", "c"]);
    }
}
