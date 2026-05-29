//! [`StringList`]: a `name` field that accepts a single string or an array.
//!
//! `SCHEMA.md §1` repeatedly defines `name` as "string or array (first =
//! canonical, rest = aliases)". This type deserializes from either JSON form and
//! re-serializes to the same form it was read from, so round-tripping a spec is
//! lossless.

use serde::{Deserialize, Serialize};

/// A name field accepting either a bare string or an array of strings.
///
/// The first element is the canonical name; any remaining elements are aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StringList {
    /// A single name, e.g. `"git"`.
    One(String),
    /// Multiple forms, e.g. `["-a", "--all"]`.
    Many(Vec<String>),
}

impl StringList {
    /// The canonical (first) name.
    ///
    /// For the [`StringList::Many`] empty-vector edge case this returns `""`;
    /// authored specs are expected to provide at least one name.
    pub fn canonical(&self) -> &str {
        match self {
            StringList::One(s) => s,
            StringList::Many(v) => v.first().map(String::as_str).unwrap_or(""),
        }
    }

    /// All names as a slice (canonical first, then aliases).
    pub fn all(&self) -> &[String] {
        match self {
            StringList::One(s) => std::slice::from_ref(s),
            StringList::Many(v) => v,
        }
    }
}

impl From<&str> for StringList {
    fn from(s: &str) -> Self {
        StringList::One(s.to_string())
    }
}

impl From<String> for StringList {
    fn from(s: String) -> Self {
        StringList::One(s)
    }
}

impl From<Vec<String>> for StringList {
    fn from(v: Vec<String>) -> Self {
        StringList::Many(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_string() {
        let s: StringList = serde_json::from_str(r#""git""#).expect("string form");
        assert_eq!(s, StringList::One("git".to_string()));
        assert_eq!(s.canonical(), "git");
        assert_eq!(s.all(), &["git".to_string()]);
    }

    #[test]
    fn accepts_array() {
        let s: StringList = serde_json::from_str(r#"["-a", "--all"]"#).expect("array form");
        assert_eq!(s.canonical(), "-a");
        assert_eq!(s.all(), &["-a".to_string(), "--all".to_string()]);
    }

    #[test]
    fn reserializes_to_original_form() {
        let one: StringList = serde_json::from_str(r#""git""#).expect("string");
        assert_eq!(serde_json::to_string(&one).expect("ser"), r#""git""#);

        let many: StringList = serde_json::from_str(r#"["a","b"]"#).expect("array");
        assert_eq!(serde_json::to_string(&many).expect("ser"), r#"["a","b"]"#);
    }
}
