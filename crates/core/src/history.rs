//! History autosuggestion (stateless algorithm).
//!
//! Given a `prefix` and a window of recent commands, [`autosuggest`] returns the
//! single best ghost-text continuation: the *full* command string the host
//! should complete the line to (fish / zsh-autosuggestions / atuin style), or
//! [`None`] when nothing matches.
//!
//! This module is **pure** (`TECH.md §2`): it performs no I/O and only inspects
//! the [`HistoryWindow`] handed in by the host. The window models are reused
//! directly from the `protocol` crate so the engine and the wire format defined
//! in `SCHEMA.md §3` cannot drift.
//!
//! # Algorithm (`TECH.md §3.1`)
//!
//! 1. **Order most-recent-first.** The host is asked to send entries
//!    most-recent-first (`SCHEMA.md §3`). As a safeguard, if *every* entry
//!    carries a `ts`, we sort by `ts` descending (stably, so ties keep their
//!    given relative order). If any entry lacks `ts`, we trust the given order
//!    and assume it is already most-recent-first.
//! 2. **Dedupe by command string**, keeping the most-recent occurrence. While
//!    deduping we also record, per distinct command, its occurrence `count`
//!    (frequency) so it can break ranking ties.
//! 3. **Prefix match** (case-sensitive): return the best entry whose `command`
//!    starts with `prefix`. An empty `prefix` matches everything, so it yields
//!    the most-recent distinct command.
//! 4. **Tie-break / weighting.** Candidates are compared by, in order:
//!    - `cwd` match first: an entry whose `cwd` equals the request `cwd` beats
//!      one that does not (only when the request supplies a `cwd`);
//!    - then **recency**: the earlier a candidate appears in the
//!      most-recent-first, deduped order, the better;
//!    - then **frequency**: a higher occurrence `count` beats a lower one.
//!
//! The comparison is deterministic and total, so the result never depends on
//! iteration nondeterminism.

use autosuggest_protocol::{HistoryEntry, HistoryWindow};

/// Compute the single best ghost-text continuation for `prefix`.
///
/// Returns the full command string to complete the line to, or [`None`] if no
/// recent command starts with `prefix`. See the [module docs](self) for the
/// exact ordering and tie-break rules.
///
/// `cwd` is the working directory of the *request* (the shell's current
/// directory); when supplied it is used only to prefer history entries recorded
/// in the same directory.
///
/// # Examples
///
/// ```
/// use autosuggest_core::history::autosuggest;
/// use autosuggest_protocol::{HistoryEntry, HistoryWindow};
///
/// let window = HistoryWindow {
///     entries: vec![
///         HistoryEntry { command: "git push origin main".into(), cwd: None, exit_code: None, ts: None },
///         HistoryEntry { command: "git pull".into(), cwd: None, exit_code: None, ts: None },
///     ],
/// };
/// assert_eq!(
///     autosuggest("git pu", &window, None).as_deref(),
///     Some("git push origin main"),
/// );
/// assert_eq!(autosuggest("cargo", &window, None), None);
/// ```
pub fn autosuggest(prefix: &str, window: &HistoryWindow, cwd: Option<&str>) -> Option<String> {
    // Step 1 + 2: produce a most-recent-first, deduped candidate list with
    // per-command occurrence counts.
    let candidates = distinct_recent_first(window);

    // Step 3 + 4: filter by prefix, then pick the best per the tie-break order.
    candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.command.starts_with(prefix))
        .max_by(|(a_idx, a), (b_idx, b)| {
            // `max_by` keeps the *last* maximal element on ties; our ordering is
            // total (recency is strictly distinct per index), so there are no
            // ambiguous ties to worry about.
            rank_key(a, *a_idx, cwd).cmp(&rank_key(b, *b_idx, cwd))
        })
        .map(|(_, c)| c.command.to_string())
}

/// A distinct command retained during dedupe, with its frequency.
struct Candidate<'a> {
    /// The command string (borrowed from the window).
    command: &'a str,
    /// The entry's recorded working directory, if any.
    cwd: Option<&'a str>,
    /// How many times this command occurred in the window.
    count: u32,
}

/// Ordering key for a candidate. Larger compares as "better".
///
/// Tuple order encodes priority: `cwd` match, then recency, then frequency.
/// Recency is expressed as the *negated* dedupe index so that a smaller index
/// (more recent) yields a larger key.
fn rank_key(c: &Candidate<'_>, dedupe_index: usize, req_cwd: Option<&str>) -> (bool, i64, u32) {
    let cwd_match = match (req_cwd, c.cwd) {
        (Some(req), Some(entry)) => req == entry,
        _ => false,
    };
    let recency = -(dedupe_index as i64);
    (cwd_match, recency, c.count)
}

/// Build the most-recent-first, deduped candidate list (`TECH.md §3.1` steps
/// 1–2).
///
/// The returned vector is ordered most-recent-first; each distinct command
/// appears once, carrying the `cwd` of its most-recent occurrence and the total
/// number of occurrences across the whole window.
fn distinct_recent_first(window: &HistoryWindow) -> Vec<Candidate<'_>> {
    let ordered = recent_first_order(&window.entries);

    let mut candidates: Vec<Candidate<'_>> = Vec::new();
    for entry in ordered {
        if let Some(existing) = candidates
            .iter_mut()
            .find(|c| c.command == entry.command.as_str())
        {
            // Seen already (an older occurrence): bump frequency only. The first
            // time we saw it is the most-recent one, so keep that cwd/position.
            existing.count = existing.count.saturating_add(1);
        } else {
            candidates.push(Candidate {
                command: entry.command.as_str(),
                cwd: entry.cwd.as_deref(),
                count: 1,
            });
        }
    }
    candidates
}

/// Return references to `entries` in most-recent-first order.
///
/// If every entry carries a `ts`, sort by `ts` descending (stable, so equal
/// timestamps preserve the given relative order). Otherwise trust the given
/// order and assume it is already most-recent-first (`SCHEMA.md §3`).
fn recent_first_order(entries: &[HistoryEntry]) -> Vec<&HistoryEntry> {
    let mut refs: Vec<&HistoryEntry> = entries.iter().collect();
    let all_have_ts = !refs.is_empty() && refs.iter().all(|e| e.ts.is_some());
    if all_have_ts {
        // `ts` is present on all entries here, so the closures only ever see
        // `Some`; default to 0 purely to keep the comparator total.
        refs.sort_by_key(|e| std::cmp::Reverse(e.ts.unwrap_or(0)));
    }
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use autosuggest_protocol::HistoryEntry;

    /// Build a window from `(command, cwd, ts)` triples, in the given order.
    fn window(entries: &[(&str, Option<&str>, Option<i64>)]) -> HistoryWindow {
        HistoryWindow {
            entries: entries
                .iter()
                .map(|(cmd, cwd, ts)| HistoryEntry {
                    command: (*cmd).to_string(),
                    cwd: cwd.map(str::to_string),
                    exit_code: None,
                    ts: *ts,
                })
                .collect(),
        }
    }

    #[test]
    fn exact_prefix_match_returns_most_recent() {
        // Most-recent-first order; two `git p...` candidates exist.
        let w = window(&[
            ("git pull", None, None),
            ("git push origin main", None, None),
            ("ls -la", None, None),
        ]);
        assert_eq!(
            autosuggest("git p", &w, None).as_deref(),
            Some("git pull"),
            "the most-recent matching command must win"
        );
    }

    #[test]
    fn dedupe_keeps_most_recent_occurrence() {
        // `git status` appears twice; the most-recent (index 0) governs.
        let w = window(&[
            ("git status", Some("/a"), None),
            ("git commit", None, None),
            ("git status", Some("/b"), None),
        ]);
        let candidates = distinct_recent_first(&w);
        let git_status = candidates
            .iter()
            .find(|c| c.command == "git status")
            .expect("git status candidate present");
        assert_eq!(git_status.count, 2, "frequency counts both occurrences");
        assert_eq!(
            git_status.cwd,
            Some("/a"),
            "dedupe keeps the most-recent occurrence's cwd"
        );
        // Three entries collapse to two distinct commands.
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn empty_prefix_returns_most_recent_distinct_command() {
        let w = window(&[
            ("cargo test", None, None),
            ("cargo build", None, None),
            ("cargo test", None, None),
        ]);
        assert_eq!(
            autosuggest("", &w, None).as_deref(),
            Some("cargo test"),
            "empty prefix yields the most-recent distinct command"
        );
    }

    #[test]
    fn no_match_returns_none() {
        let w = window(&[("git push", None, None), ("ls", None, None)]);
        assert_eq!(autosuggest("docker ", &w, None), None);
    }

    #[test]
    fn empty_window_returns_none() {
        let w = window(&[]);
        assert_eq!(autosuggest("", &w, None), None);
        assert_eq!(autosuggest("git", &w, None), None);
    }

    #[test]
    fn cwd_weighting_breaks_ties() {
        // Two distinct matches; the more-recent one is in a different cwd, the
        // older one matches the request cwd. cwd match must outrank recency.
        let w = window(&[
            ("git push origin feature", Some("/other"), None),
            ("git push origin main", Some("/repo"), None),
        ]);
        assert_eq!(
            autosuggest("git push", &w, Some("/repo")).as_deref(),
            Some("git push origin main"),
            "an entry recorded in the request cwd is preferred"
        );
        // Without a request cwd, recency wins instead.
        assert_eq!(
            autosuggest("git push", &w, None).as_deref(),
            Some("git push origin feature"),
            "with no request cwd, the most-recent match wins"
        );
    }

    #[test]
    fn ts_ordering_applied_when_present() {
        // Given order is NOT most-recent-first, but every entry has a ts, so the
        // safeguard sort by ts desc must reorder them.
        let w = window(&[
            ("git old", None, Some(100)),
            ("git newest", None, Some(300)),
            ("git middle", None, Some(200)),
        ]);
        assert_eq!(
            autosuggest("git", &w, None).as_deref(),
            Some("git newest"),
            "ts desc sort selects the highest-timestamp match"
        );
    }

    #[test]
    fn ts_ordering_is_stable_for_equal_timestamps() {
        // Equal ts: stable sort preserves the given (most-recent-first) order.
        let w = window(&[
            ("git first", None, Some(500)),
            ("git second", None, Some(500)),
        ]);
        assert_eq!(
            autosuggest("git", &w, None).as_deref(),
            Some("git first"),
            "equal timestamps keep given order; first stays most-recent"
        );
    }

    #[test]
    fn partial_ts_falls_back_to_given_order() {
        // Not all entries carry ts, so we must NOT sort; trust given order.
        let w = window(&[
            ("git given-recent", None, None),
            ("git has-ts", None, Some(9_999)),
        ]);
        assert_eq!(
            autosuggest("git", &w, None).as_deref(),
            Some("git given-recent"),
            "mixed ts presence falls back to the given most-recent-first order"
        );
    }

    #[test]
    fn prefix_match_is_case_sensitive() {
        let w = window(&[("Git push", None, None)]);
        assert_eq!(
            autosuggest("git", &w, None),
            None,
            "matching is case-sensitive per the spec"
        );
        assert_eq!(autosuggest("Git", &w, None).as_deref(), Some("Git push"));
    }

    #[test]
    fn frequency_breaks_ties_after_recency_and_cwd() {
        // Two distinct matches, neither cwd-relevant. Recency would pick the
        // first, but here we verify frequency only matters once recency/cwd are
        // equal — which they never are across distinct commands — so the most
        // recent still wins. This documents that recency dominates frequency.
        let w = window(&[
            ("git rare", None, None),
            ("git common", None, None),
            ("git common", None, None),
            ("git common", None, None),
        ]);
        assert_eq!(
            autosuggest("git", &w, None).as_deref(),
            Some("git rare"),
            "recency outranks raw frequency"
        );
    }
}
