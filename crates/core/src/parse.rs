//! Spec-tree parser / parse-state machine.
//!
//! Consumes tokens left-to-right against a spec to produce a `ParseState`
//! (command path, consumed options, active arg). Implemented in M1
//! (see `TECH.md §3.2`).

// M1+: parse-state machine + parser directives.
