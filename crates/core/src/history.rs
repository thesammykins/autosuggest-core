//! History autosuggestion (stateless algorithm).
//!
//! Given a prefix and a window of recent commands, returns the single best
//! ghost-text continuation. Implemented in M2 (see `TECH.md §3.1`).

// M2+: stateless history_autosuggest(prefix, window).
