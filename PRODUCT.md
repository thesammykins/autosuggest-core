# PRODUCT.md — `autosuggest-core`

> A lightweight, embeddable terminal completion & correction engine.
> Status: **spec-approved, implementation starting**. Owner: orchestrator.

## 1. Vision

`autosuggest-core` gives **any** terminal — [ghostty](https://github.com/ghostty-org/ghostty),
a shell plugin, a TUI, an editor's integrated terminal — the three completion
behaviours that make modern terminals (notably Warp) feel intelligent, while
remaining small, dependency-light, and free of any third-party completion data.

The engine is a **pure function of context → suggestions**. The host owns the
screen, the keystrokes, and the history storage; the engine owns the logic. This
separation is what makes it pluggable.

## 2. The three capabilities (all in v1)

### 2.1 As-you-type completion
While the user types, return ranked suggestions for the next token:
subcommands, options/flags, option-arguments, and file paths.

```
$ git ch▏            → checkout, cherry-pick, ...
$ ls -l▏             → -a/--all, -h, -t, ...
$ cd src/▏           → src/core/, src/data/, ...
```

### 2.2 History autosuggestion
Given the current input prefix and a window of recent commands, return the single
best "ghost text" continuation (fish / zsh-autosuggestions / atuin style).

```
$ git pu▏            → git push origin main   (most-recent matching history entry)
```

### 2.3 Failed-command correction
After a command fails, given the script, its stderr, and exit code, propose a
ranked list of corrected commands.

```
$ mkdir foo/bar      → mkdir: foo: No such file or directory   →  mkdir -p foo/bar
$ sl                 → command not found: sl                   →  ls
$ git comit -m x      → git: 'comit' is not a git command       →  git commit -m x
$ apt install x       → permission denied                      →  sudo apt install x
```

## 3. Non-goals (v1)

- **Not** a shell, renderer, or input editor — no UI, no keybindings.
- **Not** an AI/LLM feature — purely deterministic rules + data.
- **No** network calls, no telemetry, no background services.
- **Not** a 1:1 port of Fig/Warp's ~3,000 specs. We ship a curated, **original**
  dataset and an open schema others can extend. We may study the *shape* of
  prior art (ideas/schemas are not copyrightable) but we author our own content.

## 4. Users & integration story

| User | How they consume it |
|------|---------------------|
| Native terminal (ghostty, Zig/C/Swift) | Link the `cdylib` via the **C ABI**, call in-process. |
| Shell plugin / scripting host | Spawn the **stdio daemon**, exchange JSON lines. |
| Rust application | Depend on the `autosuggest-core` crate directly. |

A new terminal should integrate the stdio path in **< 50 lines**. See
`INTEGRATING.md`.

## 5. Product requirements

### 5.1 Functional
- FR1 — `complete(line, cursor, cwd, env)` returns ranked completion items.
- FR2 — `autosuggest(prefix, history_window)` returns 0 or 1 ghost-text item.
- FR3 — `correct(script, stderr, exit_code, cwd)` returns ranked corrected commands.
- FR4 — Completion walks a declarative spec tree (subcommands → options → args).
- FR5 — Args may resolve via templates (`filepaths`/`folders`/`history`) or
  declarative **generators** (run an allow-listed command, parse output).
- FR6 — Correction supports declarative JSON rules + a small set of native
  predicates (PATH "did you mean", subcommand-typo, `-p`, `sudo`, `cd` typo).
- FR7 — History store is **optional & host-owned by default**; engine also ships
  an opt-in SQLite-backed store module.

### 5.2 Non-functional
- NFR1 — Static completion latency **< 5 ms**; generator-backed **< 15 ms** warm.
- NFR2 — No mandatory runtime services; one linked lib or one spawned process.
- NFR3 — Generators run only allow-listed binaries, with a timeout and cache TTL;
  **never** eval arbitrary shell strings.
- NFR4 — Deterministic & side-effect-free except explicit generator execution.
- NFR5 — Cross-platform: macOS + Linux for v1 (Windows best-effort, not gated).
- NFR6 — No copied third-party completion data; license clean (see CONTRIBUTING).

## 6. Success criteria (v1 "done")

- All three capabilities pass golden tests across the seed command set (§7).
- `daemon` binary + `cdylib` + C header build and pass an end-to-end demo.
- The `mkdir`, `sl`/`ls`, `git comit`, and `sudo` correction examples all pass.
- Latency budgets (NFR1) met in `criterion` benches.
- A documented ghostty-style integration recipe exists and runs.

## 7. Initial command coverage (comprehensive from the start)

Authored specs grouped by domain (target ~45–55 specs for v1):

- **Filesystem/core:** `ls cd mkdir rmdir rm cp mv cat less head tail touch ln
  pwd find chmod chown du df stat tree`
- **Text/search:** `grep rg sed awk sort uniq wc cut tr xargs`
- **Process/system:** `ps kill top kill killall env export which`
- **Network:** `curl wget ssh scp ping`
- **Archives:** `tar gzip gunzip zip unzip`
- **Dev/VCS:** `git` (deep: 25+ subcommands), `cargo`, `npm`, `docker` (core),
  `make`, `brew`
- **Editors/misc:** `man echo`

Correction rules cover at minimum: `no_command` (PATH edit distance),
git/cargo/npm/docker subcommand typos, `mkdir -p`, `cd` file→dir + typo, `sudo`
on permission denied, `cp/mv -r` on directory, `rm -r`, `ssh`/`scp` flag fixes,
`grep -r`, `tar` flag fixes, and `apt`/`brew` install typos.

See `ROADMAP.md` for milestone sequencing and `SCHEMA.md` for data formats.
