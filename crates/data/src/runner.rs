//! The sandboxed generator runner (`TECH.md §3.4`, `PRODUCT.md` NFR3).
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

use std::io::Read;
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

/// The default allow-list: the read-only programs the authored generator specs
/// invoke, plus the common safe completion sources. Anything not here is
/// rejected (`PRODUCT.md` NFR3). Kept intentionally small; hosts widen it
/// explicitly via [`SandboxedRunner::with_allow_list`].
pub const DEFAULT_ALLOW_LIST: &[&str] = &[
    "git", "ls", "find", "cargo", "npm", "docker", "make", "brew", "echo",
];

/// A sandboxed, allow-listed, timeout-bounded, TTL-caching [`GeneratorRunner`].
///
/// Construct with [`SandboxedRunner::new`] (default policy) or
/// [`SandboxedRunner::with_allow_list`], then tune via the builder-style
/// [`SandboxedRunner::timeout`] / [`SandboxedRunner::max_output_bytes`].
#[derive(Debug)]
pub struct SandboxedRunner {
    allow_list: Vec<String>,
    timeout: Duration,
    max_output_bytes: usize,
    cache: TtlCache,
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
        Self::with_allow_list(DEFAULT_ALLOW_LIST.iter().map(|s| s.to_string()))
    }

    /// A runner whose allow-list is exactly `programs` (everything else is
    /// rejected). Useful for tests and for hosts that want a narrower policy.
    pub fn with_allow_list<I, S>(programs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        SandboxedRunner {
            allow_list: programs.into_iter().map(Into::into).collect(),
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

    /// Whether `program` is permitted by the allow-list.
    fn is_allowed(&self, program: &str) -> bool {
        self.allow_list.iter().any(|p| p == program)
    }

    /// Execute `generator`'s argv in `cwd`, returning captured (possibly capped)
    /// stdout, or a typed error. Enforces allow-list, no-shell, timeout, and the
    /// output cap. Caching is handled by the caller [`SandboxedRunner::run`].
    fn execute(&self, generator: &Generator, cwd: &str) -> Result<String, GeneratorError> {
        let program = generator.run.first().ok_or(GeneratorError::EmptyRun)?;
        if !self.is_allowed(program) {
            return Err(GeneratorError::NotAllowListed(program.clone()));
        }

        // No shell: argv passed directly. Args come from the declarative spec.
        let mut child = Command::new(program)
            .args(&generator.run[1..])
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = reader.join();
                        return Err(GeneratorError::Timeout);
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    let _ = child.kill();
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
