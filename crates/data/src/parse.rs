//! Parse generator stdout into suggestion strings (`SCHEMA.md §1.5`).
//!
//! Given a [`Generator`] and the captured stdout, this module applies the
//! declarative post-processing rules, in order:
//!
//! 1. **`splitOn`** — split the output on this delimiter (default `"\n"`).
//! 2. **`trim`** — trim surrounding ASCII/Unicode whitespace from each piece
//!    (default `true`).
//! 3. **`extract`** — if set, a regex whose **capture group 1** is the
//!    suggestion; pieces that do not match are dropped.
//! 4. Empty pieces are discarded so blank trailing lines never become
//!    suggestions.
//!
//! The compiled `extract` regex is built once per parse. An invalid regex is
//! treated as "no extraction" so a malformed spec degrades to raw lines rather
//! than producing an error at completion time.

use autosuggest_core::types::Generator;
use regex::Regex;

/// The delimiter used when a generator omits `splitOn`.
const DEFAULT_SPLIT_ON: &str = "\n";

/// Apply `generator`'s `splitOn`/`trim`/`extract` rules to `stdout`, returning
/// the produced suggestion strings (empty pieces removed).
pub fn parse_output(generator: &Generator, stdout: &str) -> Vec<String> {
    let split_on = generator.split_on.as_deref().unwrap_or(DEFAULT_SPLIT_ON);
    let trim = generator.trim.unwrap_or(true);
    // An invalid `extract` regex falls back to no extraction.
    let extract = generator
        .extract
        .as_deref()
        .and_then(|pat| Regex::new(pat).ok());

    // An empty `splitOn` would yield one piece per char; guard against that by
    // treating the whole output as a single piece.
    let pieces: Vec<&str> = if split_on.is_empty() {
        vec![stdout]
    } else {
        stdout.split(split_on).collect()
    };

    pieces
        .into_iter()
        .filter_map(|piece| {
            let piece = if trim { piece.trim() } else { piece };
            let value = match &extract {
                Some(re) => re
                    .captures(piece)
                    .and_then(|caps| caps.get(1))
                    .map(|m| m.as_str().to_string()),
                None => Some(piece.to_string()),
            }?;
            if value.is_empty() {
                None
            } else {
                Some(value)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gen_with(split_on: Option<&str>, trim: Option<bool>, extract: Option<&str>) -> Generator {
        Generator {
            run: vec!["git".to_string()],
            split_on: split_on.map(|s| s.to_string()),
            trim,
            extract: extract.map(|s| s.to_string()),
            priority: None,
            cache: None,
        }
    }

    #[test]
    fn default_splits_on_newline_and_trims() {
        let g = gen_with(None, None, None);
        let out = parse_output(&g, "main\n  feature/x  \ndev\n");
        assert_eq!(out, vec!["main", "feature/x", "dev"]);
    }

    #[test]
    fn no_trim_keeps_whitespace() {
        let g = gen_with(None, Some(false), None);
        let out = parse_output(&g, "  a\nb ");
        assert_eq!(out, vec!["  a", "b "]);
    }

    #[test]
    fn custom_split_on() {
        let g = gen_with(Some("\0"), None, None);
        let out = parse_output(&g, "a\0b\0c");
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn extract_capture_group_one() {
        // `git branch` style: a leading `* ` on the current branch.
        let g = gen_with(None, None, Some(r"^\*?\s*(\S+)"));
        let out = parse_output(&g, "* main\n  feature/x\n  dev");
        assert_eq!(out, vec!["main", "feature/x", "dev"]);
    }

    #[test]
    fn extract_drops_non_matching_lines() {
        let g = gen_with(None, None, Some(r"^branch:(\S+)"));
        let out = parse_output(&g, "branch:main\nnoise\nbranch:dev");
        assert_eq!(out, vec!["main", "dev"]);
    }

    #[test]
    fn invalid_extract_falls_back_to_raw() {
        // Unbalanced paren => invalid regex => raw lines.
        let g = gen_with(None, None, Some("("));
        let out = parse_output(&g, "a\nb");
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn empty_pieces_removed() {
        let g = gen_with(None, None, None);
        let out = parse_output(&g, "\n\nmain\n\n");
        assert_eq!(out, vec!["main"]);
    }

    #[test]
    fn empty_split_on_treats_whole_output_as_one() {
        let g = gen_with(Some(""), None, None);
        let out = parse_output(&g, "main feature");
        assert_eq!(out, vec!["main feature"]);
    }
}
