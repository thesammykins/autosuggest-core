//! Integration test: call the C ABI surface from Rust.
//!
//! Links the `autosuggest-ffi` crate as an `rlib` and exercises the real
//! `extern "C"` entry points end-to-end against the repository's `specs/` and
//! `rules/` directories, including the null and malformed-input recovery paths.
//!
//! The engine is a process-wide singleton built on first call, so the specs and
//! rules directories are set via environment variables before any request.

use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;

use autosuggest_daemon::MAX_REQUEST_BYTES;
use autosuggest_ffi::{autosuggest_request_json, autosuggest_string_free};

/// Repository root, derived from this crate's manifest dir (`crates/ffi`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Send a request string through the C ABI and return the response as a `String`.
fn call(request: &str) -> String {
    let c_request = CString::new(request).expect("no interior NUL");
    // SAFETY: `c_request` is a valid NUL-terminated string alive for the call.
    let ptr = unsafe { autosuggest_request_json(c_request.as_ptr()) };
    assert!(!ptr.is_null(), "entry point must not return null");
    // SAFETY: `ptr` was just returned by the entry point and is a valid C string.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    // SAFETY: `ptr` was produced by the library and is freed exactly once here.
    unsafe { autosuggest_string_free(ptr) };
    text
}

#[test]
fn ffi_round_trips_all_ops_and_handles_bad_input() {
    let root = repo_root();
    // Point the lazily-built engine at the real data before the first call.
    std::env::set_var("AUTOSUGGEST_SPECS_DIR", root.join("specs"));
    std::env::set_var("AUTOSUGGEST_RULES_DIR", root.join("rules"));

    // complete
    let resp = call(r#"{"v":1,"id":1,"op":"complete","line":"git co","cursor":6}"#);
    assert!(resp.contains("\"items\""), "complete items: {resp}");
    assert!(resp.contains("checkout"), "expected checkout: {resp}");

    // autosuggest
    let resp = call(
        r#"{"v":1,"id":2,"op":"autosuggest","prefix":"git pu",
            "history":{"entries":[{"command":"git push origin main"}]}}"#,
    );
    assert!(
        resp.contains("git push origin main"),
        "expected suggestion: {resp}"
    );

    // correct
    let resp = call(
        r#"{"v":1,"id":3,"op":"correct","script":"mkdir a/b/c",
            "stderr":"mkdir: cannot create directory 'a/b/c': No such file or directory",
            "exitCode":1}"#,
    );
    assert!(
        resp.contains("mkdir -p a/b/c"),
        "expected correction: {resp}"
    );

    // malformed JSON → structured error, no crash.
    let resp = call(r#"{"id":9,"op":"#);
    assert!(resp.contains("\"error\""), "malformed yields error: {resp}");
    assert!(resp.contains("bad_request"), "error code present: {resp}");
}

#[test]
fn ffi_null_input_is_safe() {
    // SAFETY: null is an explicitly supported input per the C contract.
    let ptr = unsafe { autosuggest_request_json(std::ptr::null::<c_char>()) };
    assert!(!ptr.is_null(), "null input still returns a string");
    // SAFETY: `ptr` was returned by the entry point; read then free once.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    assert!(text.contains("\"error\""), "null yields error: {text}");
    // SAFETY: freeing a library-produced pointer exactly once.
    unsafe { autosuggest_string_free(ptr) };
}

#[test]
fn ffi_oversized_input_is_rejected_before_parse() {
    let request = "x".repeat(MAX_REQUEST_BYTES + 1);
    let c_request = CString::new(request).expect("no interior NUL");
    // SAFETY: `c_request` is a valid NUL-terminated string alive for the call.
    let ptr = unsafe { autosuggest_request_json(c_request.as_ptr()) };
    assert!(!ptr.is_null(), "oversized input returns a string");
    // SAFETY: `ptr` was returned by the entry point; read then free once.
    let text = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    assert!(
        text.contains("bad_request"),
        "oversized yields error: {text}"
    );
    // SAFETY: freeing a library-produced pointer exactly once.
    unsafe { autosuggest_string_free(ptr) };
}

#[test]
fn ffi_free_null_is_noop() {
    // SAFETY: freeing null is documented as a no-op.
    unsafe { autosuggest_string_free(std::ptr::null_mut()) };
}
