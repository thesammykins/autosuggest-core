//! On-disk loading of command specs and correction rules (`TECH.md §4`).
//!
//! The engine itself is pure; this module is the host-side I/O that feeds it.
//! Both loaders scan a directory for the relevant `*.json` files, parse each
//! with the canonical `core`/`protocol` models, and return owned vectors. A
//! missing directory is treated as "no data" rather than an error so a host can
//! run completion-only or correction-only.

use std::fmt;
use std::path::{Path, PathBuf};

use autosuggest_core::correct::rule::Rule;
use autosuggest_core::types::Subcommand;

/// Default directory scanned for command specs when none is configured.
pub const DEFAULT_SPECS_DIR: &str = "./specs";

/// Default directory scanned for correction rules when none is configured.
pub const DEFAULT_RULES_DIR: &str = "./rules";

/// Filename suffix identifying a command spec file.
const SPEC_SUFFIX: &str = ".spec.json";

/// Filename suffix identifying a correction rule file.
const RULE_SUFFIX: &str = ".rule.json";

/// A failure while loading specs or rules from disk.
#[derive(Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// A directory entry could not be read.
    Io {
        /// The path being read when the error occurred.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// A JSON file failed to parse into its expected model.
    Parse {
        /// The offending file.
        path: PathBuf,
        /// The serde parse error message.
        message: String,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io { path, source } => {
                write!(f, "failed to read `{}`: {source}", path.display())
            }
            LoadError::Parse { path, message } => {
                write!(f, "failed to parse `{}`: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            LoadError::Parse { .. } => None,
        }
    }
}

/// Load every `*.spec.json` under `dir` into [`Subcommand`] models.
///
/// Returns an empty vector if `dir` does not exist. Files are processed in
/// sorted path order for deterministic indexing.
pub fn load_specs(dir: &Path) -> Result<Vec<Subcommand>, LoadError> {
    load_dir(dir, SPEC_SUFFIX)
}

/// Load every `*.rule.json` under `dir` into [`Rule`] models.
///
/// Returns an empty vector if `dir` does not exist. Files are processed in
/// sorted path order for deterministic rule ordering before the engine ranks.
pub fn load_rules(dir: &Path) -> Result<Vec<Rule>, LoadError> {
    load_dir(dir, RULE_SUFFIX)
}

/// Scan `dir` for files ending in `suffix`, parsing each into `T`.
fn load_dir<T>(dir: &Path, suffix: &str) -> Result<Vec<T>, LoadError>
where
    T: serde::de::DeserializeOwned,
{
    let paths = match collect_paths(dir, suffix) {
        Ok(paths) => paths,
        // A missing directory is "no data", not a failure.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(LoadError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    };

    let mut out = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = std::fs::read(&path).map_err(|source| LoadError::Io {
            path: path.clone(),
            source,
        })?;
        let parsed = serde_json::from_slice::<T>(&bytes).map_err(|err| LoadError::Parse {
            path: path.clone(),
            message: err.to_string(),
        })?;
        out.push(parsed);
    }
    Ok(out)
}

/// Return the sorted set of files in `dir` whose name ends in `suffix`.
fn collect_paths(dir: &Path, suffix: &str) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(suffix) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}
