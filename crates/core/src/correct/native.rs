//! Native correction predicates (`SCHEMA.md §2.1`).
//!
//! Some corrections need logic that pure JSON `match`/`rewrite` cannot express.
//! These are registered in `rules/` as `{ "id", "native": true, "priority" }`
//! entries (so ordering stays data-driven) and implemented here, keyed by `id`:
//!
//! * [`no_command`] — the base command is not on `$PATH`; suggest the nearest
//!   PATH entry within Levenshtein distance ≤ 2, ranked by distance then
//!   frequency (more frequent PATH names win ties).
//! * [`subcommand_typo`] — an unknown subcommand for a known spec; suggest the
//!   nearest known subcommand from the matching spec.
//!
//! Each returns the *rewritten script strings* (best first); the engine attaches
//! the owning rule's id/description for display.

use std::collections::HashMap;

use crate::correct::levenshtein::distance;
use crate::correct::resolver::CommandResolver;
use crate::correct::CorrectContext;
use crate::types::Subcommand;

/// Maximum edit distance for a "did you mean" suggestion (`SCHEMA.md §2.1`).
const MAX_EDIT_DISTANCE: usize = 2;

/// Split a script into whitespace-separated tokens, preserving none of the
/// original spacing. Correction operates on a normalized single-space form.
fn tokens(script: &str) -> Vec<&str> {
    script.split_whitespace().collect()
}

/// Rebuild a script from tokens with single-space separators.
fn join(tokens: &[String]) -> String {
    tokens.join(" ")
}

/// `no_command`: when the base command is unknown, suggest the nearest command
/// on `$PATH` (`SCHEMA.md §2.1`).
///
/// Returns rewritten scripts (base command swapped for each candidate), best
/// first. Candidates are PATH entries within [`MAX_EDIT_DISTANCE`] of the base
/// command, ranked by ascending edit distance, then by descending frequency on
/// `$PATH` (a name appearing in more PATH directories ranks higher), then
/// lexicographically for stability.
///
/// Emits nothing if the base command already exists, the script is empty, or no
/// candidate is close enough.
pub fn no_command(ctx: &CorrectContext<'_>, resolver: &dyn CommandResolver) -> Vec<String> {
    let toks = tokens(ctx.script);
    let Some(&base) = toks.first() else {
        return Vec::new();
    };

    // If the command resolves, there is nothing to correct here.
    if resolver.exists(base) {
        return Vec::new();
    }

    // Frequency of each command name across PATH (more dirs => higher).
    let mut freq: HashMap<String, usize> = HashMap::new();
    for cmd in resolver.path_commands() {
        *freq.entry(cmd).or_insert(0) += 1;
    }

    // Rank candidates by (distance asc, frequency desc, name asc).
    let mut ranked: Vec<(usize, usize, String)> = freq
        .iter()
        .filter_map(|(name, &count)| {
            let d = distance(base, name);
            (d <= MAX_EDIT_DISTANCE && d > 0).then(|| (d, count, name.clone()))
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(&b.2))
    });

    ranked
        .into_iter()
        .map(|(_, _, name)| {
            let mut owned: Vec<String> = toks.iter().map(|s| s.to_string()).collect();
            owned[0] = name;
            join(&owned)
        })
        .collect()
}

/// `subcommand_typo`: when the first argument is an unknown subcommand of a known
/// spec, suggest the nearest known subcommand (`SCHEMA.md §2.1`).
///
/// `specs` is the set of loaded command specs; the matching spec is the one whose
/// canonical name (or an alias) equals the base command. Returns rewritten
/// scripts (the typo'd subcommand token swapped for each candidate), ranked by
/// ascending edit distance then by the subcommand's canonical name.
///
/// Emits nothing when there is no matching spec, no second token, the second
/// token is already a known subcommand, or nothing is close enough.
pub fn subcommand_typo(ctx: &CorrectContext<'_>, specs: &[Subcommand]) -> Vec<String> {
    let toks = tokens(ctx.script);
    let (Some(&base), Some(&sub)) = (toks.first(), toks.get(1)) else {
        return Vec::new();
    };

    // Find the spec whose name set includes the base command.
    let Some(spec) = specs
        .iter()
        .find(|s| s.name.all().iter().any(|n| n == base))
    else {
        return Vec::new();
    };

    // Collect known subcommand names (canonical + aliases).
    let mut known: Vec<&str> = Vec::new();
    for s in &spec.subcommands {
        for n in s.name.all() {
            known.push(n.as_str());
        }
    }

    // Already a valid subcommand => nothing to correct.
    if known.contains(&sub) {
        return Vec::new();
    }

    let mut ranked: Vec<(usize, &str)> = known
        .iter()
        .filter_map(|&k| {
            let d = distance(sub, k);
            (d <= MAX_EDIT_DISTANCE && d > 0).then_some((d, k))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));

    ranked
        .into_iter()
        .map(|(_, candidate)| {
            let mut owned: Vec<String> = toks.iter().map(|s| s.to_string()).collect();
            owned[1] = candidate.to_string();
            join(&owned)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::correct::resolver::MockCommandResolver;

    fn ctx<'a>(script: &'a str, specs: &'a [Subcommand]) -> CorrectContext<'a> {
        CorrectContext {
            script,
            stderr: "",
            exit_code: Some(127),
            cwd: None,
            env: None,
            specs,
        }
    }

    #[test]
    fn no_command_suggests_nearest_path_entry() {
        let specs: Vec<Subcommand> = Vec::new();
        let resolver = MockCommandResolver::new(["ls", "cat", "git"]);
        let out = no_command(&ctx("sl -la", &specs), &resolver);
        assert_eq!(out.first().map(String::as_str), Some("ls -la"));
    }

    #[test]
    fn no_command_empty_when_command_exists() {
        let specs: Vec<Subcommand> = Vec::new();
        let resolver = MockCommandResolver::new(["ls"]);
        assert!(no_command(&ctx("ls", &specs), &resolver).is_empty());
    }

    #[test]
    fn no_command_empty_when_nothing_close() {
        let specs: Vec<Subcommand> = Vec::new();
        let resolver = MockCommandResolver::new(["docker"]);
        assert!(no_command(&ctx("xyzzy", &specs), &resolver).is_empty());
    }

    #[test]
    fn subcommand_typo_suggests_known_subcommand() {
        let git: Subcommand = serde_json::from_str(include_str!("../../../../specs/git.spec.json"))
            .expect("git spec");
        let specs = vec![git];
        let out = subcommand_typo(&ctx("git comit -m x", &specs), &specs);
        assert_eq!(out.first().map(String::as_str), Some("git commit -m x"));
    }

    #[test]
    fn subcommand_typo_empty_for_valid_subcommand() {
        let git: Subcommand = serde_json::from_str(include_str!("../../../../specs/git.spec.json"))
            .expect("git spec");
        let specs = vec![git];
        assert!(subcommand_typo(&ctx("git commit", &specs), &specs).is_empty());
    }
}
