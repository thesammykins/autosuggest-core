//! Integration test: drive the `autosuggest-daemon` binary over stdio.
//!
//! Spawns the built daemon (located via `CARGO_BIN_EXE_autosuggest-daemon`),
//! pointed at the repository's real `specs/` and `rules/` directories, then
//! exercises all three ops plus malformed-input recovery on a single long-lived
//! process — verifying the newline-delimited request/response contract.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Repository root, derived from this crate's manifest dir (`crates/daemon`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// A spawned daemon with line-buffered stdio handles.
struct Daemon {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Daemon {
    /// Spawn the daemon against the repo's `specs/` and `rules/` directories.
    fn spawn() -> Self {
        let root = repo_root();
        Self::spawn_with(&root.join("specs"), &root.join("rules"))
    }

    /// Spawn the daemon against arbitrary `specs`/`rules` directories.
    fn spawn_with(specs: &std::path::Path, rules: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_autosuggest-daemon"))
            .arg(specs)
            .arg(rules)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn daemon");
        let stdin = child.stdin.take().expect("daemon stdin");
        let stdout = BufReader::new(child.stdout.take().expect("daemon stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    /// Send one request line and read exactly one response line.
    fn request(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}").expect("write request");
        self.stdin.flush().expect("flush request");
        let mut response = String::new();
        let n = self.stdout.read_line(&mut response).expect("read response");
        assert!(n > 0, "daemon closed stdout unexpectedly");
        response.trim_end().to_string()
    }

    /// Close stdin and assert the daemon exits successfully on EOF.
    fn shutdown(mut self) {
        drop(self.stdin);
        let status = self.child.wait().expect("wait for daemon");
        assert!(
            status.success(),
            "daemon should exit 0 on EOF, got {status:?}"
        );
    }
}

#[test]
fn daemon_handles_all_ops_and_malformed_input() {
    let mut daemon = Daemon::spawn();

    // complete: `git co` should suggest the `checkout` subcommand.
    let resp = daemon.request(r#"{"v":1,"id":1,"op":"complete","line":"git co","cursor":6}"#);
    assert!(resp.contains("\"id\":1"), "complete echoes id: {resp}");
    assert!(resp.contains("\"items\""), "complete returns items: {resp}");
    assert!(resp.contains("checkout"), "expected checkout in: {resp}");

    // autosuggest: history continuation for `git pu`.
    let resp = daemon.request(
        r#"{"v":1,"id":2,"op":"autosuggest","prefix":"git pu","history":{"entries":[{"command":"git push origin main"}]}}"#,
    );
    assert!(resp.contains("\"id\":2"), "autosuggest echoes id: {resp}");
    assert!(
        resp.contains("git push origin main"),
        "expected suggestion in: {resp}"
    );

    // correct: `mkdir a/b/c` with a missing-parent error → `mkdir -p a/b/c`.
    let resp = daemon.request(
        r#"{"v":1,"id":3,"op":"correct","script":"mkdir a/b/c","stderr":"mkdir: cannot create directory 'a/b/c': No such file or directory","exitCode":1}"#,
    );
    assert!(resp.contains("\"id\":3"), "correct echoes id: {resp}");
    assert!(
        resp.contains("mkdir -p a/b/c"),
        "expected correction in: {resp}"
    );

    // malformed line: structured error and the daemon keeps going. A truncated
    // object cannot yield an id, so the engine reports the sentinel id (-1).
    let resp = daemon.request(r#"{"id":4,"op":"complete""#);
    assert!(resp.contains("\"error\""), "malformed yields error: {resp}");
    assert!(resp.contains("bad_request"), "error code present: {resp}");

    // unknown op: structured error, process still alive.
    let resp = daemon.request(r#"{"v":1,"id":5,"op":"frobnicate"}"#);
    assert!(
        resp.contains("\"error\""),
        "unknown op yields error: {resp}"
    );

    // after errors, a valid request still works (loop survived).
    let resp = daemon.request(r#"{"v":1,"id":6,"op":"complete","line":"git st","cursor":6}"#);
    assert!(
        resp.contains("status"),
        "daemon recovered after errors: {resp}"
    );

    daemon.shutdown();
}

/// End-to-end proof that the daemon executes argument generators through the
/// sandboxed runner and that the dynamic output is filtered by the query.
///
/// Uses the allow-listed, deterministic `echo` program (no dependency on `git`
/// or any external state) on a purpose-built no-subcommand spec, so the single
/// top-level positional argument is the reachable cursor position.
#[test]
fn daemon_runs_argument_generator_with_echo() {
    let dir = std::env::temp_dir().join(format!("asc-gen-stdio-{}", std::process::id()));
    let specs = dir.join("specs");
    let rules = dir.join("rules");
    std::fs::create_dir_all(&specs).expect("create specs dir");
    std::fs::create_dir_all(&rules).expect("create rules dir");

    // A command with no subcommands so its positional arg is reachable, whose
    // generator prints a fixed, space-separated candidate set via `echo`.
    let spec = r#"{
        "name": "gentest",
        "description": "deterministic generator fixture",
        "args": [
            {
                "name": "item",
                "generator": {
                    "run": ["echo", "alpha beta betamax"],
                    "splitOn": " "
                }
            }
        ]
    }"#;
    std::fs::write(specs.join("gentest.spec.json"), spec).expect("write spec");

    let mut daemon = Daemon::spawn_with(&specs, &rules);

    // No query: all three generated candidates surface.
    let resp = daemon.request(r#"{"v":1,"id":1,"op":"complete","line":"gentest ","cursor":8}"#);
    assert!(resp.contains("alpha"), "generator output present: {resp}");
    assert!(resp.contains("beta"), "generator output present: {resp}");
    assert!(resp.contains("betamax"), "generator output present: {resp}");

    // Query `bet` filters to the two matching candidates, dropping `alpha`.
    let resp = daemon.request(r#"{"v":1,"id":2,"op":"complete","line":"gentest bet","cursor":11}"#);
    assert!(resp.contains("beta"), "filtered keeps beta: {resp}");
    assert!(resp.contains("betamax"), "filtered keeps betamax: {resp}");
    assert!(!resp.contains("alpha"), "filtered drops alpha: {resp}");

    daemon.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
