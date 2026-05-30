# CONTRIBUTING.md — working on `autosuggest-core`

## Originality & licensing (read this before writing anything)

This project ships an **original** completion dataset and correction ruleset.
Every spec and rule was authored from primary sources (`--help`, man pages,
observed behaviour). **No content was copied** from third-party completion
projects.

- **Do** study the *shape* of prior art (declarative completion schemas,
  correction-rule engines) — structure and ideas are not copyrightable.
- **Do not** copy spec/rule *content*, descriptions, or data files from any
  third-party project — including **Fig** (`withfig/autocomplete`), **Warp**,
  **thefuck**, **fish shell completions**, **carapace**, **click-completion**,
  or any other autocomplete provider.
- If you are writing a spec for a command that also exists in Fig or another
  project, close those files first. Write from `--help` and man pages only.
- Every new file you commit must pass the **originality checklist** below.

### Originality checklist

Before committing any new spec or rule:

- [ ] I authored this spec from the tool's `--help` output and/or man page
- [ ] I did not reference any third-party completion file during authoring
- [ ] Descriptions are my own rephrasing, not copied from another project
- [ ] The spec covers only flags/args that actually exist in the tool today
- [ ] I checked `tests/fixtures/` golden output is deterministic (no
      environment-dependent drift)

## Adding a completion spec

Specs live in `specs/<command>.spec.json` and follow the schema in `SCHEMA.md`.

### Quick reference

```jsonc
{
  "name": "mycmd",                    // string or array (first = canonical, rest = aliases)
  "description": "Does a thing",     // <= 120 chars
  "options": [
    { "name": "-v", "description": "Verbose output" },
    { "name": "--version", "description": "Show version", "exclusiveOn": ["-v"] },
    { "name": "--format", "description": "Output format (json|text)",
      "requiresSeparator": true,
      "args": [ { "name": "fmt", "suggestions": ["json", "text"] } ] }
  ],
  "subcommands": [
    { "name": "init", "description": "Initialize a thing",
      "options": [ ... ],
      "args": [ { "name": "path", "template": "folders" } ] }
  ],
  "args": [ { "name": "input", "template": "files" } ]
}
```

### How to write a spec

1. Run `<command> --help` and skim the man page. List every flag, subcommand,
   and positional argument.
2. Create `specs/<command>.spec.json`. Use existing specs as style reference
   (e.g. `specs/cd.spec.json` for a simple one, `specs/git.spec.json` for a
   deep subcommand tree).
3. Run `cargo test` to validate the spec parses and passes schema validation.
4. Run `cargo test -p autosuggest-core --test golden_complete` to generate
   golden fixtures and verify completion output.
   - If a golden fixture doesn't exist yet for this command, the test suite
     will tell you. You can add a fixture under `tests/fixtures/complete/`.
5. Update `COVERAGE.md` if the spec adds a new command to the dataset.

### How to write a correction rule

Rules live in `rules/<slug>.rule.json` and follow `SCHEMA.md §2`.

```jsonc
{
  "id": "my_rule",                   // unique, kebab-case
  "description": "Fix this problem", // <= 120 chars
  "priority": 90,                    // higher = tried first
  "match": {
    "scriptStartsWith": "badcmd ",   // match by script prefix
    "stderrContains": ["error"],     // match by stderr content
    "exitCodeIn": [1]                // match by exit code
  },
  "rewrite": {
    "insertFlag": {
      "after": "badcmd",             // word after which to insert
      "flag": "--fix"                // flag to insert
    }
  }
}
```

Available rewrite actions (see `SCHEMA.md §2.3`):
- **`insertFlag`** — insert a flag after a given word
- **`prefix`** — prepend text (e.g. `"sudo "`)
- **`regexReplace`** — find/replace with a regex

1. Identify a common command failure pattern and its fix.
2. Create `rules/<slug>.rule.json`.
3. Run `cargo test` to validate the rule parses and its id is unique.
4. Add a golden fixture under `tests/fixtures/correct/` so it's regression-tested.

## Code conventions

- Edition 2021, MSRV 1.80. `core` crate does **no** direct I/O.
- Errors: typed errors in libs; no `unwrap()` in non-test code.
- Public APIs documented with `///`. Keep functions small and focused.
- Tests live beside code (`#[cfg(test)]`) and as fixtures under `tests/`.

## Running tests

```shell
cargo test                    # all tests
cargo test --test golden_*    # golden fixture tests only
ASC_DUMP_GOLDEN=1 cargo test --test golden_complete  # refresh goldens
cargo clippy -- -D warnings
cargo fmt --check
```

## Commit messages

Conventional, present-tense, scoped: `feat(spec): add terraform spec`,
`fix(rule): mkdir_p edge case with trailing slash`,
`test(golden): refresh cd fixtures`.

## Originality check for PRs

Before a PR is merged, a reviewer must verify:

1. No spec description string appears verbatim in any third-party completion
   dataset. Spot-check 3–5 descriptions against Fig, Warp, and fish repos.
2. No rule rewrite matches a known third-party rule's exact regex or pattern.
3. The contributor has attested they didn't reference third-party sources
   (check the PR body for the attestation).

Use the bundled verification script if available, or search for suspicious
verbatim matches manually.
