//! Spec-tree parser / parse-state machine (`TECH.md §3.2`).
//!
//! Consumes the committed tokens (everything before the cursor) left-to-right
//! against a [`Subcommand`] spec tree and produces a [`ParseState`] describing
//! the resolved command path, the options already seen, any option awaiting its
//! value, the current positional index, and — crucially — how the *cursor token*
//! should be classified ([`CursorKind`]). The `complete` stage uses that
//! classification to decide which candidate sets are valid.
//!
//! This module is **pure**: no I/O. It only inspects the spec and the tokens.
//!
//! ## Honoured parser directives (`SCHEMA.md §1.6`)
//!
//! - `flagsArePosixNoncompliant`: short flags may chain, e.g. `-lah` == `-l -a
//!   -h`. When off, an unknown `-xyz` is treated as a single (long-ish) token.
//! - `optionsMustPrecedeArguments`: once a positional argument is consumed,
//!   later `-`-prefixed tokens are treated as arguments, not options.
//! - `optionArgSeparators`: separators between an option and its value, e.g. `=`
//!   for `--color=auto`. A bare space is always an implicit separator unless the
//!   option `requiresSeparator`.

use std::collections::BTreeSet;

use crate::tokenize::Token;
use crate::types::{Arg, Opt, ParserDirectives, Subcommand};

/// Classification of the cursor token — what the user is currently typing.
#[derive(Debug, Clone, PartialEq)]
pub enum CursorKind {
    /// A subcommand name is expected here (the active node has subcommands and
    /// no positional has been consumed yet).
    Subcommand,
    /// An option/flag (the query starts with `-`, or options are otherwise the
    /// dominant candidate).
    Option,
    /// The value for the given option (the previous token was an option needing
    /// an argument, or an inline `--opt=` prefix is being completed).
    OptionArgument(Opt),
    /// A positional argument for the active command, identified by its [`Arg`].
    CommandArgument(Arg),
    /// Nothing specific is expected (no subcommands, no args, no pending option).
    Empty,
}

/// The outcome of walking tokens against a spec.
#[derive(Debug, Clone)]
pub struct ParseState {
    /// Resolved spec node chain: `root -> subcommand -> ...`. The last element
    /// is the active command whose candidates apply.
    pub command_path: Vec<Subcommand>,
    /// Canonical names of options already seen on the active path (used to drop
    /// non-repeatable options and to evaluate `dependsOn`/`exclusiveOn`).
    pub seen_options: BTreeSet<String>,
    /// Persistent options inherited from ancestor nodes (`isPersistent`), valid
    /// at every descendant.
    pub persistent_options: Vec<Opt>,
    /// Index of the next positional argument to fill on the active command.
    pub arg_index: usize,
    /// How the cursor token should be completed.
    pub cursor: CursorKind,
    /// When `cursor` is [`CursorKind::OptionArgument`] for an inline
    /// `--opt=value` form, the already-typed value prefix (after the separator).
    pub inline_value_prefix: Option<String>,
    /// The root spec node. Retained so [`ParseState::active`] is total even if
    /// `command_path` is ever empty, without resorting to panics. Mirrors
    /// `command_path[0]` after a normal parse.
    root: Subcommand,
}

impl ParseState {
    /// The active (deepest resolved) command node.
    ///
    /// After a normal [`parse`] this is the last element of `command_path`
    /// (which always begins with the root); the stored [`ParseState::root`]
    /// is a total fallback that avoids any panic.
    pub fn active(&self) -> &Subcommand {
        self.command_path.last().unwrap_or(&self.root)
    }
}

/// Walk `tokens` (the committed tokens, cursor token excluded) against `root`,
/// then classify the in-progress `query` (the cursor-token prefix).
///
/// `query` is the unquoted text of the cursor token before the cursor; it is
/// empty when a brand-new token is being started.
///
/// The first committed token is `argv[0]` — the command name itself, which
/// `root` already represents — so it is skipped rather than matched against the
/// root's own subcommands/args.
pub fn parse(root: &Subcommand, committed: &[Token], query: &str) -> ParseState {
    let mut path: Vec<Subcommand> = vec![root.clone()];
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut persistent: Vec<Opt> = collect_persistent(root);
    let mut arg_index: usize = 0;
    let mut positional_started = false;
    // An option still awaiting its value (space-separated form).
    let mut pending_arg: Option<Opt> = None;

    // Skip argv[0] (the command name); `root` is that command.
    for tok in committed.iter().skip(1) {
        let text = tok.text.as_str();

        // 1. If an option is awaiting a value, this token is consumed as that
        //    value (unless it is itself clearly a new option in a strict world;
        //    we follow the common shell behaviour: the value is taken verbatim).
        if pending_arg.take().is_some() {
            continue;
        }

        let directives = active_directives(&path);
        let options_first = directives
            .as_ref()
            .map(|d| d.options_must_precede_arguments)
            .unwrap_or(false);

        // 2. Option token?
        let is_option_like = text.starts_with('-') && text != "-" && text != "--";
        let treat_as_option = is_option_like && !(options_first && positional_started);

        if treat_as_option {
            consume_option(
                &path,
                &persistent,
                directives.as_ref(),
                text,
                &mut seen,
                &mut pending_arg,
            );
            continue;
        }

        // 3. Subcommand token? Only when the active node has subcommands and no
        //    positional has been consumed yet.
        if !positional_started {
            if let Some(child) = find_subcommand(path.last(), text) {
                // Descend: reset positional index, accumulate persistent opts.
                persistent.extend(collect_persistent(&child));
                path.push(child);
                arg_index = 0;
                continue;
            }
        }

        // 4. Otherwise it is a positional argument value.
        positional_started = true;
        advance_arg_index(path.last(), &mut arg_index);
    }

    // Now classify the cursor token (the in-progress `query`).
    let directives = active_directives(&path);
    let cursor = classify_cursor(
        &path,
        &persistent,
        directives.as_ref(),
        &seen,
        pending_arg.as_ref(),
        positional_started,
        arg_index,
        query,
    );

    // For an inline `--opt=val` query, capture the typed value prefix.
    let inline_value_prefix = inline_value_of(query);

    ParseState {
        command_path: path,
        seen_options: seen,
        persistent_options: persistent,
        arg_index,
        cursor,
        inline_value_prefix,
        root: root.clone(),
    }
}

/// Collect this node's directly-declared persistent options.
fn collect_persistent(node: &Subcommand) -> Vec<Opt> {
    node.options
        .iter()
        .filter(|o| o.is_persistent)
        .cloned()
        .collect()
}

/// The active node's parser directives, if any.
fn active_directives(path: &[Subcommand]) -> Option<ParserDirectives> {
    // Directives are declared on the root (and conceptually apply to the tree);
    // a subcommand may override. Prefer the deepest node that declares them.
    path.iter().rev().find_map(|n| n.parser_directives.clone())
}

/// Find a subcommand of `parent` whose name or alias equals `text`.
fn find_subcommand(parent: Option<&Subcommand>, text: &str) -> Option<Subcommand> {
    parent.and_then(|p| {
        p.subcommands
            .iter()
            .find(|s| s.name.all().iter().any(|n| n == text))
            .cloned()
    })
}

/// Advance `arg_index` past the just-consumed positional, unless the current
/// arg is variadic (which keeps consuming).
fn advance_arg_index(node: Option<&Subcommand>, arg_index: &mut usize) {
    if let Some(node) = node {
        if let Some(arg) = node.args.get(*arg_index) {
            if !arg.is_variadic {
                *arg_index += 1;
            }
        } else {
            *arg_index += 1;
        }
    } else {
        *arg_index += 1;
    }
}

/// Record an option token in `seen`, expanding chained short flags when the
/// directives allow, and set `pending_arg` if the option needs a value that was
/// not provided inline.
fn consume_option(
    path: &[Subcommand],
    persistent: &[Opt],
    directives: Option<&ParserDirectives>,
    text: &str,
    seen: &mut BTreeSet<String>,
    pending_arg: &mut Option<Opt>,
) {
    // Split an inline value if a separator is present (e.g. `--color=auto`).
    let separators = directives
        .map(|d| d.option_arg_separators.clone())
        .unwrap_or_default();

    if let Some((flag, _value)) = split_inline(text, &separators) {
        if let Some(opt) = lookup_option(path, persistent, flag) {
            seen.insert(canonical_opt(&opt));
        }
        return;
    }

    // Chained short flags: only for `-xyz` (single dash, len > 2) when allowed.
    let chaining = directives
        .map(|d| d.flags_are_posix_noncompliant)
        .unwrap_or(false);
    let is_short_cluster =
        chaining && text.starts_with('-') && !text.starts_with("--") && text.len() > 2;

    if is_short_cluster {
        // Expand `-lah` -> `-l`, `-a`, `-h`. The last flag in the cluster may
        // take an argument; if so it becomes pending.
        let chars: Vec<char> = text[1..].chars().collect();
        for (idx, c) in chars.iter().enumerate() {
            let single = format!("-{c}");
            if let Some(opt) = lookup_option(path, persistent, &single) {
                seen.insert(canonical_opt(&opt));
                let last = idx + 1 == chars.len();
                if last && !opt.args.is_empty() {
                    *pending_arg = Some(opt);
                }
            }
        }
        return;
    }

    // Plain single option.
    if let Some(opt) = lookup_option(path, persistent, text) {
        seen.insert(canonical_opt(&opt));
        if !opt.args.is_empty() && !opt.requires_separator {
            *pending_arg = Some(opt);
        }
    }
}

/// The canonical (first) name of an option, used as the `seen` key.
fn canonical_opt(opt: &Opt) -> String {
    opt.name.canonical().to_string()
}

/// Find an option by any of its name forms across the active node's options and
/// inherited persistent options.
fn lookup_option(path: &[Subcommand], persistent: &[Opt], form: &str) -> Option<Opt> {
    if let Some(node) = path.last() {
        if let Some(o) = node
            .options
            .iter()
            .find(|o| o.name.all().iter().any(|n| n == form))
        {
            return Some(o.clone());
        }
    }
    persistent
        .iter()
        .find(|o| o.name.all().iter().any(|n| n == form))
        .cloned()
}

/// If `text` contains one of `separators` (and is option-like), split into
/// `(flag, value)`. Only `=` style separators are meaningful inline; a bare
/// space never appears within a single token.
fn split_inline<'a>(text: &'a str, separators: &[String]) -> Option<(&'a str, &'a str)> {
    for sep in separators {
        if sep == " " || sep.is_empty() {
            continue;
        }
        if let Some(pos) = text.find(sep.as_str()) {
            // Must look like an option (`-x=...` / `--long=...`).
            if text.starts_with('-') {
                let (flag, rest) = text.split_at(pos);
                let value = &rest[sep.len()..];
                return Some((flag, value));
            }
        }
    }
    None
}

/// Extract the already-typed value prefix from an inline `--opt=val` query.
fn inline_value_of(query: &str) -> Option<String> {
    if !query.starts_with('-') {
        return None;
    }
    // Default separators include `=`; we accept `=` for inline value completion.
    query.find('=').map(|pos| query[pos + 1..].to_string())
}

/// Decide what the cursor token is.
#[allow(clippy::too_many_arguments)]
fn classify_cursor(
    path: &[Subcommand],
    persistent: &[Opt],
    directives: Option<&ParserDirectives>,
    _seen: &BTreeSet<String>,
    pending_arg: Option<&Opt>,
    positional_started: bool,
    arg_index: usize,
    query: &str,
) -> CursorKind {
    // 1. A space-separated option value is the strongest signal.
    if let Some(opt) = pending_arg {
        return CursorKind::OptionArgument(opt.clone());
    }

    // 2. Inline `--opt=<value>` completion.
    if query.starts_with('-') {
        if let Some(eq) = query.find('=') {
            let flag = &query[..eq];
            if let Some(opt) = lookup_option(path, persistent, flag) {
                if !opt.args.is_empty() {
                    return CursorKind::OptionArgument(opt);
                }
            }
        }
        return CursorKind::Option;
    }

    let active = path.last();
    let has_subcommands = active.map(|n| !n.subcommands.is_empty()).unwrap_or(false);

    // 3. Subcommand position: active node has subcommands and we have not yet
    //    consumed a positional argument.
    if has_subcommands && !positional_started {
        // If the active node also has args at this index we still prefer
        // subcommands first; complete will include args too if appropriate.
        return CursorKind::Subcommand;
    }

    // 4. Positional argument.
    if let Some(node) = active {
        if let Some(arg) = node.args.get(arg_index) {
            return CursorKind::CommandArgument(arg.clone());
        }
        // Variadic last arg keeps applying past its index.
        if let Some(last) = node.args.last() {
            if last.is_variadic && arg_index >= node.args.len() {
                return CursorKind::CommandArgument(last.clone());
            }
        }
    }

    let _ = directives;
    CursorKind::Empty
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tokenize::tokenize;
    use crate::types::StringList;

    fn opt(names: &[&str]) -> Opt {
        Opt {
            name: StringList::Many(names.iter().map(|s| s.to_string()).collect()),
            description: None,
            args: vec![],
            is_required: false,
            is_repeatable: crate::types::Repeatable::Flag(false),
            is_persistent: false,
            requires_separator: false,
            exclusive_on: vec![],
            depends_on: vec![],
        }
    }

    fn parse_line(root: &Subcommand, line: &str) -> ParseState {
        let cursor = line.len();
        let toks = tokenize(line, cursor);
        parse(root, toks.committed(), toks.query())
    }

    fn ls_spec() -> Subcommand {
        Subcommand {
            name: "ls".into(),
            description: None,
            subcommands: vec![],
            options: vec![opt(&["-l"]), opt(&["-a", "--all"]), opt(&["-h"])],
            args: vec![Arg {
                name: Some("file".into()),
                description: None,
                is_optional: true,
                is_variadic: true,
                template: Some(crate::types::Template::Filepaths),
                suggestions: vec![],
                generator: None,
                is_command: false,
            }],
            requires_subcommand: false,
            parser_directives: Some(ParserDirectives {
                flags_are_posix_noncompliant: true,
                options_must_precede_arguments: false,
                option_arg_separators: vec!["=".into(), " ".into()],
            }),
        }
    }

    #[test]
    fn classifies_first_token_as_subcommand_or_option() {
        let spec = ls_spec();
        let st = parse_line(&spec, "ls -");
        assert_eq!(st.cursor, CursorKind::Option);
    }

    #[test]
    fn chained_short_flags_are_all_seen() {
        let spec = ls_spec();
        let toks = tokenize("ls -la ", 7);
        let st = parse(&spec, toks.committed(), toks.query());
        assert!(st.seen_options.contains("-l"));
        assert!(st.seen_options.contains("-a"));
    }

    #[test]
    fn empty_query_after_command_is_argument_when_no_subcommands() {
        let spec = ls_spec();
        let st = parse_line(&spec, "ls ");
        match st.cursor {
            CursorKind::CommandArgument(a) => assert_eq!(a.name.as_deref(), Some("file")),
            other => panic!("expected arg, got {other:?}"),
        }
    }

    #[test]
    fn subcommand_resolution_and_aliases() {
        let git = Subcommand {
            name: "git".into(),
            description: None,
            subcommands: vec![Subcommand {
                name: StringList::Many(vec!["checkout".into(), "co".into()]),
                description: None,
                subcommands: vec![],
                options: vec![opt(&["-b"])],
                args: vec![],
                requires_subcommand: false,
                parser_directives: None,
            }],
            options: vec![],
            args: vec![],
            requires_subcommand: true,
            parser_directives: None,
        };
        // Alias `co` resolves to the checkout node.
        let toks = tokenize("git co ", 7);
        let st = parse(&git, toks.committed(), toks.query());
        assert_eq!(st.active().name.canonical(), "checkout");
    }

    #[test]
    fn option_with_space_value_is_pending() {
        let mut o = opt(&["-m", "--message"]);
        o.args = vec![Arg {
            name: Some("msg".into()),
            description: None,
            is_optional: false,
            is_variadic: false,
            template: None,
            suggestions: vec![],
            generator: None,
            is_command: false,
        }];
        let spec = Subcommand {
            name: "commit".into(),
            description: None,
            subcommands: vec![],
            options: vec![o],
            args: vec![],
            requires_subcommand: false,
            parser_directives: None,
        };
        let st = parse_line(&spec, "commit -m ");
        assert!(matches!(st.cursor, CursorKind::OptionArgument(_)));
    }

    #[test]
    fn inline_separator_value_classified() {
        let mut o = opt(&["--color"]);
        o.requires_separator = true;
        o.args = vec![Arg {
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
            options: vec![o],
            args: vec![],
            requires_subcommand: false,
            parser_directives: Some(ParserDirectives {
                flags_are_posix_noncompliant: true,
                options_must_precede_arguments: false,
                option_arg_separators: vec!["=".into(), " ".into()],
            }),
        };
        let st = parse_line(&spec, "ls --color=al");
        assert!(matches!(st.cursor, CursorKind::OptionArgument(_)));
        assert_eq!(st.inline_value_prefix.as_deref(), Some("al"));
    }

    #[test]
    fn options_must_precede_arguments_flips_later_dashes() {
        let spec = Subcommand {
            name: "tool".into(),
            description: None,
            subcommands: vec![],
            options: vec![opt(&["-x"])],
            args: vec![Arg {
                name: Some("f".into()),
                description: None,
                is_optional: false,
                is_variadic: true,
                template: None,
                suggestions: vec![],
                generator: None,
                is_command: false,
            }],
            requires_subcommand: false,
            parser_directives: Some(ParserDirectives {
                flags_are_posix_noncompliant: false,
                options_must_precede_arguments: true,
                option_arg_separators: vec!["=".into(), " ".into()],
            }),
        };
        // After a positional, `-x` is not treated as an option.
        let toks = tokenize("tool file -x ", 13);
        let st = parse(&spec, toks.committed(), toks.query());
        assert!(!st.seen_options.contains("-x"));
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        let spec = ls_spec();
        for line in [
            "",
            "   ",
            "-",
            "--",
            "ls ''",
            "ls \"",
            "ls -\\",
            "ls -la -- x",
        ] {
            let toks = tokenize(line, line.len());
            let _ = parse(&spec, toks.committed(), toks.query());
        }
    }
}
