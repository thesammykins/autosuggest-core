//! `autosuggest-protocol` — serde models for the stdio JSON-lines protocol.
//!
//! Implements the versioned envelope defined normatively in `SCHEMA.md §4`
//! (requests `complete`/`autosuggest`/`correct`; responses items/suggestion/
//! error) plus the [`HistoryWindow`] input from `SCHEMA.md §3`.
//!
//! Protocol rules (`SCHEMA.md §4.3`):
//! - Unknown fields MUST be ignored for forward-compatibility (we do not set
//!   `deny_unknown_fields`).
//! - `id` echoes the request; hosts may pipeline by `id`.
//! - One response object per request line.
//!
//! All field names match the schema exactly via `serde` renaming (`exitCode`,
//! `cursor`, camelCase as written). This crate carries only data models; no I/O
//! or transport logic lives here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod history;
pub mod request;
pub mod response;

pub use history::{HistoryEntry, HistoryWindow};
pub use request::Request;
pub use response::{ErrorBody, Item, Response};

/// The protocol version carried in every envelope (`"v": 1`).
pub const PROTOCOL_VERSION: u32 = 1;
