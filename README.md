# autosuggest-core

A lightweight, embeddable terminal completion & correction engine.

## What this is

A Rust library that takes a command-line context (buffer, cursor, cwd) and returns
ranked suggestions. It doesn't render a UI, manage a history database, or ship a
terminal — it's a **pure function the host calls on every keystroke**.

```
input:  "git ch" at cursor 6 in /repo
output: [
  { "insert": "checkout ", "score": 0.97, "desc": "Switch branches" },
  { "insert": "cherry-pick ", "score": 0.90, "desc": "Apply commits" },
  ...
]
```

## What it can do (verified)

| Capability | How it works | Verified by |
|---|---|---|
| **Spec-driven completion** | Subcommands, options, option-args, static suggestions, exclusive/depends-on relationships, argument generators | 111 spec files, 14 golden fixtures |
| **History autosuggestion** | Prefix-match against a recent-command window with dedup, cwd weighting, recency/frequency ranking | 6 golden fixtures |
| **Failed-command correction** | Rule engine (insert flag, prefix command, regex replace) against script+stderr+exit code | 15 rule files + 10 golden fixtures + native `no_command`/`subcommand_typo` |
| **Generator-backed args** | Constrained execution of allow-listed programs, output parsing, TTL cache | `echo`-based integration test (+ daemon E2E) |
| **C ABI** | Single `autosuggest_request_json` entry point, catch_unwind-guarded, cbindgen header | 3 FFI round-trip tests |
| **Stdio daemon** | JSON-lines protocol, all 3 ops, graceful degradation on bad input | 2 daemon E2E tests |

## Known limitations (not faked)

- **Spec data is authored from `--help`, not verified against real commands.**
  A spec says `ls` has `--color` with values `auto`/`always`/`never` — it was
  written by reading `ls --help`, not by testing every flag. We don't know which
  flags are rarely used, broken, or platform-specific.
- **Correction rules are hand-crafted for common patterns.** The 15 shipped rules
  cover `mkdir -p`, `sudo`, `cd` typos, subcommand typos, etc. They work on the
  cases we thought of and documented. They don't learn from real failures.
- **Generator tests run `echo`, not real commands.** The generator runner
  (allow-listed absolute executables, minimal environment, 100ms timeout, output
  capped) is tested with an allow-listed `echo` program. Real generators (`git
  branch`, `docker ps`, etc.) are wired in the spec files but run only in the
  daemon path. You own the security of what you put on the allow-list.
- **No filesystem completion tests.** The engine supports `template: "files"` and
  `template: "folders"`, but there are no golden fixtures for them (they'd be
  cwd-dependent and non-deterministic).
- **One fixture looks real; the rest are synthetic.** The `git_push_recorded`
  autosuggest fixture was captured from a real terminal session. Every other
  test fixture was crafted to exercise a specific code path.
- **111 specs / 15 rules.** That's useful coverage, not exhaustive coverage.
  It covers developer daily-drivers (git, docker, npm, cargo, ssh, etc.) and a
  selection of OS tools, but not every command on your PATH.

## Measured latency (criterion, M-series Mac)

Benchmarks run via `cargo bench`. All times are **mean μs**, measured on
optimized release builds.

| Scenario | Mean | Budget (< 5 ms) |
|---|---|---|
| `git ` subcommand listing | **5.4 μs** | ✅ |
| `git che` prefix match | **5.8 μs** | ✅ |
| `grep --` long option filter | **7.9 μs** | ✅ |
| `grep --color=a` option value | **5.6 μs** | ✅ |
| `git checkout <branch>` (warm cache) | **1.7 μs** | ✅ |

Generator-backed completions hit the cache after the first call; cold starts
add process spawn time (~1–10 ms depending on the command).

## Project state

All M0–M6 milestones are merged. Test suite: **~160 tests, all passing**.
Code lint gates: `cargo fmt --check` + `clippy -D warnings`.

## Workspace

| Crate | Role |
|---|---|
| `core` | Pure engine: types, tokenizer, parser, completer, ranker, history, correction |
| `protocol` | Serde models for stdio JSON protocol |
| `data` | Spec & rule loading, indexed engine, constrained generator runner |
| `daemon` | Binary: stdio JSON-lines server |
| `ffi` | cdylib: C ABI via cbindgen |
| `history-store` | Optional SQLite history persistence |

## Quick start

```shell
cargo build --release
cargo test
cargo bench
```

## Integration

```shell
autosuggest-daemon ./specs ./rules
{"v":1,"id":1,"op":"complete","line":"git ch","cursor":6}
```

Or link the C ABI for in-process embedding. See [INTEGRATING.md](./INTEGRATING.md).

## Documentation

| Document | What it covers |
|---|---|
| [PRODUCT.md](./PRODUCT.md) | Product vision, user stories, NFRs |
| [TECH.md](./TECH.md) | Architecture, algorithms, module boundaries |
| [SCHEMA.md](./SCHEMA.md) | Normative data and protocol formats |
| [INTEGRATING.md](./INTEGRATING.md) | How to embed the engine (daemon + C ABI) |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Adding specs/rules, originality policy |
| [COVERAGE.md](./COVERAGE.md) | What's covered and how it's tested |

## License

MIT OR Apache-2.0
