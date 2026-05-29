# TECH.md — `autosuggest-core` technical specification

> Implements the product defined in `PRODUCT.md`. Data formats are normative in
> `SCHEMA.md`; this document is normative for architecture, module boundaries,
> algorithms, and the integration surface.

## 1. Language & toolchain

- **Rust** (edition 2021, MSRV 1.80). Workspace with multiple crates.
- Build: `cargo`. Tests: `cargo test` + golden fixtures. Benches: `criterion`.
- Lint gate: `cargo fmt --check` + `cargo clippy -D warnings`.
- Dependencies kept minimal: `serde`/`serde_json` (data), `rusqlite` (optional
  history feature, behind a cargo feature flag), `libc`/`cbindgen` (FFI adapter).
  No async runtime in core. No network crates anywhere.

## 2. Workspace layout

```
autosuggest-core/
├── Cargo.toml                # workspace
├── crates/
│   ├── core/                 # pure engine: no I/O except generator runner hook
│   │   ├── types/            # spec, suggestion, request/response models
│   │   ├── tokenize/         # command-line tokenizer
│   │   ├── parse/            # spec-tree parser / parse-state machine
│   │   ├── complete/         # completion candidate collection
│   │   ├── rank/             # filtering (prefix|fuzzy) + scoring
│   │   ├── history/          # history autosuggester (stateless algo)
│   │   └── correct/          # correction rule engine + native predicates
│   ├── data/                 # spec & rule loading, indexing, generator executor
│   ├── protocol/             # serde models for the stdio JSON protocol (versioned)
│   ├── daemon/               # bin: stdio JSON-lines server
│   ├── ffi/                  # cdylib: C ABI + cbindgen-generated header
│   └── history-store/        # optional SQLite store (feature = "sqlite-store")
├── specs/                    # authored *.spec.json dataset
├── rules/                    # authored *.rule.json correction rules
├── tests/                    # integration + golden tests
│   └── fixtures/             # golden inputs/outputs per command
├── benches/                  # criterion latency benches
└── docs/                     # SCHEMA.md, INTEGRATING.md, etc. (or repo root)
```

`core` MUST NOT perform file or network I/O directly. Generator execution is
injected via a `GeneratorRunner` trait so `core` stays pure and testable; the
`data` crate provides the real (sandboxed) runner.

## 3. Architecture

```
host → adapter (stdio daemon | C ABI) → protocol → engine core → dataset/rules
```

### 3.1 Pipelines

**complete:**
`tokenize(line, cursor)` → `parse(tokens, spec)` produces a `ParseState`
(current command path, consumed options, the "active" arg or token-to-complete)
→ `complete(ParseState)` gathers candidates (subcommands + options valid in
state + arg suggestions via template/generator/static) → `rank(query, candidates)`
filters by prefix or fuzzy and scores by `priority` + recency → top-N items.

**autosuggest:**
`history_autosuggest(prefix, window)` → first history entry (most-recent-first,
deduped) whose text starts with `prefix` (optionally cwd/exit-weighted) →
ghost-text remainder.

**correct:**
`correct(ctx)` where `ctx = {script, stderr, exit_code, cwd, env}` → evaluate
all rules whose `match` predicate holds → each emits 0+ candidate rewrites →
dedupe + rank by rule `priority` → top-N corrected command strings.

### 3.2 Parser / parse-state machine (the riskiest component)

State tracked while consuming tokens left-to-right:
- `command_path`: resolved spec node chain (root → subcommand → ...).
- `seen_options`: set, to enforce `exclusiveOn`, `dependsOn`, non-repeatable.
- `pending_arg`: an option that still needs its argument value.
- `arg_index`: position within the current subcommand's `args`.
- `parser_directives`: `optionsMustPrecedeArguments`, `flagsArePosixNoncompliant`
  (chained short flags `-lah`), `optionArgSeparators` (`=`/space), `requiresSeparator`.

Output `ParseState` classifies the cursor token as one of:
`Subcommand | Option | OptionArgument(option) | CommandArgument(arg) | Empty`.
Completion uses this to decide which candidate sets are valid.

### 3.3 Ranking

`score = base_priority/100 * w_p + recency_boost * w_r + match_quality * w_m`
where `match_quality` is exact-prefix > prefix > fuzzy-subsequence, and
`recency_boost` derives from optional history frequency passed by the host.
Stable sort, ties broken by shorter `insert` then lexicographic.

### 3.4 Generators (dynamic args)

Declarative only (see `SCHEMA.md`). The `data` crate's runner:
1. Verifies `run[0]` ∈ allow-list (e.g. `git`, `cargo`, `npm`, `docker`, `ls`).
2. Spawns with a hard timeout (default 200 ms) and captures stdout.
3. Splits via `splitOn` and/or applies declared trim/regex extraction.
4. Caches by `(run, cwd)` for `cache.ttlMs`.
No shell interpolation; args are passed as an argv vector, never a shell string.

### 3.5 Correction engine

- JSON rules: `match` (predicates over `script`/`stderr`/`exit_code`/`cwd`) +
  `rewrite` (insert flag, replace token, prefix `sudo`, swap subcommand).
- Native predicates (not expressible in pure JSON), each a small Rust fn:
  - `no_command`: scan `$PATH`, Levenshtein ≤ 2 → did-you-mean.
  - `subcommand_typo`: Levenshtein vs the spec's known subcommands.
  - others as needed (kept behind a registry keyed by id).

## 4. Integration surface

### 4.1 stdio daemon (primary, universal)
Line-delimited JSON (one request per line, one response per line). Versioned
envelope (`"v":1`). Defined normatively in `SCHEMA.md §4`. The daemon is
single-threaded request/response by default; concurrency optional via `id`.

### 4.2 C ABI (`cdylib`, native embedding)
`cbindgen`-generated header. Minimal surface:
```c
AscEngine* asc_engine_new(const char* specs_dir, const char* rules_dir);
char*      asc_complete(AscEngine*, const char* request_json);   // returns JSON
char*      asc_autosuggest(AscEngine*, const char* request_json);
char*      asc_correct(AscEngine*, const char* request_json);
void       asc_string_free(char*);
void       asc_engine_free(AscEngine*);
```
All strings UTF-8; engine owns nothing the caller passes; returned strings freed
via `asc_string_free`. No panics cross the boundary (catch_unwind at the edge).

## 5. Testing strategy

- **Golden tests**: per command, fixture pairs `(request.json, expected.json)`
  under `tests/fixtures/<capability>/<command>/`. A harness asserts engine
  output equals expected (order-sensitive for ranking).
- **Property tests**: tokenizer/parser round-trips and never panics on arbitrary
  input (`proptest`).
- **Correction cases**: a table of `(script, stderr, exit) → expected[0]`.
- **Benches**: `criterion` gates NFR1 latency.
- **Definition of done per task** = matches `SCHEMA.md`, golden tests green,
  `clippy -D warnings` clean, latency budget met, no copied third-party data.

## 6. Milestone → module mapping

| Milestone | Crates/modules touched |
|-----------|------------------------|
| M0 | `types`, `protocol`, schemas, fixtures harness, seed specs |
| M1 | `tokenize`, `parse`, `complete`, `rank`, filepaths/folders templates |
| M2 | `history` |
| M3 | `correct` + JSON rules + native predicates |
| M4 | `data` generator runner + cache, generator-backed specs |
| M5 | `daemon`, `ffi`, reference host demo |
| M6 | spec/rule dataset growth, docs |

See `ROADMAP.md` for sequencing, parallelism, and exit criteria.
