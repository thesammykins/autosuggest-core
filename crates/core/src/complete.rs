//! Completion candidate collection (`TECH.md §3.1`).
//!
//! Given a [`ParseState`] from [`crate::parse`], this module gathers the raw
//! candidate suggestions valid at the cursor:
//!
//! - **Subcommands** of the active node (when the cursor is a subcommand slot).
//! - **Options** valid in the current state: not already seen (unless
//!   repeatable), satisfying `dependsOn`, not blocked by `exclusiveOn`, plus
//!   `isPersistent` options inherited from ancestors.
//! - **Argument suggestions**: static `suggestions` as-is; `filepaths`/`folders`
//!   templates read from the filesystem relative to `cwd` (the one allowed I/O,
//!   delegated to [`crate::fs_source`]).
//!
//! Generators are **not** executed in M1 (that is M4); an arg whose only source
//! is a `generator` contributes no dynamic items here. The hook is marked below.
//!
//! Output [`Candidate`]s are unranked; [`crate::rank`] filters and scores them.

use std::path::Path;

use crate::fs_source::{self, FsKind};
use crate::parse::{CursorKind, ParseState};
use crate::types::{Arg, Opt, Subcommand, Template};

/// A single unranked completion candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Text to insert when accepted.
    pub insert: String,
    /// Display label (defaults to the insert text when absent).
    pub display: Option<String>,
    /// Short description shown alongside the candidate.
    pub desc: Option<String>,
    /// Authored priority `0..=100` (default 50 applied by ranking).
    pub priority: Option<u8>,
    /// Host may warn before accepting (e.g. `rm -rf`).
    pub dangerous: bool,
    /// Marked deprecated.
    pub deprecated: bool,
    /// Excluded unless the query matches the name exactly.
    pub hidden: bool,
    /// The names this candidate matches against (for ranking). The first is the
    /// primary name; option candidates list all forms so any can be matched.
    pub match_names: Vec<String>,
}

impl Candidate {
    fn simple(insert: impl Into<String>) -> Self {
        let insert = insert.into();
        Candidate {
            match_names: vec![insert.clone()],
            insert,
            display: None,
            desc: None,
            priority: None,
            dangerous: false,
            deprecated: false,
            hidden: false,
        }
    }
}

/// Collect all candidates valid for `state`, reading the filesystem relative to
/// `cwd` for path templates.
///
/// `query` is the in-progress cursor token (e.g. `src/ma` when completing a
/// nested path); it selects which directory path templates read from. For an
/// inline `--opt=value` form the parser's `inline_value_prefix` takes
/// precedence as the path partial.
pub fn collect(state: &ParseState, query: &str, cwd: &Path) -> Vec<Candidate> {
    match &state.cursor {
        CursorKind::Subcommand => {
            let mut out = subcommand_candidates(state.active());
            // Options are still offerable in a subcommand slot if the user has
            // started typing a dash; that case is classified as `Option`, so
            // here we additionally surface any positional arg of the node (some
            // commands accept both a subcommand and a positional). Kept minimal:
            // include args only when the node declares them at index 0.
            out.extend(option_candidates(state));
            out
        }
        CursorKind::Option => option_candidates(state),
        CursorKind::OptionArgument(opt) => option_argument_candidates(state, opt, query, cwd),
        CursorKind::CommandArgument(arg) => command_argument_candidates(arg, state, query, cwd),
        CursorKind::Empty => Vec::new(),
    }
}

/// Subcommand name candidates for the active node.
fn subcommand_candidates(node: &Subcommand) -> Vec<Candidate> {
    node.subcommands
        .iter()
        .map(|sc| {
            let canonical = sc.name.canonical().to_string();
            Candidate {
                insert: format!("{canonical} "),
                display: Some(canonical.clone()),
                desc: sc.description.clone(),
                priority: None,
                dangerous: false,
                deprecated: false,
                hidden: false,
                // Match against all aliases so `co` finds `checkout`.
                match_names: sc.name.all().to_vec(),
            }
        })
        .collect()
}

/// Option candidates valid in the current state.
fn option_candidates(state: &ParseState) -> Vec<Candidate> {
    let active = state.active();
    let mut out = Vec::new();

    // Local options on the active node, then inherited persistent options.
    let locals = active.options.iter();
    let persistents = state
        .persistent_options
        .iter()
        .filter(|p| !active.options.iter().any(|o| o.name == p.name));

    for opt in locals.chain(persistents) {
        if !option_is_valid(opt, state) {
            continue;
        }
        out.push(option_to_candidate(opt));
    }
    out
}

/// Whether an option may still be offered given what has been seen.
fn option_is_valid(opt: &Opt, state: &ParseState) -> bool {
    let canonical = opt.name.canonical();

    // Drop non-repeatable options already seen.
    let already_seen = state.seen_options.contains(canonical);
    let repeatable = match opt.is_repeatable {
        crate::types::Repeatable::Flag(b) => b,
        crate::types::Repeatable::Max(n) => n > 1,
    };
    if already_seen && !repeatable {
        return false;
    }

    // `exclusiveOn`: hidden if any conflicting option is present.
    for ex in &opt.exclusive_on {
        if state.seen_options.contains(ex) {
            return false;
        }
    }

    // `dependsOn`: only offer once all dependencies are present.
    for dep in &opt.depends_on {
        if !state.seen_options.contains(dep) {
            return false;
        }
    }

    true
}

/// Map an [`Opt`] to a [`Candidate`]. The canonical (first) form is inserted;
/// all forms are matchable so typing `--al` finds `["-a","--all"]`.
fn option_to_candidate(opt: &Opt) -> Candidate {
    let canonical = opt.name.canonical().to_string();
    // Options that take a value get a trailing space so the value can follow;
    // options requiring a separator get the `=` appended instead.
    let insert = if opt.requires_separator && !opt.args.is_empty() {
        format!("{canonical}=")
    } else if !opt.args.is_empty() {
        format!("{canonical} ")
    } else {
        canonical.clone()
    };

    Candidate {
        insert,
        display: Some(canonical),
        desc: opt.description.clone(),
        priority: None,
        dangerous: false,
        deprecated: false,
        hidden: false,
        match_names: opt.name.all().to_vec(),
    }
}

/// Candidates for an option's argument value.
fn option_argument_candidates(
    state: &ParseState,
    opt: &Opt,
    query: &str,
    cwd: &Path,
) -> Vec<Candidate> {
    // An option may declare one arg (its value); use the first.
    let Some(arg) = opt.args.first() else {
        return Vec::new();
    };
    arg_candidates(arg, state, query, cwd)
}

/// Candidates for a positional command argument.
fn command_argument_candidates(
    arg: &Arg,
    state: &ParseState,
    query: &str,
    cwd: &Path,
) -> Vec<Candidate> {
    arg_candidates(arg, state, query, cwd)
}

/// Shared arg-suggestion logic: static suggestions + template filesystem reads.
fn arg_candidates(arg: &Arg, state: &ParseState, query: &str, cwd: &Path) -> Vec<Candidate> {
    let mut out = Vec::new();

    // Static suggestions, included as-is.
    for sug in &arg.suggestions {
        let canonical = sug.name.canonical().to_string();
        let insert = sug
            .insert_value
            .clone()
            .unwrap_or_else(|| canonical.clone());
        out.push(Candidate {
            insert,
            display: sug.display_name.clone().or_else(|| Some(canonical.clone())),
            desc: sug.description.clone(),
            priority: sug.priority,
            dangerous: sug.is_dangerous.unwrap_or(false),
            deprecated: sug.deprecated.unwrap_or(false),
            hidden: sug.hidden.unwrap_or(false),
            match_names: sug.name.all().to_vec(),
        });
    }

    // Template-driven filesystem suggestions (the one allowed I/O).
    if let Some(template) = arg.template {
        // The path partial selects which directory to read. For an inline
        // `--opt=value` form the parser captured the value after `=`; otherwise
        // the cursor query itself is the partial (e.g. `src/ma`). `fs_source`
        // reads the directory component and filters on the file component, and
        // ranking re-filters the bare entry names against the same query.
        let partial = state
            .inline_value_prefix
            .as_deref()
            .filter(|p| !p.is_empty())
            .unwrap_or(query);
        match template {
            Template::Filepaths => {
                out.extend(fs_to_candidates(fs_source::list_entries(
                    cwd,
                    partial,
                    FsKind::FilesAndDirs,
                )));
            }
            Template::Folders => {
                out.extend(fs_to_candidates(fs_source::list_entries(
                    cwd,
                    partial,
                    FsKind::DirsOnly,
                )));
            }
            // `history` template is an M2 concern; no items in M1.
            Template::History => {}
        }
    }

    // M4 hook: if `arg.generator` is set we would execute it through the
    // injected `GeneratorRunner` here. In M1 generators contribute nothing.
    // TODO(M4): run arg.generator via GeneratorRunner and extend `out`.
    let _ = &arg.generator;

    out
}

/// Convert filesystem entries into candidates. Directories get a small priority
/// boost so they sort ahead of files of equal match quality.
///
/// The match name is the full insert path (e.g. `src/main.rs`) so a nested
/// query like `src/ma` prefix-matches in ranking; the display stays the bare
/// entry name.
fn fs_to_candidates(entries: Vec<fs_source::FsEntry>) -> Vec<Candidate> {
    entries
        .into_iter()
        .map(|e| {
            let mut c = Candidate::simple(e.insert.clone());
            c.display = Some(e.display);
            c.match_names = vec![e.insert];
            c.priority = Some(if e.is_dir { 55 } else { 50 });
            c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::tokenize::tokenize;
    use crate::types::{ParserDirectives, Repeatable, StringList};

    fn opt(names: &[&str]) -> Opt {
        Opt {
            name: StringList::Many(names.iter().map(|s| s.to_string()).collect()),
            description: None,
            args: vec![],
            is_required: false,
            is_repeatable: Repeatable::Flag(false),
            is_persistent: false,
            requires_separator: false,
            exclusive_on: vec![],
            depends_on: vec![],
        }
    }

    fn run(spec: &Subcommand, line: &str, cwd: &Path) -> Vec<Candidate> {
        let toks = tokenize(line, line.len());
        let st = parse(spec, toks.committed(), toks.query());
        collect(&st, toks.query(), cwd)
    }

    fn ls_spec() -> Subcommand {
        Subcommand {
            name: "ls".into(),
            description: None,
            subcommands: vec![],
            options: vec![opt(&["-l"]), opt(&["-a", "--all"]), {
                let mut h = opt(&["-h"]);
                h.depends_on = vec!["-l".into()];
                h
            }],
            args: vec![],
            requires_subcommand: false,
            parser_directives: Some(ParserDirectives {
                flags_are_posix_noncompliant: true,
                options_must_precede_arguments: false,
                option_arg_separators: vec!["=".into(), " ".into()],
            }),
        }
    }

    #[test]
    fn offers_options_after_dash() {
        let spec = ls_spec();
        let c = run(&spec, "ls -", Path::new("/tmp"));
        let inserts: Vec<&str> = c.iter().map(|x| x.insert.as_str()).collect();
        assert!(inserts.contains(&"-l"));
        assert!(inserts.contains(&"-a"));
    }

    #[test]
    fn seen_option_is_dropped() {
        let spec = ls_spec();
        let toks = tokenize("ls -l -", 7);
        let st = parse(&spec, toks.committed(), toks.query());
        let c = collect(&st, toks.query(), Path::new("/tmp"));
        assert!(c.iter().all(|x| x.insert != "-l"));
    }

    #[test]
    fn depends_on_hides_until_satisfied() {
        let spec = ls_spec();
        // Without -l, -h should not be offered.
        let c = run(&spec, "ls -", Path::new("/tmp"));
        assert!(c.iter().all(|x| x.insert != "-h"));
        // With -l, -h appears.
        let toks = tokenize("ls -l -", 7);
        let st = parse(&spec, toks.committed(), toks.query());
        let c2 = collect(&st, toks.query(), Path::new("/tmp"));
        assert!(c2.iter().any(|x| x.insert == "-h"));
    }

    #[test]
    fn subcommand_candidates_use_canonical_and_aliases() {
        let git = Subcommand {
            name: "git".into(),
            description: None,
            subcommands: vec![Subcommand {
                name: StringList::Many(vec!["checkout".into(), "co".into()]),
                description: Some("Switch branches".into()),
                subcommands: vec![],
                options: vec![],
                args: vec![],
                requires_subcommand: false,
                parser_directives: None,
            }],
            options: vec![],
            args: vec![],
            requires_subcommand: true,
            parser_directives: None,
        };
        let c = run(&git, "git ", Path::new("/tmp"));
        let co = c.iter().find(|x| x.display.as_deref() == Some("checkout"));
        let co = co.expect("checkout present");
        assert_eq!(co.insert, "checkout ");
        assert!(co.match_names.contains(&"co".to_string()));
    }

    #[test]
    fn static_arg_suggestions_included() {
        let mut color = opt(&["--color"]);
        color.requires_separator = true;
        color.args = vec![Arg {
            name: Some("when".into()),
            description: None,
            is_optional: false,
            is_variadic: false,
            template: None,
            suggestions: vec!["auto".into(), "always".into(), "never".into()],
            generator: None,
            is_command: false,
        }];
        let spec = Subcommand {
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
        let c = run(&spec, "ls --color=", Path::new("/tmp"));
        let inserts: Vec<&str> = c.iter().map(|x| x.insert.as_str()).collect();
        assert!(inserts.contains(&"auto"));
        assert!(inserts.contains(&"never"));
    }
}
