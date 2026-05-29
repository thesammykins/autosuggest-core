//! [`Suggestion`]: the suggestion object from `SCHEMA.md §1.4`.
//!
//! The schema states: a bare string is shorthand for `{ "name": [s] }`. So this
//! type deserializes from either a JSON string or a full object, and serializes
//! back to a bare string when it is a pure shorthand (only `name` set to a single
//! value) — otherwise to the full object form. This keeps authored specs that
//! use string shorthand lossless on round-trip.

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

use super::string_list::StringList;

/// A completion suggestion (`SCHEMA.md §1.4`).
///
/// Construct from a bare string via [`From`] for the common shorthand case, or
/// build the full form field-by-field.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// Suggestion name(s); required. First is canonical.
    pub name: StringList,
    /// Display label; defaults to `name[0]` when absent.
    pub display_name: Option<String>,
    /// Text to insert; may contain a `{cursor}` marker. Defaults to `name[0]`.
    pub insert_value: Option<String>,
    /// Short human description.
    pub description: Option<String>,
    /// Priority `0..=100` (default 50 applied by the engine, not stored).
    pub priority: Option<u8>,
    /// Host may warn before accepting (e.g. `rm -rf`).
    pub is_dangerous: Option<bool>,
    /// Excluded unless explicitly typed.
    pub hidden: Option<bool>,
    /// Marked deprecated.
    pub deprecated: Option<bool>,
}

impl Suggestion {
    /// True when this suggestion carries nothing but a single canonical name,
    /// i.e. it is exactly the string-shorthand form and can serialize as a bare
    /// string.
    fn is_pure_shorthand(&self) -> bool {
        matches!(&self.name, StringList::One(_))
            && self.display_name.is_none()
            && self.insert_value.is_none()
            && self.description.is_none()
            && self.priority.is_none()
            && self.is_dangerous.is_none()
            && self.hidden.is_none()
            && self.deprecated.is_none()
    }

    /// Build a shorthand suggestion from a single name.
    fn from_name(name: String) -> Self {
        Suggestion {
            name: StringList::One(name),
            display_name: None,
            insert_value: None,
            description: None,
            priority: None,
            is_dangerous: None,
            hidden: None,
            deprecated: None,
        }
    }
}

impl From<&str> for Suggestion {
    fn from(s: &str) -> Self {
        Suggestion::from_name(s.to_string())
    }
}

impl From<String> for Suggestion {
    fn from(s: String) -> Self {
        Suggestion::from_name(s)
    }
}

impl Serialize for Suggestion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if self.is_pure_shorthand() {
            return serializer.serialize_str(self.name.canonical());
        }

        // Count present fields so the struct is sized correctly.
        let mut len = 1; // name is always present
        len += usize::from(self.display_name.is_some());
        len += usize::from(self.insert_value.is_some());
        len += usize::from(self.description.is_some());
        len += usize::from(self.priority.is_some());
        len += usize::from(self.is_dangerous.is_some());
        len += usize::from(self.hidden.is_some());
        len += usize::from(self.deprecated.is_some());

        let mut s = serializer.serialize_struct("Suggestion", len)?;
        s.serialize_field("name", &self.name)?;
        if let Some(v) = &self.display_name {
            s.serialize_field("displayName", v)?;
        }
        if let Some(v) = &self.insert_value {
            s.serialize_field("insertValue", v)?;
        }
        if let Some(v) = &self.description {
            s.serialize_field("description", v)?;
        }
        if let Some(v) = &self.priority {
            s.serialize_field("priority", v)?;
        }
        if let Some(v) = &self.is_dangerous {
            s.serialize_field("isDangerous", v)?;
        }
        if let Some(v) = &self.hidden {
            s.serialize_field("hidden", v)?;
        }
        if let Some(v) = &self.deprecated {
            s.serialize_field("deprecated", v)?;
        }
        s.end()
    }
}

impl<'de> Deserialize<'de> for Suggestion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SuggestionVisitor;

        impl<'de> Visitor<'de> for SuggestionVisitor {
            type Value = Suggestion;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a suggestion string or object")
            }

            fn visit_str<E>(self, value: &str) -> Result<Suggestion, E>
            where
                E: de::Error,
            {
                Ok(Suggestion::from_name(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> Result<Suggestion, E>
            where
                E: de::Error,
            {
                Ok(Suggestion::from_name(value))
            }

            fn visit_map<M>(self, mut map: M) -> Result<Suggestion, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut name: Option<StringList> = None;
                let mut display_name: Option<String> = None;
                let mut insert_value: Option<String> = None;
                let mut description: Option<String> = None;
                let mut priority: Option<u8> = None;
                let mut is_dangerous: Option<bool> = None;
                let mut hidden: Option<bool> = None;
                let mut deprecated: Option<bool> = None;

                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "name" => {
                            if name.is_some() {
                                return Err(de::Error::duplicate_field("name"));
                            }
                            name = Some(map.next_value()?);
                        }
                        "displayName" => display_name = Some(map.next_value()?),
                        "insertValue" => insert_value = Some(map.next_value()?),
                        "description" => description = Some(map.next_value()?),
                        "priority" => priority = Some(map.next_value()?),
                        "isDangerous" => is_dangerous = Some(map.next_value()?),
                        "hidden" => hidden = Some(map.next_value()?),
                        "deprecated" => deprecated = Some(map.next_value()?),
                        // Forward-compat: ignore unknown fields (SCHEMA.md §4.3).
                        _ => {
                            let _: de::IgnoredAny = map.next_value()?;
                        }
                    }
                }

                let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
                Ok(Suggestion {
                    name,
                    display_name,
                    insert_value,
                    description,
                    priority,
                    is_dangerous,
                    hidden,
                    deprecated,
                })
            }
        }

        deserializer.deserialize_any(SuggestionVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_shorthand_deserializes() {
        let s: Suggestion = serde_json::from_str(r#""checkout""#).expect("shorthand");
        assert_eq!(s.name, StringList::One("checkout".to_string()));
        assert!(s.is_pure_shorthand());
    }

    #[test]
    fn string_shorthand_reserializes_to_string() {
        let s: Suggestion = serde_json::from_str(r#""checkout""#).expect("shorthand");
        assert_eq!(serde_json::to_string(&s).expect("ser"), r#""checkout""#);
    }

    #[test]
    fn full_object_roundtrips() {
        // Mirrors the SCHEMA.md §1.4 example.
        let json = r#"{
            "name": ["checkout"],
            "displayName": "checkout",
            "insertValue": "checkout ",
            "description": "Switch branches",
            "priority": 75,
            "isDangerous": false,
            "hidden": false,
            "deprecated": false
        }"#;
        let s: Suggestion = serde_json::from_str(json).expect("object");
        assert_eq!(s.insert_value.as_deref(), Some("checkout "));
        assert_eq!(s.priority, Some(75));

        let out = serde_json::to_string(&s).expect("ser");
        assert!(out.contains("\"insertValue\""));
        assert!(out.contains("\"displayName\""));
        assert!(out.contains("\"isDangerous\""));

        let back: Suggestion = serde_json::from_str(&out).expect("re-de");
        assert_eq!(back, s);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = r#"{ "name": "x", "futureField": 42 }"#;
        let s: Suggestion = serde_json::from_str(json).expect("ignore unknown");
        assert_eq!(s.name, StringList::One("x".to_string()));
    }

    #[test]
    fn object_with_array_name_is_not_shorthand() {
        let s: Suggestion = serde_json::from_str(r#"{ "name": ["a", "b"] }"#).expect("array name");
        assert!(!s.is_pure_shorthand());
        let out = serde_json::to_string(&s).expect("ser");
        assert_eq!(out, r#"{"name":["a","b"]}"#);
    }
}
