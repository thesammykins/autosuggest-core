# autosuggest-core

A lightweight, embeddable terminal completion & correction engine for any terminal — Ghostty, a shell plugin, a TUI, or an editor's integrated terminal.

## Why this exists

Terminal autocomplete is everywhere — Warp, Fish, Zsh plugins, VS Code's integrated terminal — but every host reimplements the same logic from scratch. `autosuggest-core` is a **drop-in brain** that any terminal host can embed to get:

- **Better suggestions** — ranked completions from an authored spec dataset covering 110+ commands (git, docker, kubectl, cargo, opencode, claude, etc.)
- **Smarter correction** — when you type `mkdir a/b` and get "No such file", it suggests `mkdir -p a/b`
- **History-aware ghost text** — predicts what you're about to type from recent commands, like Warp

The dataset is **human-authored** from `--help` and man pages, not copied from other projects. It's what makes the engine useful for real developers who actually type commands (as opposed to AI agents that generate them).

## Capabilities

- **As-you-type completion** — ranked suggestions for subcommands, options, option-arguments, and file paths
- **History autosuggestion** — predict the rest of the current command from recent history
- **Correction** — detect and suggest fixes for mistyped commands, flags, and paths

All three behaviours are a **pure function of context → suggestions**. The host owns the screen, keystrokes, and history storage; the engine owns the logic.

## Workspace

| Crate | Role |
|-------|------|
| `core` | Pure engine: types, tokenizer, parser, completer, ranker, history, correction |
| `protocol` | Serde models for the stdio JSON protocol |
| `data` | Spec & rule loading, indexing, generator executor |
| `daemon` | Binary: stdio JSON-lines server |
| `ffi` | cdylib: C ABI via cbindgen |
| `history-store` | Optional SQLite history persistence |

## Quick start

```shell
cargo build --release
cargo test
cargo bench
```

## Documentation

| Document | Content |
|----------|---------|
| [PRODUCT.md](./PRODUCT.md) | Product vision and the three capabilities |
| [TECH.md](./TECH.md) | Architecture, algorithms, integration surface |
| [SCHEMA.md](./SCHEMA.md) | Normative data and protocol formats |
| [INTEGRATING.md](./INTEGRATING.md) | How to embed the engine in a host |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Adding specs/rules, originality checks, development workflow |
| [COVERAGE.md](./COVERAGE.md) | Spec/rule coverage by command |

## Integration

`autosuggest-core` communicates over a simple **stdio JSON-lines protocol** (defined in `SCHEMA.md`). Start the daemon and pipe requests:

```shell
autosuggest-daemon ./specs ./rules
{"v":1,"id":1,"op":"complete","line":"git ch","cursor":6}
```

Or link the FFI library for in-process embedding.

## License

MIT OR Apache-2.0
