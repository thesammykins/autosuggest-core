//! Data models for the completion spec format (`SCHEMA.md §1`).
//!
//! Every type here mirrors a JSON object defined normatively in `SCHEMA.md §1`.
//! Containers carry `#[serde(rename_all = "camelCase")]` so the on-disk JSON
//! matches the schema exactly (e.g. `insertValue`, `isOptional`,
//! `optionArgSeparators`). Optional fields use [`Option`] and `#[serde(default)]`
//! so absent keys round-trip faithfully and defaults are not re-emitted.
//!
//! Where the schema permits "string OR array" (the `name` field) the
//! [`StringList`] helper accepts both forms. Where it permits "string shorthand
//! OR object" ([`Suggestion`] and `suggestions` entries) a custom
//! [`serde::Deserialize`] implementation accepts both.

mod string_list;
mod suggestion;

pub use string_list::StringList;
pub use suggestion::Suggestion;

use serde::{Deserialize, Serialize};

/// A subcommand node, also used as the root of a spec (`SCHEMA.md §1.1`).
///
/// A spec file describes one top-level command; nested subcommands form the
/// completion tree the parser walks in M1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subcommand {
    /// Command name(s): first is canonical, the rest are aliases.
    pub name: StringList,

    /// Short human description (`<= 120` chars per schema).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Nested subcommands, if any.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subcommands: Vec<Subcommand>,

    /// Options/flags valid at this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Opt>,

    /// Positional arguments, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,

    /// Whether a subcommand is required to form a complete invocation.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_subcommand: bool,

    /// Parser behaviour overrides for this command tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parser_directives: Option<ParserDirectives>,
}

/// An option/flag definition (`SCHEMA.md §1.2`).
///
/// Named `Opt` rather than `Option` to avoid shadowing [`core::option::Option`].
/// Its JSON object is unaffected — the schema's concept is "option".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Opt {
    /// All forms of the option, e.g. `["-a", "--all"]`.
    pub name: StringList,

    /// Short human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Arguments the option takes; presence implies the option expects a value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,

    /// Whether the option must be present.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_required: bool,

    /// Repeatability: `false`/`true`, or an integer max count.
    #[serde(default, skip_serializing_if = "Repeatable::is_default")]
    pub is_repeatable: Repeatable,

    /// Whether the option applies to all descendant subcommands.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_persistent: bool,

    /// Whether the option must use the `--opt=value` form.
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_separator: bool,

    /// Option names this option is mutually exclusive with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclusive_on: Vec<String>,

    /// Option names that must also be present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
}

/// Repeatability of an option (`isRepeatable` in `SCHEMA.md §1.2`).
///
/// The schema allows a boolean *or* an integer max count, so this enum accepts
/// both forms while serializing back to the same JSON it was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Repeatable {
    /// `false` (not repeatable) or `true` (unbounded).
    Flag(bool),
    /// A maximum repeat count.
    Max(u32),
}

impl Default for Repeatable {
    fn default() -> Self {
        Repeatable::Flag(false)
    }
}

impl Repeatable {
    /// True when this is the schema default (`false`), so it can be omitted.
    fn is_default(&self) -> bool {
        matches!(self, Repeatable::Flag(false))
    }
}

/// A positional argument (`SCHEMA.md §1.3`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arg {
    /// Display-only label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Short human description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Whether the argument may be omitted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_optional: bool,

    /// Whether the argument consumes all remaining tokens.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_variadic: bool,

    /// Built-in suggestion source: `filepaths`, `folders`, or `history`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<Template>,

    /// Static suggestions (string shorthand or full objects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Suggestion>,

    /// Dynamic suggestion source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,

    /// Whether the argument is itself a command (e.g. `sudo`, `xargs`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_command: bool,
}

/// Built-in argument suggestion templates (`template` in `SCHEMA.md §1.3`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    /// Suggest files and directories.
    Filepaths,
    /// Suggest directories only.
    Folders,
    /// Suggest from history.
    History,
}

/// A declarative dynamic suggestion source (`SCHEMA.md §1.5`).
///
/// `run[0]` MUST be allow-listed by the runner; the engine never interprets a
/// shell string (`TECH.md §3.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Generator {
    /// argv to execute; `run[0]` is the (allow-listed) program.
    pub run: Vec<String>,

    /// Delimiter to split stdout on; defaults to `"\n"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_on: Option<String>,

    /// Whether to trim each produced entry; defaults to `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trim: Option<bool>,

    /// Regex whose capture group 1 extracts the suggestion from each entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<String>,

    /// Priority applied to produced suggestions (`0..=100`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,

    /// Cache configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<GeneratorCache>,
}

/// Cache configuration for a [`Generator`] (`cache` in `SCHEMA.md §1.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratorCache {
    /// Time-to-live in milliseconds; default `0` means no cache.
    pub ttl_ms: u64,
}

/// Parser behaviour overrides for a command tree (`SCHEMA.md §1.6`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserDirectives {
    /// `true` => short flags may chain (e.g. `-lah`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub flags_are_posix_noncompliant: bool,

    /// `true` => options are invalid after the first positional argument.
    #[serde(default, skip_serializing_if = "is_false")]
    pub options_must_precede_arguments: bool,

    /// Accepted separators between an option and its argument value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub option_arg_separators: Vec<String>,
}

/// serde helper: skip serializing `bool` fields that hold the schema default.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(b: &bool) -> bool {
    !*b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let json = serde_json::to_string(value).expect("serialize");
        serde_json::from_str(&json).expect("deserialize")
    }

    #[test]
    fn subcommand_example_roundtrips() {
        // Mirrors the SCHEMA.md §1.1 example shape.
        let json = r#"{
            "name": ["git"],
            "description": "Distributed VCS",
            "requiresSubcommand": false
        }"#;
        let sub: Subcommand = serde_json::from_str(json).expect("deserialize §1.1");
        assert_eq!(sub.name.canonical(), "git");
        assert_eq!(sub.description.as_deref(), Some("Distributed VCS"));
        assert!(!sub.requires_subcommand);
        assert_eq!(roundtrip(&sub), sub);
    }

    #[test]
    fn option_example_roundtrips_and_renames() {
        // Mirrors the SCHEMA.md §1.2 example.
        let json = r#"{
            "name": ["-a", "--all"],
            "description": "Include all",
            "isRequired": false,
            "isRepeatable": false,
            "isPersistent": false,
            "requiresSeparator": false,
            "exclusiveOn": ["--quiet"],
            "dependsOn": ["-l"]
        }"#;
        let opt: Opt = serde_json::from_str(json).expect("deserialize §1.2");
        assert_eq!(opt.name.all(), &["-a".to_string(), "--all".to_string()]);
        assert_eq!(opt.exclusive_on, vec!["--quiet".to_string()]);
        assert_eq!(opt.depends_on, vec!["-l".to_string()]);

        // Re-serialize: camelCase keys must be present.
        let out = serde_json::to_string(&opt).expect("serialize");
        assert!(out.contains("\"exclusiveOn\""));
        assert!(out.contains("\"dependsOn\""));
        assert_eq!(roundtrip(&opt), opt);
    }

    #[test]
    fn option_is_repeatable_accepts_int_and_bool() {
        let as_int: Opt =
            serde_json::from_str(r#"{ "name": "-v", "isRepeatable": 3 }"#).expect("int form");
        assert_eq!(as_int.is_repeatable, Repeatable::Max(3));

        let as_bool: Opt =
            serde_json::from_str(r#"{ "name": "-v", "isRepeatable": true }"#).expect("bool form");
        assert_eq!(as_bool.is_repeatable, Repeatable::Flag(true));

        // Default (absent) is Flag(false) and is omitted on re-serialize.
        let default: Opt = serde_json::from_str(r#"{ "name": "-v" }"#).expect("absent");
        assert_eq!(default.is_repeatable, Repeatable::Flag(false));
        let out = serde_json::to_string(&default).expect("serialize");
        assert!(!out.contains("isRepeatable"));
    }

    #[test]
    fn arg_example_roundtrips() {
        // Mirrors the SCHEMA.md §1.3 example, including string-shorthand suggestion.
        let json = r#"{
            "name": "path",
            "description": "a path",
            "isOptional": false,
            "isVariadic": false,
            "template": "filepaths",
            "suggestions": ["one", { "name": ["two"], "priority": 75 }],
            "isCommand": false
        }"#;
        let arg: Arg = serde_json::from_str(json).expect("deserialize §1.3");
        assert_eq!(arg.name.as_deref(), Some("path"));
        assert_eq!(arg.template, Some(Template::Filepaths));
        assert_eq!(arg.suggestions.len(), 2);
        assert_eq!(roundtrip(&arg), arg);
    }

    #[test]
    fn generator_example_roundtrips() {
        // Mirrors the SCHEMA.md §1.5 example.
        let json = r#"{
            "run": ["git", "branch", "--format=%(refname:short)"],
            "splitOn": "\n",
            "trim": true,
            "extract": "^(\\S+)",
            "priority": 60,
            "cache": { "ttlMs": 3000 }
        }"#;
        let g: Generator = serde_json::from_str(json).expect("deserialize §1.5");
        assert_eq!(g.run[0], "git");
        assert_eq!(g.priority, Some(60));
        assert_eq!(g.cache, Some(GeneratorCache { ttl_ms: 3000 }));
        let out = serde_json::to_string(&g).expect("serialize");
        assert!(out.contains("\"splitOn\""));
        assert!(out.contains("\"ttlMs\""));
        assert_eq!(roundtrip(&g), g);
    }

    #[test]
    fn parser_directives_example_roundtrips() {
        // Mirrors the SCHEMA.md §1.6 example.
        let json = r#"{
            "flagsArePosixNoncompliant": false,
            "optionsMustPrecedeArguments": false,
            "optionArgSeparators": ["=", " "]
        }"#;
        let pd: ParserDirectives = serde_json::from_str(json).expect("deserialize §1.6");
        assert_eq!(
            pd.option_arg_separators,
            vec!["=".to_string(), " ".to_string()]
        );
        let out = serde_json::to_string(&pd).expect("serialize");
        assert!(out.contains("\"optionArgSeparators\""));
        assert_eq!(roundtrip(&pd), pd);
    }
}
