# CONTRIBUTING.md — working on `autosuggest-core`

## Originality & licensing (read first)

This project ships an **original** completion dataset and correction ruleset.

- **Do** study the *shape* of prior art (declarative completion schemas,
  correction-rule engines) — structure and ideas are not copyrightable.
- **Do not** copy spec/rule *content*, descriptions, or data files from any
  third-party project (Fig/`withfig/autocomplete`, Warp, thefuck, etc.).
- Every spec and rule must be authored from primary sources: the tool's own
  `--help`, man pages, and observed behaviour.
- If unsure whether something is too close to a source, ask the orchestrator
  before committing.

## Workflow (subagents)

1. You are assigned one task = one milestone branch `feat/m<N>-<slug>` in a
   dedicated worktree under `.worktrees/`.
2. Implement strictly to `SCHEMA.md` (normative) and `TECH.md` (architecture).
3. Definition of done:
   - Conforms to `SCHEMA.md` formats exactly.
   - Golden tests for your scope are green (`cargo test`).
   - `cargo fmt --check` and `cargo clippy -- -D warnings` are clean.
   - Latency budgets met where applicable (`cargo bench`).
   - No copied third-party data (see above).
   - No new dependencies beyond those listed in `TECH.md §1` without approval.
4. Report back: branch name, what changed, test output, and any deviations.
5. The orchestrator audits and either merges or returns the branch with a
   specific defect list to resolve.

## Code conventions

- Edition 2021, MSRV 1.80. `core` crate does **no** direct I/O.
- Errors: `thiserror`-style typed errors in libs; no `unwrap()` in non-test code.
- Public APIs documented with `///`. Keep functions small and focused.
- Tests live beside code (`#[cfg(test)]`) and as fixtures under `tests/`.

## Commit messages

Conventional, present-tense, scoped: `feat(parse): posix flag chaining`,
`test(correct): mkdir -p case`, `docs(schema): clarify cursor offset`.
