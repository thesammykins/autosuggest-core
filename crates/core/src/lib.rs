//! `autosuggest-core` — the pure completion & correction engine.
//!
//! This crate is the heart of the project: a side-effect-free transformation of
//! `context -> suggestions`. Per `TECH.md §2`, the host owns I/O and the engine
//! owns logic. Generator execution is injected through the [`GeneratorRunner`]
//! trait so the engine never performs file or network I/O directly.
//!
//! # Milestone status (M1 + M2 + M3)
//!
//! As-you-type completion is implemented: [`tokenize`] (pure lexer), [`parse`]
//! (pure parse-state machine), [`complete`] (candidate collection, including the
//! one permitted filesystem read via [`fs_source`] for path templates), and
//! [`rank`] (filter + score). The top-level entry point is [`complete_line`].
//!
//! History autosuggestion is implemented: [`history`] provides the stateless
//! [`history::autosuggest`] continuation finder.
//!
//! Failed-command correction is implemented: [`correct`] is a JSON rule engine
//! plus native predicates (`SCHEMA.md §2`/§2.1), with the host's `$PATH` probe
//! injected via [`correct::CommandResolver`].
//!
//! Generators (dynamic arguments) are executed only when a [`GeneratorRunner`]
//! is injected via [`complete_line_with_generators`]; the real constrained runner
//! lives in the `autosuggest-data` crate (M4). The pure [`complete_line`] never
//! runs a generator. See `ROADMAP.md`.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use autosuggest_core::{complete_line, types::Subcommand};
//!
//! # fn demo(spec: &Subcommand) {
//! let items = complete_line(spec, "git co", 6, Path::new("."));
//! for item in &items {
//!     println!("{}  ({:.2})", item.insert, item.score);
//! }
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod complete;
pub mod correct;
pub mod fs_source;
pub mod history;
pub mod parse;
pub mod rank;
pub mod tokenize;
pub mod types;

mod generator_runner;

pub use generator_runner::{GeneratorError, GeneratorRunner};
pub use rank::CompletionItem;

use std::path::Path;

use types::Subcommand;

/// Compute ranked completions for `line` at byte offset `cursor`, against the
/// command tree `spec`, resolving path templates relative to `cwd`.
///
/// This is the M1 entry point and composes the engine's pure stages:
/// [`tokenize`] → [`parse`] → [`complete`] → [`rank`]. Only the [`complete`]
/// stage may touch the filesystem (for `filepaths`/`folders` templates), and
/// only under `cwd`; everything else is pure.
///
/// `cursor` is clamped to the line by the tokenizer, so any offset is safe.
/// Returns at most [`rank::DEFAULT_TOP_N`] items in descending score order.
pub fn complete_line(
    spec: &Subcommand,
    line: &str,
    cursor: usize,
    cwd: &Path,
) -> Vec<CompletionItem> {
    let tokens = tokenize::tokenize(line, cursor);
    let state = parse::parse(spec, tokens.committed(), tokens.query());

    // The query used for filtering candidates. For an inline `--opt=value`
    // form the value after `=` is what the user is completing, not the whole
    // `--opt=value` token; the parser exposes that as `inline_value_prefix`.
    let effective_query = state
        .inline_value_prefix
        .as_deref()
        .unwrap_or_else(|| tokens.query());

    let candidates = complete::collect(&state, effective_query, cwd);
    rank::rank(candidates, effective_query)
}

/// As [`complete_line`], but executes argument generators through the injected
/// `runner` (M4). This is the entry point a host uses to surface dynamic
/// suggestions (e.g. `git checkout <branch>` listing local branches).
///
/// `core` still performs no process I/O itself: the `runner` (provided by the
/// `autosuggest-data` crate's constrained implementation, or a test mock) owns all
/// execution, allow-listing, timeouts, and caching. A generator that fails
/// contributes no candidates, so completion degrades gracefully.
///
/// The pure [`complete_line`] remains unchanged and never runs a generator.
pub fn complete_line_with_generators(
    spec: &Subcommand,
    line: &str,
    cursor: usize,
    cwd: &Path,
    runner: &dyn GeneratorRunner,
) -> Vec<CompletionItem> {
    let tokens = tokenize::tokenize(line, cursor);
    let state = parse::parse(spec, tokens.committed(), tokens.query());

    let effective_query = state
        .inline_value_prefix
        .as_deref()
        .unwrap_or_else(|| tokens.query());

    let candidates = complete::collect_with_generators(&state, effective_query, cwd, runner);
    rank::rank(candidates, effective_query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use types::{Opt, ParserDirectives, Repeatable, StringList};

    fn temp_dir(tag: &str) -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "asc_line_{}_{}_{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&d).expect("mk temp");
        d
    }

    fn git_spec() -> Subcommand {
        Subcommand {
            name: "git".into(),
            description: Some("Distributed VCS".into()),
            subcommands: vec![
                Subcommand {
                    name: "status".into(),
                    description: Some("Show status".into()),
                    subcommands: vec![],
                    options: vec![],
                    args: vec![],
                    requires_subcommand: false,
                    parser_directives: None,
                },
                Subcommand {
                    name: StringList::Many(vec!["checkout".into(), "co".into()]),
                    description: Some("Switch branches".into()),
                    subcommands: vec![],
                    options: vec![],
                    args: vec![],
                    requires_subcommand: false,
                    parser_directives: None,
                },
            ],
            options: vec![],
            args: vec![],
            requires_subcommand: true,
            parser_directives: None,
        }
    }

    #[test]
    fn completes_subcommands_by_prefix() {
        let spec = git_spec();
        let items = complete_line(&spec, "git st", 6, std::path::Path::new("."));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "status ");
        assert_eq!(items[0].display, "status");
    }

    #[test]
    fn completes_subcommand_alias() {
        let spec = git_spec();
        let items = complete_line(&spec, "git co", 6, std::path::Path::new("."));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "checkout ");
    }

    #[test]
    fn empty_after_command_lists_all_subcommands() {
        let spec = git_spec();
        let items = complete_line(&spec, "git ", 4, std::path::Path::new("."));
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn cursor_clamped_when_out_of_range() {
        let spec = git_spec();
        // Cursor far past end must not panic.
        let items = complete_line(&spec, "git st", 9999, std::path::Path::new("."));
        assert_eq!(items[0].insert, "status ");
    }

    #[test]
    fn completes_nested_filepath_argument() {
        let d = temp_dir("nested");
        fs::create_dir(d.join("src")).expect("mkdir");
        fs::write(d.join("src").join("main.rs"), "x").expect("write");
        fs::write(d.join("src").join("lib.rs"), "x").expect("write");

        let cat = Subcommand {
            name: "cat".into(),
            description: None,
            subcommands: vec![],
            options: vec![],
            args: vec![types::Arg {
                name: Some("file".into()),
                description: None,
                is_optional: false,
                is_variadic: false,
                template: Some(types::Template::Filepaths),
                suggestions: vec![],
                generator: None,
                is_command: false,
            }],
            requires_subcommand: false,
            parser_directives: None,
        };

        let line = "cat src/ma";
        let items = complete_line(&cat, line, line.len(), &d);
        assert!(
            items.iter().any(|i| i.insert == "src/main.rs"),
            "expected src/main.rs, got {items:?}"
        );
        assert!(items.iter().all(|i| i.insert != "src/lib.rs"));
        fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn option_value_inline_separator() {
        let mut color = Opt {
            name: "--color".into(),
            description: None,
            args: vec![types::Arg {
                name: Some("when".into()),
                description: None,
                is_optional: false,
                is_variadic: false,
                template: None,
                suggestions: vec!["auto".into(), "always".into(), "never".into()],
                generator: None,
                is_command: false,
            }],
            is_required: false,
            is_repeatable: Repeatable::Flag(false),
            is_persistent: false,
            requires_separator: true,
            exclusive_on: vec![],
            depends_on: vec![],
        };
        color.requires_separator = true;
        let ls = Subcommand {
            name: "ls".into(),
            description: None,
            subcommands: vec![],
            options: vec![color],
            args: vec![],
            requires_subcommand: false,
            parser_directives: Some(ParserDirectives {
                flags_are_posix_noncompliant: true,
                options_must_precede_arguments: false,
                option_arg_separators: vec!["=".into(), " ".into()],
            }),
        };
        let line = "ls --color=al";
        let items = complete_line(&ls, line, line.len(), std::path::Path::new("."));
        assert!(items.iter().any(|i| i.insert == "always"));
        assert!(items.iter().all(|i| i.insert != "never"));
    }

    /// A deterministic in-memory [`GeneratorRunner`] that returns canned values
    /// (or a canned error) without spawning a process. This proves the engine
    /// stays pure: dynamic suggestions flow through `complete`/`rank` driven
    /// purely by the trait, with no I/O in `core`.
    struct MockRunner {
        result: Result<Vec<String>, GeneratorError>,
        calls: std::cell::Cell<usize>,
    }

    impl MockRunner {
        fn ok(values: &[&str]) -> Self {
            MockRunner {
                result: Ok(values.iter().map(|s| s.to_string()).collect()),
                calls: std::cell::Cell::new(0),
            }
        }
        fn err(e: GeneratorError) -> Self {
            MockRunner {
                result: Err(e),
                calls: std::cell::Cell::new(0),
            }
        }
    }

    impl GeneratorRunner for MockRunner {
        fn run(
            &self,
            _generator: &types::Generator,
            _cwd: &str,
        ) -> Result<Vec<String>, GeneratorError> {
            self.calls.set(self.calls.get() + 1);
            self.result.clone()
        }
    }

    /// A spec with `checkout <branch>` whose `branch` arg is generator-backed.
    fn checkout_with_generator() -> Subcommand {
        Subcommand {
            name: "git".into(),
            description: None,
            subcommands: vec![Subcommand {
                name: "checkout".into(),
                description: None,
                subcommands: vec![],
                options: vec![],
                args: vec![types::Arg {
                    name: Some("branch".into()),
                    description: None,
                    is_optional: false,
                    is_variadic: false,
                    template: None,
                    suggestions: vec![],
                    generator: Some(types::Generator {
                        run: vec!["git".into(), "branch".into()],
                        split_on: None,
                        trim: None,
                        extract: None,
                        priority: Some(80),
                        cache: None,
                    }),
                    is_command: false,
                }],
                requires_subcommand: false,
                parser_directives: None,
            }],
            options: vec![],
            args: vec![],
            requires_subcommand: true,
            parser_directives: None,
        }
    }

    #[test]
    fn generator_suggestions_flow_through_ranking() {
        let spec = checkout_with_generator();
        let runner = MockRunner::ok(&["main", "feature/login", "feature/cache"]);
        let line = "git checkout fea";
        let items = complete_line_with_generators(
            &spec,
            line,
            line.len(),
            std::path::Path::new("."),
            &runner,
        );
        assert_eq!(runner.calls.get(), 1, "runner must be invoked once");
        // Only the prefix-matching branches survive ranking, best first.
        assert!(items.iter().any(|i| i.insert == "feature/login"));
        assert!(items.iter().any(|i| i.insert == "feature/cache"));
        assert!(
            items.iter().all(|i| i.insert != "main"),
            "non-matching branch must be filtered: {items:?}"
        );
    }

    #[test]
    fn generator_failure_degrades_to_no_candidates() {
        let spec = checkout_with_generator();
        let runner = MockRunner::err(GeneratorError::Timeout);
        let line = "git checkout fea";
        let items = complete_line_with_generators(
            &spec,
            line,
            line.len(),
            std::path::Path::new("."),
            &runner,
        );
        assert!(
            items.is_empty(),
            "a failing generator must not produce candidates: {items:?}"
        );
    }

    #[test]
    fn pure_complete_line_never_invokes_generator() {
        // The pure entry point must not surface generator values at all.
        let spec = checkout_with_generator();
        let line = "git checkout fea";
        let items = complete_line(&spec, line, line.len(), std::path::Path::new("."));
        assert!(
            items.iter().all(|i| !i.insert.starts_with("feature")),
            "pure path must not run generators: {items:?}"
        );
    }
}
