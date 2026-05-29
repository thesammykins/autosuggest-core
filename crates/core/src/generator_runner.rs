//! Generator execution hook.
//!
//! `core` MUST NOT perform process, file, or network I/O directly (`TECH.md §2`).
//! Dynamic argument generators are therefore executed through this trait, which a
//! host crate (the `data` crate, per `TECH.md §3.4`) implements with a sandboxed,
//! allow-listed, timeout-bounded runner. Keeping execution behind a trait lets
//! the engine stay pure and unit-testable with mock runners.
//!
//! # Milestone status (M0)
//!
//! This is a stub: the trait shape is defined but no runner is provided. The
//! real sandboxed implementation lands in M4 (see `ROADMAP.md`).

use crate::types::Generator;

/// Error returned by a [`GeneratorRunner`] when a generator cannot be executed.
///
/// The concrete variants will be refined in M4; M0 only fixes the public shape.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeneratorError {
    /// `run[0]` was not in the runner's allow-list (`TECH.md §3.4`).
    NotAllowListed(String),
    /// The generator exceeded its execution timeout.
    Timeout,
    /// The generator process failed to spawn or exited abnormally.
    Execution(String),
}

impl core::fmt::Display for GeneratorError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GeneratorError::NotAllowListed(cmd) => {
                write!(f, "command not allow-listed: {cmd}")
            }
            GeneratorError::Timeout => write!(f, "generator timed out"),
            GeneratorError::Execution(msg) => write!(f, "generator execution failed: {msg}"),
        }
    }
}

impl std::error::Error for GeneratorError {}

/// Executes a declarative [`Generator`] and returns its produced suggestion
/// strings (one per line after `splitOn`/`trim`/`extract` processing).
///
/// Implementations MUST enforce the allow-list, timeout, and caching contract
/// described in `TECH.md §3.4`. The engine treats this purely as a data source
/// and never passes a shell string — only the generator's `run` argv vector.
///
/// # Milestone status (M0)
///
/// No production implementation exists yet; this trait is the injection seam the
/// M4 `data` crate will satisfy.
pub trait GeneratorRunner {
    /// Run `generator` with the given working directory and return the raw
    /// candidate strings it produced, or a [`GeneratorError`].
    fn run(&self, generator: &Generator, cwd: &str) -> Result<Vec<String>, GeneratorError>;
}
