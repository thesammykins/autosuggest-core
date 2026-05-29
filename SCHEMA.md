# SCHEMA.md — normative data & protocol formats

> This document is **normative**. All crates, specs, rules, and adapters MUST
> conform. Changes here require an orchestrator-approved version bump.
> Schema version: **1**.

These formats are our own. They are *inspired by* the general shape of
declarative completion systems (ideas/structure, which are not copyrightable)
but the field set, semantics, and all content are authored for this project.

---

## 1. Completion spec (`specs/*.spec.json`)

A spec describes one top-level command. UTF-8 JSON.

### 1.1 Subcommand object (also the root)

```jsonc
{
  "name": ["git"],                  // string or array (first = canonical, rest = aliases)
  "description": "Distributed VCS", // optional, <= 120 chars
  "subcommands": [ <Subcommand>, ... ],   // optional
  "options":     [ <Option>, ... ],       // optional
  "args":        [ <Arg>, ... ],          // optional, positional, in order
  "requiresSubcommand": false,      // optional, default false
  "parserDirectives": <ParserDirectives>  // optional
}
```

### 1.2 Option object

```jsonc
{
  "name": ["-a", "--all"],          // string or array of forms
  "description": "Include all",     // optional
  "args": [ <Arg>, ... ],           // optional; presence => option takes a value
  "isRequired": false,              // optional, default false
  "isRepeatable": false,            // optional; or an integer max count
  "isPersistent": false,            // optional; applies to all descendant subcommands
  "requiresSeparator": false,       // optional; true => must be --opt=value
  "exclusiveOn": ["--quiet"],       // optional; mutually exclusive option names
  "dependsOn": ["-l"]               // optional; requires these options present
}
```

### 1.3 Arg object

```jsonc
{
  "name": "path",                   // optional label (for display only)
  "description": "...",             // optional
  "isOptional": false,              // optional, default false
  "isVariadic": false,              // optional; consumes remaining tokens
  "template": "filepaths",          // optional: "filepaths"|"folders"|"history"
  "suggestions": [ <Suggestion>|string, ... ], // optional static suggestions
  "generator": <Generator>,         // optional dynamic source
  "isCommand": false                // optional; arg is itself a command (e.g. `sudo`, `xargs`)
}
```

### 1.4 Suggestion object

```jsonc
{
  "name": ["checkout"],             // string or array; required
  "displayName": "checkout",        // optional; defaults to name[0]
  "insertValue": "checkout ",       // optional; supports "{cursor}" marker; defaults to name[0]
  "description": "Switch branches", // optional
  "priority": 75,                   // optional 0..=100, default 50
  "isDangerous": false,             // optional; host may warn (e.g. rm -rf)
  "hidden": false,                  // optional; excluded unless explicitly typed
  "deprecated": false               // optional
}
```
A bare string is shorthand for `{ "name": [s] }`.

### 1.5 Generator object

```jsonc
{
  "run": ["git", "branch", "--format=%(refname:short)"],  // argv; run[0] MUST be allow-listed
  "splitOn": "\n",                  // optional; default "\n"
  "trim": true,                     // optional; default true
  "extract": "^(\\S+)",             // optional regex; capture group 1 = suggestion
  "priority": 60,                   // optional; applied to produced suggestions
  "cache": { "ttlMs": 3000 }        // optional; default ttlMs 0 (no cache)
}
```

### 1.6 ParserDirectives object

```jsonc
{
  "flagsArePosixNoncompliant": false,   // true => short flags may chain: -lah
  "optionsMustPrecedeArguments": false, // true => options invalid after first arg
  "optionArgSeparators": ["=", " "]     // accepted separators for option args
}
```

---

## 2. Correction rule (`rules/*.rule.json`)

```jsonc
{
  "id": "mkdir_p",                  // unique id; required
  "description": "Add -p when parent dir missing",
  "priority": 90,                   // optional 0..=100, default 50
  "match": {                        // ALL present conditions must hold (AND)
    "scriptStartsWith": "mkdir ",   // optional
    "scriptRegex": "^mkdir ",       // optional
    "stderrContains": ["No such file or directory"], // optional (any-of)
    "stderrRegex": "...",           // optional
    "exitCodeIn": [1],              // optional
    "commandExists": "mkdir"        // optional; require base cmd on PATH
  },
  "rewrite": {                      // exactly one strategy
    "insertFlag": { "after": "mkdir", "flag": "-p" }
    // | "replaceToken": { "index": 0, "with": "ls" }
    // | "prefix": "sudo "
    // | "swapSubcommand": { "from": "comit", "to": "commit" }
    // | "regexReplace": { "pattern": "...", "with": "..." }
  }
}
```

### 2.1 Native predicate rules
Some rules require logic beyond JSON and are registered in Rust by `id`:
- `no_command` — base command not on `$PATH`; suggest nearest PATH entry
  (Levenshtein ≤ 2), ranked by distance then frequency.
- `subcommand_typo` — unknown subcommand; suggest nearest known subcommand from
  the matching spec.
These appear in `rules/` as `{ "id": "...", "native": true, "priority": N }`
entries so ordering/priority stay data-driven; the implementation lives in code.

---

## 3. History window (input to autosuggest)

The host passes recent commands, most-recent-first preferred:
```jsonc
{ "entries": [
    { "command": "git push origin main", "cwd": "/repo", "exitCode": 0, "ts": 1730000000 },
    ...
] }
```
Only `command` is required per entry; `cwd`/`exitCode`/`ts` enable weighting.

---

## 4. stdio protocol (JSON lines, version 1)

One JSON object per line, request and response. Envelope:

### 4.1 Requests
```jsonc
// complete
{ "v": 1, "id": 7, "op": "complete",
  "line": "git ch", "cursor": 6, "cwd": "/repo", "env": { "SHELL": "zsh" } }

// autosuggest
{ "v": 1, "id": 8, "op": "autosuggest",
  "prefix": "git pu", "cwd": "/repo", "history": <HistoryWindow> }

// correct
{ "v": 1, "id": 9, "op": "correct",
  "script": "mkdir a/b", "stderr": "mkdir: a: No such file or directory",
  "exitCode": 1, "cwd": "/repo" }
```
`cursor` is a byte offset into `line`. If omitted, defaults to end of `line`.

### 4.2 Responses
```jsonc
// complete / correct
{ "v": 1, "id": 7, "items": [
    { "insert": "checkout ", "display": "checkout",
      "desc": "Switch branches", "score": 0.97,
      "dangerous": false, "deprecated": false } ,
    ...
] }

// autosuggest (0 or 1 item)
{ "v": 1, "id": 8, "suggestion": "git push origin main" }   // or null

// error
{ "v": 1, "id": 9, "error": { "code": "bad_request", "message": "..." } }
```

### 4.3 Rules
- Unknown fields MUST be ignored (forward-compat).
- `id` echoes the request; hosts may pipeline by `id`.
- Engine MUST NOT emit anything except one response object per request line.
- `score` is 0..=1 float, already sorted descending in `items`.

---

## 5. Versioning

- This schema is version **1**. Additive fields do not bump the version.
- Breaking changes bump `v` and require an orchestrator-approved migration note
  appended to this file.
