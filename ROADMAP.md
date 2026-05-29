# ROADMAP.md — milestones, sequencing & status

> Orchestrator-owned. Implementation is done by subagents in isolated git
> worktrees under `.worktrees/`. The orchestrator audits each returned branch
> against `SCHEMA.md` + `TECH.md` and merges only if it conforms.

## Conventions

- One branch per task: `feat/m<N>-<slug>` (e.g. `feat/m1-completer`).
- Worktrees live in `.worktrees/<branch-slug>/` (gitignored).
- Max **3** concurrent subagents during the M1–M3 fan-out.
- Definition of done (every task): conforms to `SCHEMA.md`; golden tests green;
  `cargo fmt --check` + `cargo clippy -D warnings` clean; latency budget met
  where applicable; **no copied third-party completion data**.

## Dependency graph

```
M0 (serial, blocks all)
 ├─ M1 completer ─┐
 ├─ M2 history    ├─ (parallel, cap 3)
 └─ M3 corrector ─┘
       │
       ├─ M4 generators        (needs M1)
       ├─ M5 adapters daemon+ffi (needs M1,M2,M3)
       └─ M6 dataset+docs        (needs M1,M3)
```

## Milestones

### M0 — Skeleton & contracts  `feat/m0-skeleton`  — STATUS: ✅ merged (77c6305)
Cargo workspace; `types` + `protocol` crates implementing every model in
`SCHEMA.md §1–4`; serde round-trip tests; golden-test harness; `GeneratorRunner`
trait stub; ~5 seed specs (`ls`, `cd`, `mkdir`, `echo`, `git` minimal) that
**parse and validate** (no completion logic yet).
**Exit:** `cargo test` green; all schema models (de)serialize; harness loads
fixtures; clippy clean.

### M1 — As-you-type completion  `feat/m1-completer`  — STATUS: ✅ merged (5bf5075)
`tokenize`, `parse` (parse-state machine + parser directives), `complete`,
`rank` (prefix + fuzzy, priority+recency); `filepaths`/`folders` templates.
**Exit:** golden tests for `ls cd mkdir cp mv rm cat grep git` pass;
completion `<5 ms` bench; clippy clean.

### M2 — History autosuggestion  `feat/m2-history`  — STATUS: ✅ merged (cae41d0)
Stateless `history_autosuggest(prefix, window)`; dedupe; optional cwd/exit
weighting.
**Exit:** golden tests on a recorded history fixture pass; clippy clean.

### M3 — Failed-command correction  `feat/m3-corrector`  — STATUS: ✅ merged (3577940)
Rule engine + JSON rule loader + native predicates (`no_command`,
`subcommand_typo`, `mkdir -p`, `sudo`, `cd` typo, `-r` fixes).
**Exit:** correction table incl. `mkdir`/`sl`→`ls`/`git comit`/`sudo` passes;
clippy clean.
**Carry-over (→ M6):** add `tests/fixtures/correct/` golden pairs for parity
with M1/M2 (M3 shipped a case-table; DoD met, goldens deferred).

### M4 — Generators + caching  `feat/m4-generators`  — STATUS: in progress
`data` crate sandboxed generator runner (allow-list, timeout, TTL cache);
generator-backed specs (`git checkout <branch>`, `git add <file>`).
**Exit:** dynamic suggestions correct; `<15 ms` warm bench; allow-list enforced.

### M5 — Adapters: daemon + C ABI  `feat/m5-adapters`  — STATUS: in progress
`daemon` (stdio JSON lines per `SCHEMA.md §4`); `ffi` cdylib + cbindgen header
(`TECH.md §4.2`); reference mock-host demo exercising all three ops.
**Exit:** end-to-end demo runs over stdio AND via C ABI; no panics cross FFI.

### M6 — Dataset growth + docs  `feat/m6-dataset`  — STATUS: blocked(M1,M3)
Expand to the full `PRODUCT.md §7` coverage (~45–55 specs + rule set); finalize
`INTEGRATING.md` recipe; coverage report.
**Exit:** coverage list authored & tested; docs complete.

## Progress log

| Date | Milestone | Event | Result |
|------|-----------|-------|--------|
| (init) | — | Spec docs authored & committed | baseline |
| (init) | M0 | Subagent built workspace/types/protocol/harness/seed specs; orchestrator audited (schema+fmt+clippy+test) | ✅ merged 77c6305 |
| (init) | M1/M2/M3 | Worktrees created, subagents dispatched in parallel (cap 3) | in progress |
| (init) | M1 | Completion engine; audited (fmt/clippy/80 tests, bench <5ms, original specs) | ✅ merged 5bf5075 |
| (init) | M2 | History autosuggester; audited (fmt/clippy/44 tests, 6 goldens) | ✅ merged cae41d0 |
| (init) | M3 | Correction engine; audited (fmt/clippy/59 tests, case table) | ✅ merged 3577940 |
| (init) | — | Merged tree verified: fmt/clippy clean, 123 tests green | ✅ |
| (init) | M4/M5 | Worktrees created, subagents dispatched (M6 held: needs M1+M3 only, runs after) | in progress |
