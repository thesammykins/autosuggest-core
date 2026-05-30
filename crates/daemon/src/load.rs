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
    /// A configured data directory is not acceptable for production loading.
    UntrustedDir {
        /// The directory that failed validation.
        path: PathBuf,
        /// Human-readable reason.
        reason: &'static str,
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
            LoadError::UntrustedDir { path, reason } => {
                write!(
                    f,
                    "refusing untrusted data directory `{}`: {reason}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            LoadError::Parse { .. } | LoadError::UntrustedDir { .. } => None,
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
    validate_data_dir(dir)?;

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

/// Validate configured data directories before reading specs/rules from them.
///
/// Relative defaults are convenient during local development but are unsafe for
/// privileged/GUI integrations where cwd or env can be attacker influenced.
fn validate_data_dir(dir: &Path) -> Result<(), LoadError> {
    if !dir.is_absolute() && !allow_relative_data_dirs() {
        return Err(LoadError::UntrustedDir {
            path: dir.to_path_buf(),
            reason: "path must be absolute unless AUTOSUGGEST_ALLOW_RELATIVE_DATA_DIRS=1",
        });
    }

    if let Ok(meta) = std::fs::symlink_metadata(dir) {
        if meta.file_type().is_symlink() {
            return Err(LoadError::UntrustedDir {
                path: dir.to_path_buf(),
                reason: "directory must not be a symlink",
            });
        }
    }

    Ok(())
}

fn allow_relative_data_dirs() -> bool {
    std::env::var_os("AUTOSUGGEST_ALLOW_RELATIVE_DATA_DIRS").is_some_and(|v| v == "1")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_data_dir_is_rejected_by_validator() {
        let err = validate_data_dir(Path::new("specs")).expect_err("relative path rejected");
        assert!(matches!(err, LoadError::UntrustedDir { .. }));
    }

    #[test]
    fn absolute_missing_data_dir_passes_trust_validation() {
        let path =
            std::env::temp_dir().join(format!("asc-missing-data-dir-{}", std::process::id()));
        assert!(validate_data_dir(&path).is_ok());
    }
}
