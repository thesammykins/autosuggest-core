//! `autosuggest-daemon` — newline-delimited JSON stdio adapter (`SCHEMA.md §4.1`).
//!
//! Reads one JSON request object per line from stdin, dispatches it through the
//! shared [`Engine`](autosuggest_daemon::Engine), and writes exactly one JSON
//! response object per line to stdout, flushing after each. Malformed lines
//! produce a structured error response and the loop continues; EOF exits `0`.
//!
//! ## Configuration
//!
//! * Specs directory: first CLI arg, else `$AUTOSUGGEST_SPECS_DIR`, else
//!   [`DEFAULT_SPECS_DIR`](autosuggest_daemon::DEFAULT_SPECS_DIR).
//! * Rules directory: second CLI arg, else `$AUTOSUGGEST_RULES_DIR`, else
//!   [`DEFAULT_RULES_DIR`](autosuggest_daemon::DEFAULT_RULES_DIR).

#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use autosuggest_daemon::{Engine, DEFAULT_RULES_DIR, DEFAULT_SPECS_DIR};

/// Environment variable overriding the specs directory.
const ENV_SPECS_DIR: &str = "AUTOSUGGEST_SPECS_DIR";
/// Environment variable overriding the rules directory.
const ENV_RULES_DIR: &str = "AUTOSUGGEST_RULES_DIR";

/// Environment variable overriding the history database path.
const ENV_HISTORY_DB: &str = "AUTOSUGGEST_HISTORY_DB";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let mut history_db: Option<PathBuf> = None;
    let mut pos_args = Vec::new();

    // Simple argument parser: consume `--history-db <path>` from the arg list,
    // treat everything else as positional.
    while let Some(arg) = args.next() {
        if arg == "--history-db" {
            history_db = args.next().map(PathBuf::from);
        } else {
            pos_args.push(arg);
        }
    }

    let mut pos = pos_args.into_iter();
    let specs_dir = resolve_dir(pos.next(), ENV_SPECS_DIR, DEFAULT_SPECS_DIR);
    let rules_dir = resolve_dir(pos.next(), ENV_RULES_DIR, DEFAULT_RULES_DIR);
    let history_db = history_db.or_else(|| std::env::var_os(ENV_HISTORY_DB).map(PathBuf::from));

    let engine = match Engine::load_with_history(&specs_dir, &rules_dir, history_db.as_deref()) {
        Ok(engine) => engine,
        Err(err) => {
            // A startup data error is fatal and reported on stderr; the wire
            // protocol on stdout stays clean.
            eprintln!("autosuggest-daemon: {err}");
            return ExitCode::FAILURE;
        }
    };

    match run(&engine) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("autosuggest-daemon: I/O error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve a directory from (in order) an explicit arg, an env var, a default.
fn resolve_dir(arg: Option<String>, env_key: &str, default: &str) -> PathBuf {
    if let Some(arg) = arg {
        return PathBuf::from(arg);
    }
    if let Some(env) = std::env::var_os(env_key) {
        return PathBuf::from(env);
    }
    PathBuf::from(default)
}

/// The read→dispatch→write loop. Returns once stdin reaches EOF.
fn run(engine: &Engine) -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        // Blank lines carry no request; skip them silently to be lenient with
        // editors/pipes that emit stray newlines.
        if line.trim().is_empty() {
            continue;
        }
        let response = engine.handle_line(&line);
        out.write_all(response.as_bytes())?;
        out.write_all(b"\n")?;
        out.flush()?;
    }
    Ok(())
}
