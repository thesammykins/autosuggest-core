//! `autosuggest-ffi` — a minimal, panic-safe C ABI over the autosuggest engine.
//!
//! The host passes a JSON request string and receives a JSON response string,
//! exactly as on the stdio wire (`SCHEMA.md §4`). All dispatch is delegated to
//! the shared [`Engine`](autosuggest_daemon::Engine), so the C surface and the
//! stdio daemon behave identically.
//!
//! # Safety contract
//!
//! * [`autosuggest_request_json`] takes a NUL-terminated, UTF-8 C string and
//!   returns a heap-allocated, NUL-terminated C string owned by this library.
//!   The caller MUST return it via [`autosuggest_string_free`] and MUST NOT free
//!   it with `libc::free` or hold it past that call.
//! * A null or non-UTF-8 input never crashes: it yields a structured JSON error
//!   response string.
//! * No Rust panic ever crosses the FFI boundary; every entry point wraps its
//!   body in [`std::panic::catch_unwind`].

#![deny(unsafe_op_in_unsafe_fn)]
#![warn(missing_docs)]

use std::ffi::{c_char, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::OnceLock;

use autosuggest_daemon::{Engine, DEFAULT_RULES_DIR, DEFAULT_SPECS_DIR};

/// Environment variable overriding the specs directory (mirrors the daemon).
const ENV_SPECS_DIR: &str = "AUTOSUGGEST_SPECS_DIR";
/// Environment variable overriding the rules directory (mirrors the daemon).
const ENV_RULES_DIR: &str = "AUTOSUGGEST_RULES_DIR";

/// A pre-serialized error response used when the engine cannot be built or the
/// happy path cannot run. `id` is `-1` because no request id is available.
const FALLBACK_ERROR: &str =
    "{\"v\":1,\"id\":-1,\"error\":{\"code\":\"internal\",\"message\":\"engine unavailable\"}}";

/// Process-wide engine, built once on first use from the configured directories.
static ENGINE: OnceLock<Option<Engine>> = OnceLock::new();

/// Lazily construct (or reuse) the shared engine.
///
/// Returns `None` if loading specs/rules failed; callers then emit
/// [`FALLBACK_ERROR`]. Construction happens at most once for the process.
fn engine() -> Option<&'static Engine> {
    ENGINE
        .get_or_init(|| {
            let specs_dir = dir_from_env(ENV_SPECS_DIR, DEFAULT_SPECS_DIR);
            let rules_dir = dir_from_env(ENV_RULES_DIR, DEFAULT_RULES_DIR);
            Engine::load(&specs_dir, &rules_dir).ok()
        })
        .as_ref()
}

/// Resolve a directory from an env var, falling back to `default`.
fn dir_from_env(env_key: &str, default: &str) -> std::path::PathBuf {
    std::env::var_os(env_key)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(default))
}

/// Pure core of the entry point: borrow the request line, produce a response.
///
/// Kept separate from the `extern "C"` shim so it is ordinary safe Rust and can
/// be unit-tested directly.
fn respond(request: &str) -> String {
    match engine() {
        Some(engine) => engine.handle_line(request),
        None => FALLBACK_ERROR.to_string(),
    }
}

/// Handle one JSON request and return a newly allocated JSON response string.
///
/// The returned pointer is owned by this library; free it with
/// [`autosuggest_string_free`]. Returns a valid error-response string for null
/// or non-UTF-8 input, and never returns null on the normal path.
///
/// # Safety
///
/// `request` must be either null or a valid pointer to a NUL-terminated C
/// string that stays valid for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn autosuggest_request_json(request: *const c_char) -> *mut c_char {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Borrow the C string safely; null or invalid UTF-8 becomes an error
        // response rather than a crash.
        let response = if request.is_null() {
            error_response("null request pointer")
        } else {
            // SAFETY: the caller guarantees `request` is a valid NUL-terminated
            // C string for the duration of this call (see `# Safety`).
            let cstr = unsafe { CStr::from_ptr(request) };
            match cstr.to_str() {
                Ok(line) => respond(line),
                Err(_) => error_response("request was not valid UTF-8"),
            }
        };
        into_c_string(response)
    }));

    match result {
        Ok(ptr) => ptr,
        // A panic was caught: hand back a best-effort error string, or null only
        // if even that allocation failed.
        Err(_) => into_c_string(FALLBACK_ERROR.to_string()),
    }
}

/// Free a string previously returned by [`autosuggest_request_json`].
///
/// Passing null is a no-op. Passing any other pointer not produced by this
/// library, or freeing the same pointer twice, is undefined behavior.
///
/// # Safety
///
/// `s` must be either null or a pointer previously returned by
/// [`autosuggest_request_json`] and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn autosuggest_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    // Reclaim ownership and drop. Wrapped in `catch_unwind` so a corrupted
    // pointer's drop can never unwind into C (it is still UB, but we contain it).
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: per the contract `s` was produced by `CString::into_raw` in
        // `into_c_string` and has not been freed; reconstituting the `CString`
        // takes back ownership so it is dropped exactly once.
        drop(unsafe { CString::from_raw(s) });
    }));
}

/// Build a structured JSON error response string for a bad FFI input.
fn error_response(message: &str) -> String {
    format!(
        "{{\"v\":1,\"id\":-1,\"error\":{{\"code\":\"bad_request\",\"message\":\"{message}\"}}}}"
    )
}

/// Convert an owned Rust string into a heap C string pointer.
///
/// Interior NUL bytes cannot occur in our JSON output, but if conversion ever
/// fails we fall back to the static error so we still return a valid C string.
fn into_c_string(s: String) -> *mut c_char {
    let cstring = CString::new(s).unwrap_or_else(|_| {
        // `FALLBACK_ERROR` is a compile-time constant with no NUL bytes.
        CString::new(FALLBACK_ERROR).unwrap_or_default()
    });
    cstring.into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respond_handles_malformed_json_gracefully() {
        let out = respond("not json at all");
        assert!(out.contains("\"error\""));
        assert!(out.contains("bad_request"));
    }

    #[test]
    fn error_response_is_valid_json_shape() {
        let out = error_response("boom");
        assert!(out.starts_with("{\"v\":1,\"id\":-1,\"error\""));
        assert!(out.contains("boom"));
    }

    #[test]
    fn into_and_free_roundtrip_is_safe() {
        let ptr = into_c_string("hello".to_string());
        assert!(!ptr.is_null());
        // SAFETY: `ptr` came from `into_c_string` and is freed exactly once.
        unsafe { autosuggest_string_free(ptr) };
    }

    #[test]
    fn null_request_returns_error_string() {
        // SAFETY: null is an explicitly supported input.
        let ptr = unsafe { autosuggest_request_json(std::ptr::null()) };
        assert!(!ptr.is_null());
        // SAFETY: `ptr` was returned by the entry point; free once.
        let text = unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned();
        assert!(text.contains("null request pointer"));
        unsafe { autosuggest_string_free(ptr) };
    }
}
