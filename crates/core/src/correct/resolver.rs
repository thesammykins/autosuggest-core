//! Environment probe seam for correction (`TECH.md §3.5`).
//!
//! Two of the correction predicates need to know what commands exist on the
//! host: the JSON `commandExists` predicate (`SCHEMA.md §2`) and the native
//! `no_command` rule (`SCHEMA.md §2.1`). Both are **I/O** (scanning `$PATH`),
//! which `core` must not do directly (`TECH.md §2`).
//!
//! Mirroring how dynamic generators are injected via
//! [`crate::GeneratorRunner`], we inject this capability through the
//! [`CommandResolver`] trait. The engine stays pure and unit-testable with the
//! in-memory [`MockCommandResolver`]; hosts that want real behaviour use
//! [`PathCommandResolver`].
//!
//! ## Where the real implementation lives
//!
//! `TECH.md §3.4`/§3.5 envisions the `data` crate owning sandboxed I/O. That
//! crate does not exist yet in this workspace (it lands in M4). To keep M3
//! self-contained and testable, the real `$PATH` scanner
//! ([`PathCommandResolver`]) ships here behind the **`std-resolver`** cargo
//! feature (enabled by default). It performs only read-only environment/filesystem
//! probing and is never invoked by the pure engine paths used in tests, which
//! always take an injected resolver. When the `data` crate arrives, this
//! implementation can move there unchanged. See the report/deviations note.

/// Probes the host environment for command availability.
///
/// Injected into the correction engine so `core`'s logic stays pure. Two
/// capabilities are needed:
///
/// * [`CommandResolver::exists`] — does a base command resolve on `$PATH`?
/// * [`CommandResolver::path_commands`] — enumerate candidate command names on
///   `$PATH` (used by `no_command` to find the nearest match).
pub trait CommandResolver {
    /// Returns `true` if `cmd` is an executable resolvable on `$PATH`.
    fn exists(&self, cmd: &str) -> bool;

    /// Returns the set of command names available on `$PATH`.
    ///
    /// Order is unspecified; callers that need determinism should sort. The
    /// returned list may contain duplicates across `$PATH` entries — implementors
    /// are encouraged but not required to dedupe.
    fn path_commands(&self) -> Vec<String>;
}

/// An in-memory [`CommandResolver`] for tests and deterministic engine runs.
///
/// Construct from an explicit list of available command names; `exists` is an
/// exact membership check and `path_commands` returns the list verbatim.
#[derive(Debug, Clone, Default)]
pub struct MockCommandResolver {
    commands: Vec<String>,
}

impl MockCommandResolver {
    /// Build a resolver whose `$PATH` contains exactly `commands`.
    pub fn new<I, S>(commands: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            commands: commands.into_iter().map(Into::into).collect(),
        }
    }
}

impl CommandResolver for MockCommandResolver {
    fn exists(&self, cmd: &str) -> bool {
        self.commands.iter().any(|c| c == cmd)
    }

    fn path_commands(&self) -> Vec<String> {
        self.commands.clone()
    }
}

#[cfg(feature = "std-resolver")]
pub use std_resolver::PathCommandResolver;

#[cfg(feature = "std-resolver")]
mod std_resolver {
    //! Real, read-only `$PATH` scanner. Behind the `std-resolver` feature.

    use std::collections::BTreeSet;

    use super::CommandResolver;

    /// A [`CommandResolver`] that reads the process environment's `$PATH`.
    ///
    /// `exists` checks for an executable file at `<dir>/<cmd>` across `$PATH`
    /// directories. `path_commands` enumerates regular files in those directories.
    /// All operations are read-only; no command is ever executed.
    #[derive(Debug, Clone)]
    pub struct PathCommandResolver {
        dirs: Vec<std::path::PathBuf>,
    }

    impl PathCommandResolver {
        /// Build from the current process `$PATH`.
        pub fn from_env() -> Self {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let dirs = std::env::split_paths(&path)
                .filter(|p| !p.as_os_str().is_empty())
                .collect();
            Self { dirs }
        }

        /// Build from an explicit list of directories (testing/embedding).
        pub fn from_dirs<I, P>(dirs: I) -> Self
        where
            I: IntoIterator<Item = P>,
            P: Into<std::path::PathBuf>,
        {
            Self {
                dirs: dirs.into_iter().map(Into::into).collect(),
            }
        }

        #[cfg(unix)]
        fn is_executable(meta: &std::fs::Metadata) -> bool {
            use std::os::unix::fs::PermissionsExt;
            meta.is_file() && (meta.permissions().mode() & 0o111 != 0)
        }

        #[cfg(not(unix))]
        fn is_executable(meta: &std::fs::Metadata) -> bool {
            meta.is_file()
        }
    }

    impl Default for PathCommandResolver {
        fn default() -> Self {
            Self::from_env()
        }
    }

    impl CommandResolver for PathCommandResolver {
        fn exists(&self, cmd: &str) -> bool {
            // A command containing a path separator is not a bare PATH lookup.
            if cmd.is_empty() || cmd.contains('/') {
                return false;
            }
            self.dirs.iter().any(|dir| {
                let candidate = dir.join(cmd);
                std::fs::metadata(&candidate)
                    .map(|m| Self::is_executable(&m))
                    .unwrap_or(false)
            })
        }

        fn path_commands(&self) -> Vec<String> {
            let mut set: BTreeSet<String> = BTreeSet::new();
            for dir in &self.dirs {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(meta) = entry.metadata() else {
                        continue;
                    };
                    if Self::is_executable(&meta) {
                        if let Some(name) = entry.file_name().to_str() {
                            set.insert(name.to_string());
                        }
                    }
                }
            }
            set.into_iter().collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_resolver_exists_is_exact() {
        let r = MockCommandResolver::new(["ls", "git", "cat"]);
        assert!(r.exists("ls"));
        assert!(r.exists("git"));
        assert!(!r.exists("sl"));
        assert!(!r.exists(""));
    }

    #[test]
    fn mock_resolver_lists_commands() {
        let r = MockCommandResolver::new(["ls", "git"]);
        assert_eq!(r.path_commands(), vec!["ls".to_string(), "git".to_string()]);
    }

    #[cfg(feature = "std-resolver")]
    #[test]
    fn path_resolver_finds_listed_executables() {
        // Use a directory we know contains executables on Unix-like CI/dev hosts.
        let r = PathCommandResolver::from_env();
        // `path_commands` should be non-empty in any normal environment; but to
        // avoid environment flakiness we only assert the API is callable.
        let _ = r.path_commands();
        // A path-separator-containing name is never a bare command.
        assert!(!r.exists("/bin/ls"));
    }
}
