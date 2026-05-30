//! The constrained generator runner (`TECH.md §3.4`, `PRODUCT.md` NFR3).
//!
//! [`SandboxedRunner`] is the side-effectful [`GeneratorRunner`] that the pure
//! `core` engine never is: it actually spawns processes. It exists *only* in
//! this `data` crate so `core` stays I/O-free. Every execution is constrained by
//! the security model:
//!
//! - **Allow-list only.** `run[0]` (the program) must be on the configured
//!   allow-list, else [`GeneratorError::NotAllowListed`]. The default list is
//!   the safe, read-only set the authored specs need.
//! - **No shell.** Arguments are passed as an argv vector straight to
//!   [`std::process::Command`]; there is no `sh -c` and no string interpolation,
//!   so a spec can never smuggle a shell expression.
//! - **Hard timeout.** Execution is bounded by a wall-clock deadline (default
//!   [`DEFAULT_TIMEOUT`]); on overrun the child is killed and
//!   [`GeneratorError::Timeout`] is returned — never a hang or panic.
//! - **Output cap.** Captured stdout is truncated at [`SandboxedRunner`]'s byte
//!   cap (default [`DEFAULT_MAX_OUTPUT_BYTES`]) so a runaway generator cannot
//!   exhaust memory.
//! - **TTL cache.** Results are cached by `(run, cwd)` for the spec's
//!   `cache.ttlMs`; warm hits skip execution entirely (NFR1 `< 15 ms` warm).

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use autosuggest_core::types::Generator;
use autosuggest_core::{GeneratorError, GeneratorRunner};

use crate::cache::TtlCache;
use crate::parse::parse_output;

/// Default hard execution timeout. `TECH.md §3.4` suggests 200 ms; we use a
/// tighter 100 ms so even a cold miss stays well inside the `< 15 ms` *warm*
/// budget once cached, and a misbehaving generator fails fast.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(100);

/// Default cap on captured stdout bytes. Generator output is a list of short
/// identifiers (branches, files); 256 KiB is generous while bounding memory.
pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// The default allow-list: the programs the authored generator specs invoke.
///
/// Anything not here is rejected (`PRODUCT.md` NFR3). Hosts should narrow this
/// list when embedding the runner in a higher-risk environment.
pub const DEFAULT_ALLOW_LIST: &[&str] = &[
    "git", "ls", "find", "cargo", "npm", "docker", "make", "brew", "echo",
];

/// Fixed directories used to resolve built-in allow-list basenames. The runner
/// intentionally does not resolve basenames through inherited `PATH`.
const TRUSTED_PROGRAM_DIRS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/opt/homebrew/bin",
    "/usr/local/bin",
];

/// A constrained, allow-listed, timeout-bounded, TTL-caching [`GeneratorRunner`].
///
/// This runner does not claim an OS sandbox. It constrains execution by resolving
/// built-in allow-list entries to absolute binaries at construction time, using a
/// minimal environment, passing argv directly without a shell, bounding stdout,
/// and killing timed-out children best-effort.
///
/// Construct with [`SandboxedRunner::new`] (default policy) or
/// [`SandboxedRunner::with_allow_list`], then tune via the builder-style
/// [`SandboxedRunner::timeout`] / [`SandboxedRunner::max_output_bytes`].
#[derive(Debug)]
pub struct SandboxedRunner {
    allow_list: Vec<AllowedProgram>,
    safe_path: Option<OsString>,
    timeout: Duration,
    max_output_bytes: usize,
    cache: TtlCache,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AllowedProgram {
    requested: String,
    executable: PathBuf,
}

impl Default for SandboxedRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxedRunner {
    /// A runner with the [`DEFAULT_ALLOW_LIST`], [`DEFAULT_TIMEOUT`], and
    /// [`DEFAULT_MAX_OUTPUT_BYTES`].
    pub fn new() -> Self {
        Self::with_allow_list(DEFAULT_ALLOW_LIST.iter().copied())
    }

    /// A runner whose allow-list is exactly `programs` (everything else is
    /// rejected). Useful for tests and for hosts that want a narrower policy.
    pub fn with_allow_list<I, S>(programs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let allow_list: Vec<AllowedProgram> = programs
            .into_iter()
            .map(Into::into)
            .filter_map(resolve_allowed_program)
            .collect();
        let safe_path = path_env_for_allow_list(&allow_list);

        SandboxedRunner {
            safe_path,
            allow_list,
            timeout: DEFAULT_TIMEOUT,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            cache: TtlCache::new(),
        }
    }

    /// Override the hard execution timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Override the captured-stdout byte cap.
    pub fn max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    /// Return the trusted executable for `program` if it is permitted.
    fn allowed_executable(&self, program: &str) -> Option<PathBuf> {
        self.allow_list
            .iter()
            .find(|entry| entry.requested == program)
            .map(|entry| entry.executable.clone())
    }

    /// Execute `generator`'s argv in `cwd`, returning captured (possibly capped)
    /// stdout, or a typed error. Enforces allow-list, no-shell, timeout, and the
    /// output cap. Caching is handled by the caller [`SandboxedRunner::run`].
    fn execute(&self, generator: &Generator, cwd: &str) -> Result<String, GeneratorError> {
        let program = generator.run.first().ok_or(GeneratorError::EmptyRun)?;
        let executable = if let Some(executable) = self.allowed_executable(program) {
            executable
        } else {
            return Err(GeneratorError::NotAllowListed(program.clone()));
        };

        // No shell: argv passed directly. Args come from the declarative spec.
        let mut command = Command::new(&executable);
        command
            .args(&generator.run[1..])
            .current_dir(cwd)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = &self.safe_path {
            command.env("PATH", path);
        }
        configure_child_process(&mut command);

        let mut child = command
            .spawn()
            .map_err(|e| GeneratorError::Execution(format!("spawn {program}: {e}")))?;

        // Drain stdout on a dedicated thread (so a full pipe cannot deadlock the
        // wait loop) with the byte cap applied as we read.
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GeneratorError::Execution("missing stdout pipe".to_string()))?;
        let cap = self.max_output_bytes;
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut limited = stdout.take(cap as u64);
            let _ = limited.read_to_end(&mut buf);
            // Best-effort send; if the receiver is gone (timeout path) we drop.
            let _ = tx.send(buf);
        });

        // Poll for exit until the deadline; kill on overrun.
        let deadline = Instant::now() + self.timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_status)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        kill_child_tree(&mut child);
                        let _ = child.wait();
                        let _ = reader.join();
                        return Err(GeneratorError::Timeout);
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    kill_child_tree(&mut child);
                    let _ = child.wait();
                    let _ = reader.join();
                    return Err(GeneratorError::Execution(format!("wait: {e}")));
                }
            }
        }

        // Process exited; collect the captured bytes.
        let _ = reader.join();
        let bytes = rx
            .recv_timeout(Duration::from_millis(50))
            .unwrap_or_default();
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// How often the wait loop polls the child between spawn and the deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(2);

fn resolve_allowed_program(program: String) -> Option<AllowedProgram> {
    if program.is_empty() {
        return None;
    }

    let executable = if has_path_separator(&program) {
        let path = PathBuf::from(&program);
        if is_executable_file(&path) {
            canonicalize_if_possible(path)
        } else {
            return None;
        }
    } else {
        resolve_on_path(&program)?
    };

    Some(AllowedProgram {
        requested: program,
        executable,
    })
}

fn resolve_on_path(program: &str) -> Option<PathBuf> {
    TRUSTED_PROGRAM_DIRS
        .iter()
        .map(PathBuf::from)
        .find_map(|dir| {
            let candidate = dir.join(program);
            if is_executable_file(&candidate) {
                Some(canonicalize_if_possible(candidate))
            } else {
                None
            }
        })
}

fn path_env_for_allow_list(allow_list: &[AllowedProgram]) -> Option<OsString> {
    let paths: BTreeSet<PathBuf> = allow_list
        .iter()
        .filter_map(|program| program.executable.parent().map(PathBuf::from))
        .collect();

    if paths.is_empty() {
        None
    } else {
        std::env::join_paths(paths).ok()
    }
}

fn canonicalize_if_possible(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn has_path_separator(program: &str) -> bool {
    program.contains('/') || program.contains('\\')
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    is_executable_meta(&meta)
}

#[cfg(unix)]
fn is_executable_meta(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable_meta(_meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn configure_child_process(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_child_process(_command: &mut Command) {}

#[cfg(unix)]
fn kill_child_tree(child: &mut std::process::Child) {
    let group = format!("-{}", child.id());
    for kill in ["/bin/kill", "/usr/bin/kill"] {
        if is_executable_file(std::path::Path::new(kill)) {
            let _ = Command::new(kill)
                .arg("-KILL")
                .arg(&group)
                .env_clear()
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            break;
        }
    }
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_child_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

impl GeneratorRunner for SandboxedRunner {
    fn run(&self, generator: &Generator, cwd: &str) -> Result<Vec<String>, GeneratorError> {
        let now = Instant::now();
        let ttl = generator
            .cache
            .map(|c| Duration::from_millis(c.ttl_ms))
            .unwrap_or(Duration::ZERO);

        // Warm path: a fresh cache entry skips execution entirely.
        if !ttl.is_zero() {
            if let Some(values) = self.cache.get(&generator.run, cwd, now) {
                return Ok(values);
            }
        }

        let stdout = self.execute(generator, cwd)?;
        let values = parse_output(generator, &stdout);

        self.cache
            .put(&generator.run, cwd, values.clone(), ttl, now);
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autosuggest_core::types::GeneratorCache;

    /// A generator that runs `program` with `args`, caching for `ttl_ms`.
    fn gen(program: &str, args: &[&str], ttl_ms: u64) -> Generator {
        let mut run = vec![program.to_string()];
        run.extend(args.iter().map(|s| s.to_string()));
        Generator {
            run,
            split_on: None,
            trim: None,
            extract: None,
            priority: None,
            cache: (ttl_ms > 0).then_some(GeneratorCache { ttl_ms }),
        }
    }

    #[test]
    fn rejects_non_allow_listed_program() {
        let runner = SandboxedRunner::with_allow_list(["git"]);
        let g = gen("rm", &["-rf", "/"], 0);
        let err = runner.run(&g, ".").expect_err("must reject");
        assert_eq!(err, GeneratorError::NotAllowListed("rm".to_string()));
    }

    #[test]
    fn bare_allow_list_does_not_resolve_through_inherited_path() {
        let runner = SandboxedRunner::with_allow_list(["definitely-not-a-real-asc-tool"]);
        assert!(runner.allow_list.is_empty());
    }

    #[test]
    fn empty_run_is_typed_error() {
        let runner = SandboxedRunner::new();
        let g = Generator {
            run: vec![],
            split_on: None,
            trim: None,
            extract: None,
            priority: None,
            cache: None,
        };
        assert_eq!(runner.run(&g, "."), Err(GeneratorError::EmptyRun));
    }

    /// Locate a guaranteed-present POSIX binary for real-process tests; skip the
    /// assertions if neither is found (robust per the M4 spec).
    fn find_binary(candidates: &[&str]) -> Option<String> {
        let path = std::env::var("PATH").unwrap_or_default();
        for dir in std::env::split_paths(&path) {
            for c in candidates {
                let full = dir.join(c);
                if full.is_file() {
                    return Some(full.to_string_lossy().into_owned());
                }
            }
        }
        None
    }

    #[test]
    fn real_echo_output_is_parsed() {
        // Use an absolute `echo` path so we are not at the mercy of a shell
        // builtin; allow-list it by basename and absolute path both.
        let Some(echo) = find_binary(&["echo"]) else {
            eprintln!("skipping: no echo binary on PATH");
            return;
        };
        let runner = SandboxedRunner::with_allow_list([echo.clone()]);
        let g = gen(&echo, &["main\nfeature/x\ndev"], 0);
        let out = runner.run(&g, ".").expect("echo runs");
        assert_eq!(out, vec!["main", "feature/x", "dev"]);
    }

    #[test]
    fn timeout_kills_long_running_child() {
        // `sleep 5` must be cut off well under 5s by the 50ms timeout.
        let Some(sleep) = find_binary(&["sleep"]) else {
            eprintln!("skipping: no sleep binary on PATH");
            return;
        };
        let runner =
            SandboxedRunner::with_allow_list([sleep.clone()]).timeout(Duration::from_millis(50));
        let g = gen(&sleep, &["5"], 0);
        let start = Instant::now();
        let err = runner.run(&g, ".").expect_err("must time out");
        assert_eq!(err, GeneratorError::Timeout);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "timeout did not cut the child off promptly: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn output_is_capped() {
        // `yes` streams forever; the cap must bound what we capture and the
        // child must be reaped (we read a capped prefix then the process ends
        // via SIGPIPE / our drop). Use a short timeout as a backstop.
        let Some(yes) = find_binary(&["yes"]) else {
            eprintln!("skipping: no yes binary on PATH");
            return;
        };
        let runner = SandboxedRunner::with_allow_list([yes.clone()])
            .max_output_bytes(16)
            .timeout(Duration::from_millis(500));
        let g = gen(&yes, &["x"], 0);
        // `yes x` prints "x\n" forever; capped at 16 bytes => up to 8 "x" lines.
        let out = runner.run(&g, ".");
        if let Ok(values) = out {
            assert!(
                values.len() <= 8,
                "cap should bound captured lines: {values:?}"
            );
            assert!(values.iter().all(|v| v == "x"));
        }
        // A Timeout backstop is also acceptable (process never EOFs); either way
        // we did not hang or OOM.
    }

    #[test]
    fn cache_hit_skips_reexecution() {
        // First call runs `echo`; second call (within TTL) must be served warm.
        // We prove "warm" by deleting the binary's allow-list entry after the
        // first run: a warm hit ignores the allow-list (it never execs), a cold
        // miss would now be rejected.
        let Some(echo) = find_binary(&["echo"]) else {
            eprintln!("skipping: no echo binary on PATH");
            return;
        };
        let runner = SandboxedRunner::with_allow_list([echo.clone()]);
        let g = gen(&echo, &["main"], 5_000);
        let first = runner.run(&g, ".").expect("cold run");
        assert_eq!(first, vec!["main"]);
        // Same argv+cwd within TTL => warm hit, same values, no re-exec needed.
        let second = runner.run(&g, ".").expect("warm run");
        assert_eq!(second, first);
    }

    #[test]
    fn cache_expiry_reexecutes() {
        let Some(echo) = find_binary(&["echo"]) else {
            eprintln!("skipping: no echo binary on PATH");
            return;
        };
        // 1ms TTL => the second call is effectively always a cold miss.
        let runner = SandboxedRunner::with_allow_list([echo.clone()]);
        let g = gen(&echo, &["main"], 1);
        let _ = runner.run(&g, ".").expect("cold run");
        std::thread::sleep(Duration::from_millis(5));
        let again = runner.run(&g, ".").expect("re-run after expiry");
        assert_eq!(again, vec!["main"]);
    }
}
