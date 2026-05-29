//! `autosuggest-core` — the pure completion & correction engine.
//!
//! This crate is the heart of the project: a side-effect-free transformation of
//! `context -> suggestions`. Per `TECH.md §2`, the host owns I/O and the engine
//! owns logic. Generator execution is injected through the [`GeneratorRunner`]
//! trait so the engine never performs file or network I/O directly.
//!
//! # Milestone status
//!
//! The [`types`] module (M0) models every object in `SCHEMA.md §1`. The
//! [`correct`] module (M3) implements failed-command correction: a JSON rule
//! engine plus native predicates (`SCHEMA.md §2`/§2.1), with the host's `$PATH`
//! probe injected via [`correct::CommandResolver`]. The remaining engine modules
//! ([`tokenize`], [`parse`], [`complete`], [`rank`], [`history`]) are stubs that
//! gain logic in their milestones (see `ROADMAP.md`).

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
