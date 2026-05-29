//! Correction rule data models (`SCHEMA.md §2`).
//!
//! A correction rule is one of two kinds:
//!
//! * a **JSON rule** with a [`Match`] predicate set (AND of present conditions)
//!   and exactly one [`Rewrite`] strategy, or
//! * a **native rule** (`SCHEMA.md §2.1`): `{ "id", "native": true, "priority" }`
//!   whose logic lives in Rust ([`crate::correct::native`]), keyed by `id`.
//!
//! Both kinds carry an `id`, optional `description`, and a `priority` (`0..=100`,
//! default `50`) so ranking and ordering stay fully data-driven.
//!
//! The `match`/`rewrite` shapes mirror `SCHEMA.md §2` exactly; container structs
//! use `#[serde(rename_all = "camelCase")]` and the rewrite/predicate variants
//! use field renames so the on-disk JSON is byte-for-byte the schema's form.

use serde::{Deserialize, Serialize};

/// Default rule priority when `priority` is omitted (`SCHEMA.md §2`).
pub const DEFAULT_PRIORITY: u8 = 50;

fn default_priority() -> u8 {
    DEFAULT_PRIORITY
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

/// A single correction rule (`SCHEMA.md §2`).
///
/// JSON rules carry `match` + `rewrite`. Native rules set `native: true` and omit
/// both; their behaviour is implemented in code keyed by [`Rule::id`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    /// Unique rule id; required. For native rules this selects the Rust impl.
    pub id: String,

    /// Short human description shown alongside a suggestion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Ranking priority, `0..=100`, default `50`. Higher wins.
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// `true` => behaviour is implemented natively in Rust (`SCHEMA.md §2.1`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub native: bool,

    /// Predicate conditions (AND of present conditions). Absent for native rules.
    #[serde(rename = "match", default, skip_serializing_if = "Option::is_none")]
    pub match_: Option<Match>,

    /// The single rewrite strategy. Absent for native rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite: Option<Rewrite>,
}

/// Predicate set for a JSON rule (`SCHEMA.md §2`).
///
/// Every present field is a condition; the rule fires only when **all** present
/// conditions hold (logical AND). An empty `Match` (no conditions) always holds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    /// The script (verbatim) must start with this literal prefix.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_starts_with: Option<String>,

    /// The script must match this regular expression (unanchored unless the
    /// pattern anchors itself).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_regex: Option<String>,

    /// Any-of: stderr must contain at least one of these substrings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stderr_contains: Vec<String>,

    /// The stderr text must match this regular expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_regex: Option<String>,

    /// The failed command's exit code must be one of these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit_code_in: Vec<i32>,

    /// The named base command must resolve on `$PATH`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_exists: Option<String>,
}

/// A rewrite strategy (`SCHEMA.md §2`). Exactly one per rule.
///
/// Serialized as an externally-tagged object keyed by the strategy name, e.g.
/// `{ "insertFlag": { "after": "mkdir", "flag": "-p" } }`, matching the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Rewrite {
    /// Insert `flag` immediately after the token `after` (typically the base
    /// command or a subcommand). No-op if `after` is absent or the flag already
    /// follows it.
    InsertFlag {
        /// Token after which to insert the flag.
        after: String,
        /// The flag to insert, e.g. `"-p"`.
        flag: String,
    },

    /// Replace the token at `index` (0-based) with `with`.
    ReplaceToken {
        /// 0-based token index to replace.
        index: usize,
        /// Replacement token.
        with: String,
    },

    /// Prepend a literal prefix to the whole script, e.g. `"sudo "`.
    Prefix(String),

    /// Swap a subcommand token equal to `from` for `to` (first match only).
    SwapSubcommand {
        /// Subcommand token to replace.
        from: String,
        /// Replacement subcommand token.
        to: String,
    },

    /// Apply a regex substitution over the whole script (first match).
    RegexReplace {
        /// Search pattern.
        pattern: String,
        /// Replacement (supports `$1` capture references).
        with: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rule_example_roundtrips() {
        // Mirrors the SCHEMA.md §2 mkdir_p example.
        let json = r#"{
            "id": "mkdir_p",
            "description": "Add -p when parent dir missing",
            "priority": 90,
            "match": {
                "scriptStartsWith": "mkdir ",
                "stderrContains": ["No such file or directory"],
                "exitCodeIn": [1]
            },
            "rewrite": { "insertFlag": { "after": "mkdir", "flag": "-p" } }
        }"#;
        let rule: Rule = serde_json::from_str(json).expect("deserialize §2 rule");
        assert_eq!(rule.id, "mkdir_p");
        assert_eq!(rule.priority, 90);
        assert!(!rule.native);
        let m = rule.match_.as_ref().expect("match present");
        assert_eq!(m.script_starts_with.as_deref(), Some("mkdir "));
        assert_eq!(m.stderr_contains, vec!["No such file or directory"]);
        assert_eq!(m.exit_code_in, vec![1]);
        assert_eq!(
            rule.rewrite,
            Some(Rewrite::InsertFlag {
                after: "mkdir".to_string(),
                flag: "-p".to_string(),
            })
        );

        // Round-trip stability.
        let out = serde_json::to_string(&rule).expect("serialize");
        let back: Rule = serde_json::from_str(&out).expect("re-deserialize");
        assert_eq!(back, rule);
    }

    #[test]
    fn priority_defaults_to_50() {
        let rule: Rule = serde_json::from_str(r#"{ "id": "x" }"#).expect("minimal rule");
        assert_eq!(rule.priority, DEFAULT_PRIORITY);
    }

    #[test]
    fn native_rule_roundtrips() {
        // Mirrors the SCHEMA.md §2.1 native rule shape.
        let json = r#"{ "id": "no_command", "native": true, "priority": 30 }"#;
        let rule: Rule = serde_json::from_str(json).expect("deserialize native rule");
        assert!(rule.native);
        assert_eq!(rule.priority, 30);
        assert!(rule.match_.is_none());
        assert!(rule.rewrite.is_none());
        let out = serde_json::to_string(&rule).expect("serialize");
        assert!(out.contains("\"native\":true"));
    }

    #[test]
    fn each_rewrite_variant_serializes_to_schema_key() {
        let cases = [
            (
                Rewrite::InsertFlag {
                    after: "mkdir".into(),
                    flag: "-p".into(),
                },
                "insertFlag",
            ),
            (
                Rewrite::ReplaceToken {
                    index: 0,
                    with: "ls".into(),
                },
                "replaceToken",
            ),
            (Rewrite::Prefix("sudo ".into()), "prefix"),
            (
                Rewrite::SwapSubcommand {
                    from: "comit".into(),
                    to: "commit".into(),
                },
                "swapSubcommand",
            ),
            (
                Rewrite::RegexReplace {
                    pattern: "a".into(),
                    with: "b".into(),
                },
                "regexReplace",
            ),
        ];
        for (rw, key) in cases {
            let out = serde_json::to_string(&rw).expect("serialize rewrite");
            assert!(out.contains(key), "{out} should contain {key}");
            let back: Rewrite = serde_json::from_str(&out).expect("re-deserialize rewrite");
            assert_eq!(back, rw);
        }
    }
}
