//! Response envelopes (`SCHEMA.md §4.2`).
//!
//! Responses share the `v`/`id` envelope and are distinguished by which payload
//! key is present: `items` (complete/correct), `suggestion` (autosuggest, a
//! string or `null`), or `error`. We model this as an untagged enum; each
//! variant is a distinct struct keyed on its unique payload field.
//!
//! Per `SCHEMA.md §4.3`, `score` is a `0..=1` float and `items` are already
//! sorted descending.

use serde::{Deserialize, Serialize};

/// A protocol response (`SCHEMA.md §4.2`).
///
/// Untagged: deserialization picks the variant whose payload key is present.
/// Variants are ordered so the unique keys (`items`, `error`, `suggestion`) make
/// the match unambiguous.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    /// `complete` / `correct` result: a ranked list of items.
    Items(ItemsResponse),
    /// Error result.
    Error(ErrorResponse),
    /// `autosuggest` result: a single ghost-text string, or `null`.
    Suggestion(SuggestionResponse),
}

impl Response {
    /// Build an items response (`complete`/`correct`).
    pub fn items(id: i64, items: Vec<Item>) -> Self {
        Response::Items(ItemsResponse { v: 1, id, items })
    }

    /// Build an autosuggest response (`Some` text, or `None` for `null`).
    pub fn suggestion(id: i64, suggestion: Option<String>) -> Self {
        Response::Suggestion(SuggestionResponse {
            v: 1,
            id,
            suggestion,
        })
    }

    /// Build an error response.
    pub fn error(id: i64, code: impl Into<String>, message: impl Into<String>) -> Self {
        Response::Error(ErrorResponse {
            v: 1,
            id,
            error: ErrorBody {
                code: code.into(),
                message: message.into(),
            },
        })
    }
}

/// `items` response body for `complete` / `correct` (`SCHEMA.md §4.2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemsResponse {
    /// Protocol version (`1`).
    pub v: u32,
    /// Echoed request id.
    pub id: i64,
    /// Ranked items, already sorted descending by `score`.
    pub items: Vec<Item>,
}

/// `suggestion` response body for `autosuggest` (`SCHEMA.md §4.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuggestionResponse {
    /// Protocol version (`1`).
    pub v: u32,
    /// Echoed request id.
    pub id: i64,
    /// The single ghost-text suggestion, or `null` when there is none.
    pub suggestion: Option<String>,
}

/// `error` response body (`SCHEMA.md §4.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    /// Protocol version (`1`).
    pub v: u32,
    /// Echoed request id.
    pub id: i64,
    /// Error detail.
    pub error: ErrorBody,
}

/// Error detail (`error` object in `SCHEMA.md §4.2`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// Machine-readable error code, e.g. `"bad_request"`.
    pub code: String,
    /// Human-readable message.
    pub message: String,
}

/// A single completion/correction item (`SCHEMA.md §4.2`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Text to insert.
    pub insert: String,
    /// Display label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Short description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// Score in `0..=1`, descending across `items`.
    pub score: f64,
    /// Host may warn before accepting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dangerous: Option<bool>,
    /// Marked deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_response_example_roundtrips() {
        // Mirrors the SCHEMA.md §4.2 complete/correct example.
        let json = r#"{ "v": 1, "id": 7, "items": [
            { "insert": "checkout ", "display": "checkout",
              "desc": "Switch branches", "score": 0.97,
              "dangerous": false, "deprecated": false }
        ] }"#;
        let resp: Response = serde_json::from_str(json).expect("deserialize §4.2 items");
        match &resp {
            Response::Items(r) => {
                assert_eq!(r.id, 7);
                assert_eq!(r.items.len(), 1);
                assert_eq!(r.items[0].insert, "checkout ");
                assert!((r.items[0].score - 0.97).abs() < f64::EPSILON);
            }
            other => panic!("expected items, got {other:?}"),
        }
        let out = serde_json::to_string(&resp).expect("serialize");
        let back: Response = serde_json::from_str(&out).expect("re-de");
        assert_eq!(back, resp);
    }

    #[test]
    fn suggestion_response_text_and_null() {
        // Mirrors the SCHEMA.md §4.2 autosuggest example.
        let json = r#"{ "v": 1, "id": 8, "suggestion": "git push origin main" }"#;
        let resp: Response = serde_json::from_str(json).expect("deserialize suggestion");
        match &resp {
            Response::Suggestion(s) => {
                assert_eq!(s.suggestion.as_deref(), Some("git push origin main"))
            }
            other => panic!("expected suggestion, got {other:?}"),
        }

        let null_json = r#"{ "v": 1, "id": 8, "suggestion": null }"#;
        let resp: Response = serde_json::from_str(null_json).expect("deserialize null suggestion");
        match &resp {
            Response::Suggestion(s) => assert_eq!(s.suggestion, None),
            other => panic!("expected suggestion, got {other:?}"),
        }
        // Null must round-trip back to an explicit `"suggestion":null`.
        let out = serde_json::to_string(&resp).expect("serialize");
        assert!(out.contains("\"suggestion\":null"));
    }

    #[test]
    fn error_response_example_roundtrips() {
        // Mirrors the SCHEMA.md §4.2 error example.
        let json = r#"{ "v": 1, "id": 9, "error": { "code": "bad_request", "message": "boom" } }"#;
        let resp: Response = serde_json::from_str(json).expect("deserialize error");
        match &resp {
            Response::Error(e) => {
                assert_eq!(e.error.code, "bad_request");
                assert_eq!(e.error.message, "boom");
            }
            other => panic!("expected error, got {other:?}"),
        }
        let out = serde_json::to_string(&resp).expect("serialize");
        let back: Response = serde_json::from_str(&out).expect("re-de");
        assert_eq!(back, resp);
    }

    #[test]
    fn constructors_produce_expected_variants() {
        assert!(matches!(Response::items(1, vec![]), Response::Items(_)));
        assert!(matches!(
            Response::suggestion(1, None),
            Response::Suggestion(_)
        ));
        assert!(matches!(
            Response::error(1, "bad_request", "x"),
            Response::Error(_)
        ));
    }
}
