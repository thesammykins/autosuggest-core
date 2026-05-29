//! `autosuggest-core` — the pure completion & correction engine.
//!
//! This crate is the heart of the project: a side-effect-free transformation of
//! `context -> suggestions`. Per `TECH.md §2`, the host owns I/O and the engine
//! owns logic. Generator execution is injected through the [`GeneratorRunner`]
//! trait so the engine never performs file or network I/O directly.
//!
//! # Milestone status
//!
//! - M0: the [`types`] module — Rust models for every object in `SCHEMA.md §1`.
//! - M2: the [`history`] module — the stateless history autosuggester
//!   ([`history::autosuggest`]).
//!
//! The remaining engine modules ([`tokenize`], [`parse`], [`complete`],
//! [`rank`], [`correct`]) exist as empty stubs and gain logic in later
//! milestones (see `ROADMAP.md`).

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod complete;
pub mod correct;
pub mod history;
pub mod parse;
pub mod rank;
pub mod tokenize;
pub mod types;

mod generator_runner;

pub use generator_runner::{GeneratorError, GeneratorRunner};
