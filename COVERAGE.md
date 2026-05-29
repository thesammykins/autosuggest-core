# COVERAGE.md — command + correction coverage for v1

This document maps the shipped data set to the coverage targets in
`PRODUCT.md §7` and the mechanisms in `SCHEMA.md`. It is the source of truth for
"what does v1 actually cover". Counts are mechanical: one `*.spec.json` per
command, one `*.rule.json` per correction rule (native rules included).

- **Specs:** 64 (`specs/*.spec.json`)
- **Correction rules:** 15 (`rules/*.rule.json`; 2 native, 13 JSON)

## 1. Command specs by domain (PRODUCT.md §7 + 2026 additions)

| Domain | Shipped specs |
| --- | --- |
| Filesystem/core | `ls cd mkdir rmdir rm cp mv cat less head tail touch ln pwd find chmod chown du df stat tree` (22) |
| Text/search | `grep rg sed awk sort uniq wc cut tr xargs` (10) |
| Process/system | `ps kill top killall env export which` (7) |
| Network | `curl wget ssh scp ping` (5) |
| Archives | `tar gzip gunzip zip unzip` (5) |
| Dev/VCS | `git cargo npm docker make brew` plus `gh jq uv go` (10) |
| Cloud/DevOps | `kubectl aws terraform` (3) |
| Package mgmt | `apt` (1) |
| Editors/misc | `man echo` (2) |

Total: 64 specs. `git` is authored deep (25+ subcommands); `cargo`, `npm`,
`docker`, `gh`, `kubectl`, and `terraform` carry rich subcommand trees.

## 2. Completion mechanisms (SCHEMA.md §1) and their golden proof

Every distinct completion mechanism is exercised by at least one golden fixture
under `tests/fixtures/complete/` (asserted by `crates/core/tests/golden_complete.rs`).

| Mechanism | Where used (examples) | Golden fixture |
| --- | --- | --- |
| Subcommand completion | `git`, `cargo`, `npm`, `docker`, `kubectl`, `gh`, `uv`, `go` | `complete/git`, `complete/apt` |
| Option name filtering | most specs | `complete/ls`, `complete/grep`, `complete/cat`, `complete/mkdir`, `complete/cp`, `complete/mv`, `complete/rm`, `complete/apt` |
| `suggestions` (static arg values) | `kill -s`, `cd -`, `kubectl get` resource types, … | `complete/kill_suggestions`, `complete/cd` |
| `requiresSeparator` (`--opt=` insert form) | `ls --color`, `grep --color` | `complete/ls_separator` |
| `dependsOn` (offer only after dep present) | `ls -h` depends on `-l` | `complete/ls_depends` |
| `exclusiveOn` (hide conflicting options) | `cp -i` vs `-f`/`-n`, `sort`, `rm`, … | `complete/cp_exclusive` |
| Argument generators (dynamic exec) | `make` target; `docker exec/stop/logs`; `brew uninstall/upgrade`; `npm run`; `gh pr list` | proven dynamically — see §4 |

`isDangerous` (a `Suggestion`/item attribute per `SCHEMA.md §1.4`) is surfaced
through the protocol `dangerous` field and the wire mapping in
`golden_complete.rs`.

## 3. Correction rules (SCHEMA.md §2) and their golden proof

Every distinct correction mechanism is exercised by a golden under
`tests/fixtures/correct/` (asserted by `crates/core/tests/golden_correct.rs`).

| Requirement | Rule(s) | Mechanism | Golden fixture |
| --- | --- | --- | --- |
| `no_command` (PATH edit distance) | `no_command` (native) | nearest `$PATH` command, distance ≤ 2 | `correct/no_command` |
| Subcommand typos (git/cargo/npm/docker/kubectl/apt/… any spec) | `subcommand_typo` (native) | spec-driven nearest subcommand | `correct/subcommand_typo`, `correct/apt_install_typo` |
| `mkdir -p` | `mkdir_p` | InsertFlag `-p` | `correct/mkdir_p` |
| `cd` file→dir | `cd_not_dir` | RegexReplace to parent dir | `correct/cd_not_dir` |
| `cd` typo | `subcommand_typo` / `no_command` | nearest-name | covered via native rules above |
| `sudo` on permission denied | `sudo` | Prefix `sudo ` | `correct/sudo_prefix` |
| `cp`/`mv -r` on directory | `cp_dir`, `mv_dir` | InsertFlag `-r` | `cp_dir` exercised by unit case table; `mv_dir` via id uniqueness |
| `rm -r` | `rm_dir` | InsertFlag `-r` | exercised by unit case table |
| `ssh`/`scp` flag fixes | `ssh_port_colon`, `scp_dir` | RegexReplace, InsertFlag `-r` | exercised by unit case table |
| `grep -r` | `grep_r` | InsertFlag `-r` | exercised by unit case table |
| `tar` flag fixes | `tar_gz` | scriptRegex-gated InsertFlag `-z` | `correct/tar_gz` |
| `brew` install typos | `brew_cask` | InsertFlag `--cask` | `correct/brew_cask` |
| `apt` install / subcommand typos | `subcommand_typo` (native) | spec-driven | `correct/apt_install_typo` |
| `pip` subcommand typos | `pip_install_typo` | RegexReplace | `correct/pip_install_typo` |
| `docker-compose` → `docker compose` | `docker_compose_deprecated` | RegexReplace | `correct/docker_compose_deprecated` |

Every rule in `rules/` is additionally validated by
`crates/core/tests/specs`-adjacent unit tests:
`shipped_rules_all_parse_with_unique_ids` (all parse, ids unique) and
`shipped_rules_case_table` (each rule fires on a representative case).

## 4. Generators (SCHEMA.md §1.5) — dynamic-output proof

`core` performs no I/O; generators run only through an injected
`GeneratorRunner`. The daemon wires the sandboxed runner
(`SandboxedRunner`, `DEFAULT_ALLOW_LIST`), and failures degrade silently and
never crash the daemon.

Because the pure `complete_line` path used by `golden_complete.rs` does not run
generators, dynamic output is proven separately by an end-to-end daemon test,
`daemon_runs_argument_generator_with_echo` in `crates/daemon/tests/stdio.rs`:
it drives a generator backed by the allow-listed, deterministic `echo` program
and asserts both full output and query filtering. This avoids depending on `git`
or any external state being installed.

Generators ship only on **reachable** argument positions (top-level args of a
no-subcommand command, or subcommand-leaf args), and only on commands already on
`DEFAULT_ALLOW_LIST`:

| Spec | Position | Generator (allow-listed program) |
| --- | --- | --- |
| `make` | target arg | `make` |
| `docker` | `exec`/`stop`/`logs` container arg | `docker` |
| `brew` | `uninstall`/`upgrade` formula arg | `brew` |
| `npm` | `run` script arg | `npm` |

Specs with generator-backed arg suggestions available via the spec tree
but not wired to `DEFAULT_ALLOW_LIST` (host adds as needed):
`git`, `gh` (PR/issue/release listing).

## 5. History persistence (optional)

The daemon supports an optional SQLite-backed history store via
`--history-db <path>` (feature flag `sqlite-store`). When enabled, the store
provides history context for the `autosuggest` op when the host does not supply
a history window in the request. The history-store crate
(`crates/history-store/`) is built on `rusqlite` with a bundled SQLite.

| Crate | Tests | Description |
| --- | --- | --- |
| `autosuggest-history-store` | 5 unit + 1 doc | Record, query (prefix + cwd filter), clear, limit |

## 6. Test inventory

| Suite | File | Asserts |
| --- | --- | --- |
| Spec validation | `crates/core/tests/specs.rs` | all 66 specs parse + validate |
| Completion goldens | `crates/core/tests/golden_complete.rs` | 14 fixtures, exact ranked output |
| Autosuggest goldens | `crates/core/tests/autosuggest.rs` | history suggestion fixtures |
| Correction goldens | `crates/core/tests/golden_correct.rs` | 9 fixtures, exact ranked output |
| Daemon stdio | `crates/daemon/tests/stdio.rs` | all ops, malformed input, live generator |
| FFI ABI | `crates/ffi/tests/ffi.rs` | C ABI round-trips all ops + recovery |
| History store | `crates/history-store/tests` | record, query, cwd filter, clear, empty |

Refresh goldens by running any golden suite with `ASC_DUMP_GOLDEN=1` to print
produced output instead of asserting.
