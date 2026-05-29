//! Shared adapter engine for the autosuggest stdio/FFI surfaces (`TECH.md §4`).
//!
//! This library is the single source of truth for turning a protocol
//! [`Request`](autosuggest_protocol::Request) into a
//! [`Response`](autosuggest_protocol::Response). Both the stdio daemon binary
//! (`src/main.rs`) and the C ABI shim (`crates/ffi`) call into it, so dispatch
//! logic is written exactly once.
//!
//! The engine is a thin host adapter around the pure `autosuggest-core` engine:
//!
//! * It owns the I/O that `core` must not do — loading command specs and
//!   correction rules from disk, and probing `$PATH` via
//!   [`PathCommandResolver`](autosuggest_core::correct::PathCommandResolver).
//! * It maps wire types (`protocol`) to/from engine types (`core`) without
//!   redefining any wire model.
//!
//! Every fallible path returns a typed error or a structured protocol error
//! response; there are no `unwrap`/`expect`/`panic` calls in this crate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod load;

use std::collections::BTreeMap;
use std::path::Path;

use autosuggest_core::correct::rule::Rule;
use autosuggest_core::correct::{self, CorrectContext, PathCommandResolver, Resolver};
use autosuggest_core::types::Subcommand;
use autosuggest_core::{complete_line, history, CompletionItem};
use autosuggest_protocol::request::{AutosuggestRequest, CompleteRequest, CorrectRequest};
use autosuggest_protocol::{Item, Request, Response};

pub use load::{LoadError, DEFAULT_RULES_DIR, DEFAULT_SPECS_DIR};

/// Error code emitted for malformed request lines (`SCHEMA.md §4.2`).
const CODE_BAD_REQUEST: &str = "bad_request";
/// Error code emitted when an operation fails internally.
const CODE_INTERNAL: &str = "internal";

/// Request id used in error responses when the id could not be parsed.
const UNKNOWN_ID: i64 = -1;

/// The shared adapter engine: loaded specs + rules + a `$PATH` resolver.
///
/// Construct once at startup via [`Engine::load`] (or [`Engine::new`] for an
/// in-memory engine), then call [`Engine::handle_line`] per request line, or
/// [`Engine::handle`] when you already hold a parsed [`Request`].
pub struct Engine {
    /// Command specs indexed by every name/alias they answer to.
    specs_by_name: BTreeMap<String, usize>,
    /// Owned specs; `specs_by_name` holds indices into this vector.
    specs: Vec<Subcommand>,
    /// Loaded correction rules (`rules/*.rule.json`).
    rules: Vec<Rule>,
    /// Host capability for `$PATH` probing, injected into correction.
    resolver: Box<dyn Resolver + Send + Sync>,
}

impl Engine {
    /// Build an engine from in-memory specs, rules, and a resolver.
    ///
    /// Useful for tests and embedders that supply their own data. The on-disk
    /// loader [`Engine::load`] is the usual entry point.
    pub fn new(
        specs: Vec<Subcommand>,
        rules: Vec<Rule>,
        resolver: Box<dyn Resolver + Send + Sync>,
    ) -> Self {
        let specs_by_name = index_specs(&specs);
        Self {
            specs_by_name,
            specs,
            rules,
            resolver,
        }
    }

    /// Load specs from `specs_dir` and rules from `rules_dir`, using the real
    /// `$PATH` resolver.
    ///
    /// Both directories are scanned for their respective `*.spec.json` /
    /// `*.rule.json` files. A missing directory yields an empty set rather than
    /// an error, so a host can run completion-only or correction-only.
    pub fn load(specs_dir: &Path, rules_dir: &Path) -> Result<Self, LoadError> {
        let specs = load::load_specs(specs_dir)?;
        let rules = load::load_rules(rules_dir)?;
        Ok(Self::new(
            specs,
            rules,
            Box::new(PathCommandResolver::from_env()),
        ))
    }

    /// Handle one newline-free request line, returning the JSON response line.
    ///
    /// This never fails and never panics: malformed JSON, unknown ops, or
    /// internal engine errors are all turned into a structured protocol error
    /// [`Response`] and serialized. The returned string has no trailing newline.
    pub fn handle_line(&self, line: &str) -> String {
        let response = match serde_json::from_str::<Request>(line) {
            Ok(request) => self.handle(&request),
            Err(err) => {
                let id = best_effort_id(line);
                Response::error(id, CODE_BAD_REQUEST, format!("invalid request: {err}"))
            }
        };
        // `Response` is a plain data model and always serializes; fall back to a
        // hand-written error string in the impossible failure case so we never
        // panic on the hot path.
        serde_json::to_string(&response).unwrap_or_else(|_| {
            format!(
                "{{\"v\":1,\"id\":{UNKNOWN_ID},\"error\":{{\"code\":\"{CODE_INTERNAL}\",\
                 \"message\":\"failed to serialize response\"}}}}"
            )
        })
    }

    /// Dispatch a parsed [`Request`] to the matching engine operation.
    pub fn handle(&self, request: &Request) -> Response {
        match request {
            Request::Complete(req) => self.handle_complete(req),
            Request::Autosuggest(req) => self.handle_autosuggest(req),
            Request::Correct(req) => self.handle_correct(req),
        }
    }

    /// `complete`: as-you-type completion against the spec for the base command.
    fn handle_complete(&self, req: &CompleteRequest) -> Response {
        let cursor = req.cursor_or_end();
        let cwd = req.cwd.as_deref().unwrap_or(".");
        let base = req.line.split_whitespace().next().unwrap_or("");

        let Some(spec) = self.spec_for(base) else {
            // No spec for this command: a well-formed request with nothing to
            // suggest yields an empty item list, not an error.
            return Response::items(req.id, Vec::new());
        };

        let items = complete_line(spec, &req.line, cursor, Path::new(cwd));
        Response::items(req.id, items.iter().map(item_to_wire).collect())
    }

    /// `autosuggest`: stateless history continuation.
    fn handle_autosuggest(&self, req: &AutosuggestRequest) -> Response {
        let cwd = req.cwd.as_deref();
        let suggestion = match &req.history {
            Some(window) => history::autosuggest(&req.prefix, window, cwd),
            // No history window provided: nothing to continue from.
            None => None,
        };
        Response::suggestion(req.id, suggestion)
    }

    /// `correct`: ranked corrections for a failed command.
    fn handle_correct(&self, req: &CorrectRequest) -> Response {
        let stderr = req.stderr.as_deref().unwrap_or("");
        let mut ctx = CorrectContext::new(&req.script, stderr, req.exit_code);
        ctx.cwd = req.cwd.as_deref();
        ctx.specs = &self.specs;

        match correct::correct(&ctx, &self.rules, self.resolver.as_ref()) {
            Ok(corrections) => {
                let items = corrections
                    .iter()
                    .map(|c| Item {
                        insert: c.command.clone(),
                        display: None,
                        desc: c.description.clone(),
                        // Corrections are pre-ranked by the engine; expose a
                        // monotonically descending score derived from priority
                        // so the wire contract (`score` desc) holds.
                        score: f64::from(c.priority) / 100.0,
                        dangerous: None,
                        deprecated: None,
                    })
                    .collect();
                Response::items(req.id, items)
            }
            Err(err) => Response::error(req.id, CODE_INTERNAL, err.to_string()),
        }
    }

    /// Look up the spec answering to `name` (canonical name or alias).
    fn spec_for(&self, name: &str) -> Option<&Subcommand> {
        self.specs_by_name
            .get(name)
            .and_then(|&i| self.specs.get(i))
    }
}

/// Build the name→index map for `specs`, registering every name/alias.
///
/// On a name collision the first spec wins; later duplicates are skipped so a
/// stray duplicate file cannot shadow an earlier, intentional spec.
fn index_specs(specs: &[Subcommand]) -> BTreeMap<String, usize> {
    let mut map = BTreeMap::new();
    for (i, spec) in specs.iter().enumerate() {
        for name in spec.name.all() {
            map.entry(name.clone()).or_insert(i);
        }
    }
    map
}

/// Map a core [`CompletionItem`] onto the wire [`Item`].
fn item_to_wire(item: &CompletionItem) -> Item {
    Item {
        insert: item.insert.clone(),
        display: Some(item.display.clone()),
        desc: item.desc.clone(),
        score: item.score,
        // Only surface boolean flags when set, keeping the wire output compact.
        dangerous: item.dangerous.then_some(true),
        deprecated: item.deprecated.then_some(true),
    }
}

/// Best-effort extraction of `id` from a line that failed full parsing, so an
/// error response can still echo the caller's id where possible.
fn best_effort_id(line: &str) -> i64 {
    serde_json::from_str::<IdOnly>(line)
        .ok()
        .and_then(|v| v.id)
        .unwrap_or(UNKNOWN_ID)
}

/// Minimal shape used to salvage `id` from an otherwise-malformed request.
#[derive(serde::Deserialize)]
struct IdOnly {
    id: Option<i64>,
}
