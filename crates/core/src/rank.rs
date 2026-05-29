//! Filtering and scoring of completion candidates (`TECH.md §3.3`).
//!
//! Takes the raw [`Candidate`]s from [`crate::complete`], filters them against
//! the cursor `query`, scores the survivors, and returns the top-N
//! [`CompletionItem`]s in descending score order.
//!
//! ## Match quality
//!
//! A candidate matches if any of its `match_names` (or its insert text)
//! relates to the query as one of, best first:
//!
//! 1. **exact** — the name equals the query,
//! 2. **prefix** — the name starts with the query,
//! 3. **fuzzy** — the query is an in-order subsequence of the name.
//!
//! An empty query matches everything as a prefix match (the whole slot is being
//! completed). Candidates with no relation to a non-empty query are dropped.
//!
//! ## Score
//!
//! ```text
//! score = base_priority/100 * W_P + recency_boost * W_R + match_quality * W_M
//! ```
//!
//! `recency_boost` comes from optional host-supplied history frequency; in M1
//! no history is wired in, so it is `0`. The result is clamped to `0.0..=1.0`
//! to satisfy the protocol's score range (`protocol::Item`).
//!
//! ## Ordering
//!
//! Stable sort by descending score; ties broken by shorter `insert`, then
//! lexicographic `insert`. Hidden candidates are excluded unless the query
//! matches one of their names exactly. At most [`DEFAULT_TOP_N`] items return.

use crate::complete::Candidate;

/// Scoring weights from `TECH.md §3.3`. They sum to `1.0` so a perfect
/// candidate (priority 100, full recency, exact match) scores `1.0`.
const W_PRIORITY: f64 = 0.5;
const W_RECENCY: f64 = 0.2;
const W_MATCH: f64 = 0.3;

/// Default maximum number of items returned.
pub const DEFAULT_TOP_N: usize = 50;

/// Default authored priority when a candidate declares none (`SCHEMA.md §1.4`).
const DEFAULT_PRIORITY: u8 = 50;

/// A ranked, host-facing completion item.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletionItem {
    /// Text to insert when accepted.
    pub insert: String,
    /// Display label.
    pub display: String,
    /// Optional short description.
    pub desc: Option<String>,
    /// Final score in `0.0..=1.0`.
    pub score: f64,
    /// Host may warn before accepting.
    pub dangerous: bool,
    /// Marked deprecated.
    pub deprecated: bool,
}

/// How a candidate name relates to the query, best first.
#[derive(Debug, Clone, Copy, PartialEq)]
enum MatchQuality {
    Exact,
    Prefix,
    Fuzzy,
}

impl MatchQuality {
    /// Numeric quality in `0.0..=1.0` feeding the score formula.
    fn weight(self) -> f64 {
        match self {
            MatchQuality::Exact => 1.0,
            MatchQuality::Prefix => 0.75,
            MatchQuality::Fuzzy => 0.4,
        }
    }
}

/// Filter, score, and order `candidates` for `query`, returning the top
/// [`DEFAULT_TOP_N`] items in descending score order.
pub fn rank(candidates: Vec<Candidate>, query: &str) -> Vec<CompletionItem> {
    rank_top_n(candidates, query, DEFAULT_TOP_N)
}

/// As [`rank`] but with an explicit item cap.
pub fn rank_top_n(candidates: Vec<Candidate>, query: &str, top_n: usize) -> Vec<CompletionItem> {
    let mut scored: Vec<(CompletionItem, MatchQuality)> = candidates
        .into_iter()
        .filter_map(|c| score_candidate(c, query))
        .collect();

    // Stable sort: descending score, then shorter insert, then lexicographic.
    scored.sort_by(|a, b| {
        b.0.score
            .partial_cmp(&a.0.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.insert.len().cmp(&b.0.insert.len()))
            .then_with(|| a.0.insert.cmp(&b.0.insert))
    });

    scored.truncate(top_n);
    scored.into_iter().map(|(item, _)| item).collect()
}

/// Score one candidate against `query`, or `None` if it does not match (or is
/// hidden without an exact match).
fn score_candidate(c: Candidate, query: &str) -> Option<(CompletionItem, MatchQuality)> {
    let quality = best_quality(&c, query)?;

    // Hidden candidates appear only on an exact-name match.
    if c.hidden && quality != MatchQuality::Exact {
        return None;
    }

    let priority = f64::from(c.priority.unwrap_or(DEFAULT_PRIORITY)) / 100.0;
    let recency_boost = 0.0; // M1: no history wired in.
    let raw = priority * W_PRIORITY + recency_boost * W_RECENCY + quality.weight() * W_MATCH;
    let score = raw.clamp(0.0, 1.0);

    let display = c.display.unwrap_or_else(|| c.insert.clone());
    Some((
        CompletionItem {
            insert: c.insert,
            display,
            desc: c.desc,
            score,
            dangerous: c.dangerous,
            deprecated: c.deprecated,
        },
        quality,
    ))
}

/// The best match quality across all of a candidate's names, or `None`.
///
/// Matching uses the bare name forms; for options, leading dashes are part of
/// the typed query so they are compared verbatim. An empty query is a prefix
/// match against everything.
fn best_quality(c: &Candidate, query: &str) -> Option<MatchQuality> {
    if query.is_empty() {
        return Some(MatchQuality::Prefix);
    }
    let names = if c.match_names.is_empty() {
        std::slice::from_ref(&c.insert)
    } else {
        c.match_names.as_slice()
    };
    names
        .iter()
        .filter_map(|name| match_quality(name, query))
        .min_by(|a, b| quality_rank(*a).cmp(&quality_rank(*b)))
}

/// Lower rank == better quality (so `min_by` selects the best).
fn quality_rank(q: MatchQuality) -> u8 {
    match q {
        MatchQuality::Exact => 0,
        MatchQuality::Prefix => 1,
        MatchQuality::Fuzzy => 2,
    }
}

/// Quality of a single `name` against `query`, or `None` if unrelated.
fn match_quality(name: &str, query: &str) -> Option<MatchQuality> {
    if name == query {
        Some(MatchQuality::Exact)
    } else if name.starts_with(query) {
        Some(MatchQuality::Prefix)
    } else if is_subsequence(query, name) {
        Some(MatchQuality::Fuzzy)
    } else {
        None
    }
}

/// True when `needle` is an in-order (not necessarily contiguous) subsequence of
/// `haystack`. Comparison is case-sensitive (specs use canonical casing).
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars();
    for nc in needle.chars() {
        loop {
            match hay.next() {
                Some(hc) if hc == nc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(insert: &str, names: &[&str]) -> Candidate {
        Candidate {
            insert: insert.to_string(),
            display: None,
            desc: None,
            priority: None,
            dangerous: false,
            deprecated: false,
            hidden: false,
            match_names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_query_keeps_all() {
        let items = rank(vec![cand("a", &["a"]), cand("b", &["b"])], "");
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn prefix_filters_non_matches() {
        let items = rank(
            vec![cand("status ", &["status"]), cand("commit ", &["commit"])],
            "st",
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "status ");
    }

    #[test]
    fn exact_outranks_prefix() {
        // Same priority; "go" exact should beat "google" prefix.
        let items = rank(vec![cand("google", &["google"]), cand("go", &["go"])], "go");
        assert_eq!(items[0].insert, "go");
    }

    #[test]
    fn fuzzy_matches_subsequence() {
        let items = rank(vec![cand("checkout ", &["checkout"])], "ckt");
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn unrelated_query_drops_candidate() {
        let items = rank(vec![cand("status ", &["status"])], "xyz");
        assert!(items.is_empty());
    }

    #[test]
    fn alias_match_via_match_names() {
        let items = rank(vec![cand("checkout ", &["checkout", "co"])], "co");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].insert, "checkout ");
    }

    #[test]
    fn higher_priority_scores_higher() {
        let mut hi = cand("a", &["a"]);
        hi.priority = Some(90);
        let mut lo = cand("ab", &["ab"]);
        lo.priority = Some(10);
        let items = rank(vec![lo, hi], "a");
        assert_eq!(items[0].insert, "a");
        assert!(items[0].score > items[1].score);
    }

    #[test]
    fn ties_break_by_shorter_then_lexicographic() {
        // Equal priority + equal (prefix) quality => shorter insert first.
        let items = rank(vec![cand("alpha", &["alpha"]), cand("al", &["al"])], "al");
        // "al" is an exact match, "alpha" only prefix, so exact wins first.
        assert_eq!(items[0].insert, "al");
    }

    #[test]
    fn hidden_excluded_unless_exact() {
        let mut h = cand("--secret", &["--secret"]);
        h.hidden = true;
        let none = rank(vec![h.clone()], "--sec");
        assert!(none.is_empty());
        let some = rank(vec![h], "--secret");
        assert_eq!(some.len(), 1);
    }

    #[test]
    fn score_within_unit_range() {
        let mut c = cand("x", &["x"]);
        c.priority = Some(100);
        let items = rank(vec![c], "x");
        assert!(items[0].score <= 1.0 && items[0].score >= 0.0);
    }

    #[test]
    fn top_n_caps_results() {
        let cands: Vec<Candidate> = (0..100).map(|i| cand(&format!("c{i:03}"), &[])).collect();
        let items = rank_top_n(cands, "", 10);
        assert_eq!(items.len(), 10);
    }

    #[test]
    fn subsequence_helper() {
        assert!(is_subsequence("ckt", "checkout"));
        assert!(is_subsequence("", "anything"));
        assert!(!is_subsequence("xyz", "checkout"));
        assert!(!is_subsequence("oo", "o"));
    }
}
