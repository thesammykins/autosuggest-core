//! Failed-command correction rule engine + native predicates (`TECH.md §3.5`).
//!
//! Given the context of a failed command (`script`, `stderr`, `exit_code`,
//! `cwd`, `env`) the engine evaluates every correction rule and returns ranked
//! corrected command strings.
//!
//! There are two rule kinds (`SCHEMA.md §2`):
//!
//! * **JSON rules** with a [`rule::Match`] predicate set (logical AND of present
//!   conditions) and exactly one [`rule::Rewrite`] strategy.
//! * **Native rules** (`SCHEMA.md §2.1`) whose logic lives in [`native`], keyed
//!   by id (`no_command`, `subcommand_typo`).
//!
//! ## Pipeline
//!
//! 1. For each rule whose `match` holds (native rules always "match" — their own
//!    predicate decides whether they emit), produce 0+ candidate rewrites.
//! 2. Each candidate is tagged with the originating rule's id, description, and
//!    priority.
//! 3. Deduplicate by corrected command string, keeping the highest-priority
//!    origin.
//! 4. Rank by rule `priority` descending, ties broken by rule `id` then by the
//!    corrected string, and return the top `limit`.
//!
//! `core` stays pure: the only host capability needed — probing `$PATH` — is
//! injected via [`resolver::CommandResolver`] (see that module), mirroring how
//! generator execution is injected via [`crate::GeneratorRunner`].

pub mod levenshtein;
pub mod native;
pub mod resolver;
pub mod rule;

use regex::Regex;

use crate::types::Subcommand;
use resolver::CommandResolver;
use rule::{Match, Rewrite, Rule};

pub use resolver::{CommandResolver as Resolver, MockCommandResolver};

#[cfg(feature = "std-resolver")]
pub use resolver::PathCommandResolver;

/// Default number of corrections returned by [`correct`].
pub const DEFAULT_LIMIT: usize = 5;

/// Input context for a correction request (`TECH.md §3.5`).
///
/// Borrows its inputs so the engine allocates nothing for context. `specs` are
/// the loaded command specs, consulted only by the native `subcommand_typo`
/// predicate; pass an empty slice if unavailable.
#[derive(Debug, Clone, Copy)]
pub struct CorrectContext<'a> {
    /// The script/command line that failed.
    pub script: &'a str,
    /// Captured stderr (empty string if none).
    pub stderr: &'a str,
    /// Exit code of the failed command, if known.
    pub exit_code: Option<i32>,
    /// Working directory, if provided.
    pub cwd: Option<&'a str>,
    /// Environment hints, if provided.
    pub env: Option<&'a std::collections::BTreeMap<String, String>>,
    /// Loaded command specs (for `subcommand_typo`).
    pub specs: &'a [Subcommand],
}

impl<'a> CorrectContext<'a> {
    /// Build a context from the core fields, defaulting the rest.
    ///
    /// `specs` defaults to empty; set [`CorrectContext::specs`] afterwards if the
    /// `subcommand_typo` native rule should be able to consult a spec.
    pub fn new(script: &'a str, stderr: &'a str, exit_code: Option<i32>) -> Self {
        Self {
            script,
            stderr,
            exit_code,
            cwd: None,
            env: None,
            specs: &[],
        }
    }
}

/// A single proposed correction with its provenance, ready for display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectedCommand {
    /// The corrected command line.
    pub command: String,
    /// Id of the rule that produced it.
    pub rule_id: String,
    /// Description of the rule, if any (for display).
    pub description: Option<String>,
    /// Priority of the originating rule (`0..=100`), used for ranking.
    pub priority: u8,
}

/// Errors that can occur while building or running the correction engine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CorrectError {
    /// A rule contained an invalid regular expression.
    InvalidRegex {
        /// The owning rule id.
        rule_id: String,
        /// The offending pattern.
        pattern: String,
        /// The compiler error message.
        message: String,
    },
    /// A native rule id has no registered implementation.
    UnknownNativeRule {
        /// The unrecognized native rule id.
        rule_id: String,
    },
}

impl core::fmt::Display for CorrectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CorrectError::InvalidRegex {
                rule_id,
                pattern,
                message,
            } => write!(
                f,
                "rule `{rule_id}` has invalid regex `{pattern}`: {message}"
            ),
            CorrectError::UnknownNativeRule { rule_id } => {
                write!(f, "no native implementation for rule id `{rule_id}`")
            }
        }
    }
}

impl std::error::Error for CorrectError {}

/// Split a script into whitespace tokens.
fn tokens(script: &str) -> Vec<String> {
    script.split_whitespace().map(str::to_string).collect()
}

/// Compile a regex, mapping failure to a typed [`CorrectError`].
fn compile(rule_id: &str, pattern: &str) -> Result<Regex, CorrectError> {
    Regex::new(pattern).map_err(|e| CorrectError::InvalidRegex {
        rule_id: rule_id.to_string(),
        pattern: pattern.to_string(),
        message: e.to_string(),
    })
}

/// Evaluate a JSON rule's [`Match`] (AND of present conditions).
///
/// An absent/empty `Match` always holds. Regex predicates may fail to compile,
/// surfaced as a typed error rather than a silent skip.
fn matches(rule_id: &str, m: &Match, ctx: &CorrectContext<'_>) -> Result<bool, CorrectError> {
    if let Some(prefix) = &m.script_starts_with {
        if !ctx.script.starts_with(prefix.as_str()) {
            return Ok(false);
        }
    }
    if let Some(pat) = &m.script_regex {
        if !compile(rule_id, pat)?.is_match(ctx.script) {
            return Ok(false);
        }
    }
    if !m.stderr_contains.is_empty()
        && !m
            .stderr_contains
            .iter()
            .any(|needle| ctx.stderr.contains(needle.as_str()))
    {
        return Ok(false);
    }
    if let Some(pat) = &m.stderr_regex {
        if !compile(rule_id, pat)?.is_match(ctx.stderr) {
            return Ok(false);
        }
    }
    if !m.exit_code_in.is_empty() {
        match ctx.exit_code {
            Some(code) if m.exit_code_in.contains(&code) => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Apply a single [`Rewrite`] to the script, returning the rewritten command(s).
///
/// Returns an empty vector when the rewrite does not apply (e.g. `insertFlag`
/// whose `after` token is absent, or the flag is already present).
fn apply_rewrite(
    rule_id: &str,
    rewrite: &Rewrite,
    ctx: &CorrectContext<'_>,
) -> Result<Vec<String>, CorrectError> {
    let result = match rewrite {
        Rewrite::InsertFlag { after, flag } => {
            let mut toks = tokens(ctx.script);
            let Some(pos) = toks.iter().position(|t| t == after) else {
                return Ok(Vec::new());
            };
            // No-op if the flag already immediately follows `after`.
            if toks.get(pos + 1).map(String::as_str) == Some(flag.as_str()) {
                return Ok(Vec::new());
            }
            toks.insert(pos + 1, flag.clone());
            vec![toks.join(" ")]
        }
        Rewrite::ReplaceToken { index, with } => {
            let mut toks = tokens(ctx.script);
            if *index >= toks.len() {
                return Ok(Vec::new());
            }
            if toks[*index] == *with {
                return Ok(Vec::new());
            }
            toks[*index] = with.clone();
            vec![toks.join(" ")]
        }
        Rewrite::Prefix(prefix) => {
            if ctx.script.starts_with(prefix.as_str()) {
                return Ok(Vec::new());
            }
            vec![format!("{prefix}{}", ctx.script)]
        }
        Rewrite::SwapSubcommand { from, to } => {
            let mut toks = tokens(ctx.script);
            let Some(pos) = toks.iter().position(|t| t == from) else {
                return Ok(Vec::new());
            };
            toks[pos] = to.clone();
            vec![toks.join(" ")]
        }
        Rewrite::RegexReplace { pattern, with } => {
            let re = compile(rule_id, pattern)?;
            let replaced = re.replace(ctx.script, with.as_str()).into_owned();
            if replaced == ctx.script {
                return Ok(Vec::new());
            }
            vec![replaced]
        }
    };
    Ok(result)
}

/// Run a native rule by id, returning its rewritten command strings.
fn run_native(
    rule_id: &str,
    ctx: &CorrectContext<'_>,
    resolver: &dyn CommandResolver,
) -> Result<Vec<String>, CorrectError> {
    match rule_id {
        "no_command" => Ok(native::no_command(ctx, resolver)),
        "subcommand_typo" => Ok(native::subcommand_typo(ctx, ctx.specs)),
        other => Err(CorrectError::UnknownNativeRule {
            rule_id: other.to_string(),
        }),
    }
}

/// Evaluate `rules` against `ctx` and return up to [`DEFAULT_LIMIT`] ranked
/// corrections. Convenience wrapper over [`correct_with_limit`].
pub fn correct(
    ctx: &CorrectContext<'_>,
    rules: &[Rule],
    resolver: &dyn CommandResolver,
) -> Result<Vec<CorrectedCommand>, CorrectError> {
    correct_with_limit(ctx, rules, resolver, DEFAULT_LIMIT)
}

/// Evaluate `rules` against `ctx` and return up to `limit` ranked corrections.
///
/// See the module docs for the full pipeline. Corrections that equal the
/// original (whitespace-normalized) script are discarded. Duplicates are removed,
/// keeping the highest-priority origin (ties broken by rule id), and the result
/// is sorted by `priority` descending, then rule id, then corrected string.
pub fn correct_with_limit(
    ctx: &CorrectContext<'_>,
    rules: &[Rule],
    resolver: &dyn CommandResolver,
    limit: usize,
) -> Result<Vec<CorrectedCommand>, CorrectError> {
    let normalized_original = tokens(ctx.script).join(" ");
    let mut candidates: Vec<CorrectedCommand> = Vec::new();

    for r in rules {
        let commands = if r.native {
            run_native(&r.id, ctx, resolver)?
        } else {
            // JSON rule: predicate must hold, then apply the single rewrite.
            let holds = match &r.match_ {
                Some(m) => matches(&r.id, m, ctx)?,
                None => true,
            };
            if !holds {
                continue;
            }
            // Honor `commandExists` inside `match` via the resolver.
            if let Some(m) = &r.match_ {
                if let Some(cmd) = &m.command_exists {
                    if !resolver.exists(cmd) {
                        continue;
                    }
                }
            }
            match &r.rewrite {
                Some(rw) => apply_rewrite(&r.id, rw, ctx)?,
                None => Vec::new(),
            }
        };

        for command in commands {
            if command == normalized_original || command.is_empty() {
                continue;
            }
            candidates.push(CorrectedCommand {
                command,
                rule_id: r.id.clone(),
                description: r.description.clone(),
                priority: r.priority,
            });
        }
    }

    // Dedupe by command, keeping the highest-priority (then lowest rule id) origin.
    candidates.sort_by(|a, b| {
        a.command
            .cmp(&b.command)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.rule_id.cmp(&b.rule_id))
    });
    candidates.dedup_by(|a, b| a.command == b.command);

    // Final ranking: priority desc, then rule id asc, then command asc.
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.rule_id.cmp(&b.rule_id))
            .then_with(|| a.command.cmp(&b.command))
    });

    candidates.truncate(limit);
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use resolver::MockCommandResolver;

    fn rule(json: &str) -> Rule {
        serde_json::from_str(json).expect("parse rule")
    }

    fn empty_resolver() -> MockCommandResolver {
        MockCommandResolver::default()
    }

    #[test]
    fn insert_flag_applies_and_dedupes() {
        let rules = vec![rule(
            r#"{ "id": "mkdir_p", "priority": 90,
                 "match": { "scriptStartsWith": "mkdir ",
                            "stderrContains": ["No such file or directory"] },
                 "rewrite": { "insertFlag": { "after": "mkdir", "flag": "-p" } } }"#,
        )];
        let ctx = CorrectContext::new("mkdir a/b", "mkdir: a: No such file or directory", Some(1));
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command, "mkdir -p a/b");
        assert_eq!(out[0].rule_id, "mkdir_p");
    }

    #[test]
    fn insert_flag_noop_when_flag_present() {
        let rules = vec![rule(
            r#"{ "id": "mkdir_p",
                 "match": { "scriptStartsWith": "mkdir " },
                 "rewrite": { "insertFlag": { "after": "mkdir", "flag": "-p" } } }"#,
        )];
        let ctx = CorrectContext::new("mkdir -p a/b", "", Some(0));
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert!(out.is_empty());
    }

    #[test]
    fn prefix_sudo_applies() {
        let rules = vec![rule(
            r#"{ "id": "sudo", "priority": 80,
                 "match": { "stderrContains": ["Permission denied"] },
                 "rewrite": { "prefix": "sudo " } }"#,
        )];
        let ctx = CorrectContext::new("apt install x", "E: Permission denied", Some(1));
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert_eq!(out[0].command, "sudo apt install x");
    }

    #[test]
    fn match_is_logical_and() {
        let rules = vec![rule(
            r#"{ "id": "r",
                 "match": { "scriptStartsWith": "mkdir ", "exitCodeIn": [1] },
                 "rewrite": { "insertFlag": { "after": "mkdir", "flag": "-p" } } }"#,
        )];
        // exit code mismatch => rule does not fire.
        let ctx = CorrectContext::new("mkdir a/b", "", Some(0));
        assert!(correct(&ctx, &rules, &empty_resolver())
            .expect("correct")
            .is_empty());
    }

    #[test]
    fn command_exists_predicate_gates_on_resolver() {
        let rules = vec![rule(
            r#"{ "id": "r",
                 "match": { "scriptStartsWith": "mkdir ", "commandExists": "mkdir" },
                 "rewrite": { "insertFlag": { "after": "mkdir", "flag": "-p" } } }"#,
        )];
        let ctx = CorrectContext::new("mkdir a/b", "", Some(1));
        // mkdir not on PATH => no fire.
        assert!(correct(&ctx, &rules, &MockCommandResolver::default())
            .expect("correct")
            .is_empty());
        // mkdir present => fires.
        let out = correct(&ctx, &rules, &MockCommandResolver::new(["mkdir"])).expect("correct");
        assert_eq!(out[0].command, "mkdir -p a/b");
    }

    #[test]
    fn ranking_is_priority_then_id() {
        let rules = vec![
            rule(
                r#"{ "id": "low", "priority": 10,
                     "rewrite": { "prefix": "a " } }"#,
            ),
            rule(
                r#"{ "id": "high", "priority": 90,
                     "rewrite": { "prefix": "b " } }"#,
            ),
        ];
        let ctx = CorrectContext::new("cmd", "", Some(1));
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert_eq!(out[0].rule_id, "high");
        assert_eq!(out[1].rule_id, "low");
    }

    #[test]
    fn invalid_regex_is_typed_error() {
        let rules = vec![rule(
            r#"{ "id": "bad", "match": { "scriptRegex": "(" },
                 "rewrite": { "prefix": "x " } }"#,
        )];
        let ctx = CorrectContext::new("cmd", "", Some(1));
        let err = correct(&ctx, &rules, &empty_resolver()).unwrap_err();
        assert!(matches!(err, CorrectError::InvalidRegex { .. }));
    }

    #[test]
    fn unknown_native_rule_is_typed_error() {
        let rules = vec![rule(r#"{ "id": "mystery", "native": true }"#)];
        let ctx = CorrectContext::new("cmd", "", Some(1));
        let err = correct(&ctx, &rules, &empty_resolver()).unwrap_err();
        assert!(matches!(err, CorrectError::UnknownNativeRule { .. }));
    }

    #[test]
    fn regex_replace_applies() {
        let rules = vec![rule(
            r#"{ "id": "grep_r",
                 "match": { "stderrContains": ["Is a directory"] },
                 "rewrite": { "regexReplace": { "pattern": "^grep ", "with": "grep -r " } } }"#,
        )];
        let ctx = CorrectContext::new("grep foo src", "grep: src: Is a directory", Some(2));
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert_eq!(out[0].command, "grep -r foo src");
    }

    /// The shipped rule set under `rules/`. These `include_str!` paths resolve
    /// relative to this file: `crates/core/src/correct/ -> ../../../../rules/`.
    fn shipped_rules() -> Vec<Rule> {
        let sources = [
            include_str!("../../../../rules/mkdir_p.rule.json"),
            include_str!("../../../../rules/sudo.rule.json"),
            include_str!("../../../../rules/cp_dir.rule.json"),
            include_str!("../../../../rules/mv_dir.rule.json"),
            include_str!("../../../../rules/rm_dir.rule.json"),
            include_str!("../../../../rules/grep_r.rule.json"),
            include_str!("../../../../rules/cd_not_dir.rule.json"),
            include_str!("../../../../rules/scp_dir.rule.json"),
            include_str!("../../../../rules/tar_gz.rule.json"),
            include_str!("../../../../rules/ssh_port_colon.rule.json"),
            include_str!("../../../../rules/brew_cask.rule.json"),
            include_str!("../../../../rules/subcommand_typo.rule.json"),
            include_str!("../../../../rules/no_command.rule.json"),
        ];
        sources
            .iter()
            .map(|s| serde_json::from_str(s).expect("shipped rule parses"))
            .collect()
    }

    #[test]
    fn shipped_rules_all_parse_with_unique_ids() {
        let rules = shipped_rules();
        let mut ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        ids.sort_unstable();
        let unique = {
            let mut u = ids.clone();
            u.dedup();
            u
        };
        assert_eq!(ids, unique, "rule ids must be unique");
        // Native rules must be ones the engine actually implements.
        for r in &rules {
            if r.native {
                assert!(
                    r.id == "no_command" || r.id == "subcommand_typo",
                    "unimplemented native rule shipped: {}",
                    r.id
                );
            }
        }
    }

    /// End-to-end case table over the shipped JSON rules. Each row is a real
    /// failure (script + stderr + exit code) and the expected top correction.
    #[test]
    fn shipped_rules_case_table() {
        struct Case {
            name: &'static str,
            script: &'static str,
            stderr: &'static str,
            exit: i32,
            expect_top: &'static str,
            expect_rule: &'static str,
        }
        let cases = [
            Case {
                name: "mkdir missing parent",
                script: "mkdir a/b/c",
                stderr: "mkdir: cannot create directory 'a/b/c': No such file or directory",
                exit: 1,
                expect_top: "mkdir -p a/b/c",
                expect_rule: "mkdir_p",
            },
            Case {
                name: "permission denied -> sudo",
                script: "systemctl restart nginx",
                stderr: "Failed to restart nginx.service: Permission denied",
                exit: 1,
                expect_top: "sudo systemctl restart nginx",
                expect_rule: "sudo",
            },
            Case {
                name: "cp directory without -r",
                script: "cp src dst",
                stderr: "cp: -r not specified; omitting directory 'src'",
                exit: 1,
                expect_top: "cp -r src dst",
                expect_rule: "cp_dir",
            },
            Case {
                name: "rm directory without -r",
                script: "rm build",
                stderr: "rm: cannot remove 'build': Is a directory",
                exit: 1,
                expect_top: "rm -r build",
                expect_rule: "rm_dir",
            },
            Case {
                name: "grep directory without -r",
                script: "grep TODO src",
                stderr: "grep: src: Is a directory",
                exit: 2,
                expect_top: "grep -r TODO src",
                expect_rule: "grep_r",
            },
            Case {
                name: "cd to a file -> parent dir",
                script: "cd src/main.rs",
                stderr: "cd: not a directory: src/main.rs",
                exit: 1,
                expect_top: "cd src",
                expect_rule: "cd_not_dir",
            },
            Case {
                name: "scp directory without -r",
                script: "scp logs host:/tmp",
                stderr: "scp: logs: not a regular file",
                exit: 1,
                expect_top: "scp -r logs host:/tmp",
                expect_rule: "scp_dir",
            },
            Case {
                name: "tar gzip not auto-detected",
                script: "tar -xf bundle.tar.gz",
                stderr: "tar: This does not look like a tar archive",
                exit: 2,
                expect_top: "tar -z -xf bundle.tar.gz",
                expect_rule: "tar_gz",
            },
            Case {
                name: "ssh host:port colon syntax",
                script: "ssh deploy@example.com:2222",
                stderr: "ssh: Could not resolve hostname example.com:2222: nodename nor servname provided",
                exit: 255,
                expect_top: "ssh -p 2222 deploy@example.com",
                expect_rule: "ssh_port_colon",
            },
            Case {
                name: "brew install of a cask",
                script: "brew install firefox",
                stderr: "Error: No available formula with the name \"firefox\". Found a cask named \"firefox\" instead.",
                exit: 1,
                expect_top: "brew install --cask firefox",
                expect_rule: "brew_cask",
            },
        ];
        let rules = shipped_rules();
        for c in cases {
            let ctx = CorrectContext::new(c.script, c.stderr, Some(c.exit));
            let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
            assert!(!out.is_empty(), "[{}] expected a correction", c.name);
            assert_eq!(out[0].command, c.expect_top, "[{}] top command", c.name);
            assert_eq!(out[0].rule_id, c.expect_rule, "[{}] top rule", c.name);
        }
    }

    #[test]
    fn shipped_native_subcommand_typo_fixes_git() {
        // `git comit` -> `git commit`, driven by the bundled git spec.
        let git_spec: Subcommand =
            serde_json::from_str(include_str!("../../../../specs/git.spec.json"))
                .expect("git spec parses");
        let specs = [git_spec];
        let rules = shipped_rules();
        let mut ctx = CorrectContext::new(
            "git comit -m x",
            "git: 'comit' is not a git command.",
            Some(1),
        );
        ctx.specs = &specs;
        let out = correct(&ctx, &rules, &empty_resolver()).expect("correct");
        assert!(
            out.iter().any(|c| c.command == "git commit -m x"),
            "expected `git commit -m x` in {out:?}"
        );
    }
}
