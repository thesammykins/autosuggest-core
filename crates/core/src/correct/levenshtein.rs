//! Levenshtein edit distance (own implementation, no external dependency).
//!
//! Used by the native correction predicates (`SCHEMA.md §2.1`) to find the
//! nearest known command or subcommand to a typo. This is the standard
//! two-row dynamic-programming formulation operating over Unicode scalar values
//! (`char`s), so multi-byte input is handled correctly.

/// Compute the Levenshtein distance between `a` and `b`.
///
/// The distance is the minimum number of single-character insertions,
/// deletions, or substitutions needed to transform `a` into `b`. Comparison is
/// over `char`s (Unicode scalar values).
///
/// Runs in `O(a.len() * b.len())` time and `O(min(a, b))` extra space.
pub fn distance(a: &str, b: &str) -> usize {
    // Work over chars for correct Unicode behaviour.
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();

    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    // Ensure the inner (column) dimension is the shorter one to bound memory.
    let (short, long) = if a.len() <= b.len() {
        (&a, &b)
    } else {
        (&b, &a)
    };

    // `prev[j]` = distance between long[..i] and short[..j].
    let mut prev: Vec<usize> = (0..=short.len()).collect();
    let mut curr: Vec<usize> = vec![0; short.len() + 1];

    for (i, &lc) in long.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &sc) in short.iter().enumerate() {
            let cost = if lc == sc { 0 } else { 1 };
            let deletion = prev[j + 1] + 1;
            let insertion = curr[j] + 1;
            let substitution = prev[j] + cost;
            curr[j + 1] = deletion.min(insertion).min(substitution);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[short.len()]
}

#[cfg(test)]
mod tests {
    use super::distance;

    #[test]
    fn identical_strings_have_zero_distance() {
        assert_eq!(distance("git", "git"), 0);
        assert_eq!(distance("", ""), 0);
    }

    #[test]
    fn empty_against_nonempty_is_length() {
        assert_eq!(distance("", "abc"), 3);
        assert_eq!(distance("abc", ""), 3);
    }

    #[test]
    fn single_edits() {
        assert_eq!(distance("sl", "ls"), 2); // transposition = 2 edits here
        assert_eq!(distance("comit", "commit"), 1); // one insertion
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("gti", "git"), 2);
    }

    #[test]
    fn is_symmetric() {
        assert_eq!(distance("flow", "wolf"), distance("wolf", "flow"));
        assert_eq!(distance("apt", "apk"), distance("apk", "apt"));
    }

    #[test]
    fn handles_unicode() {
        assert_eq!(distance("café", "cafe"), 1);
        assert_eq!(distance("naïve", "naive"), 1);
    }
}
