//! Build script: generate the C header for the FFI surface.
//!
//! Runs `cbindgen` over this crate and writes `include/autosuggest.h`, the
//! committed, canonical header hosts include. Keeping generation in `build.rs`
//! means the header cannot drift from the `extern "C"` functions in `lib.rs`.
//!
//! If generation fails (e.g. an unusual environment), the build still succeeds:
//! the committed header remains the source of truth and a warning is emitted.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            println!("cargo:warning=CARGO_MANIFEST_DIR unset; skipping header generation");
            return;
        }
    };

    let config = match cbindgen::Config::from_file(crate_dir.join("cbindgen.toml")) {
        Ok(config) => config,
        Err(err) => {
            println!("cargo:warning=cbindgen config error: {err}; skipping header generation");
            return;
        }
    };

    let builder = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config);

    match builder.generate() {
        Ok(bindings) => {
            let out = crate_dir.join("include").join("autosuggest.h");
            if let Some(parent) = out.parent() {
                if let Err(err) = std::fs::create_dir_all(parent) {
                    println!("cargo:warning=could not create include dir: {err}");
                    return;
                }
            }
            // `write_to_file` only rewrites when the contents change, keeping
            // the committed header stable across no-op builds.
            bindings.write_to_file(&out);
        }
        Err(err) => {
            println!("cargo:warning=cbindgen generation failed: {err}; using committed header");
        }
    }
}
