# INTEGRATING.md — embedding `autosuggest-core` in a terminal

> Two supported paths: the **stdio daemon** (universal, any language) and the
> **C ABI** (native, in-process). Both speak the JSON shapes in `SCHEMA.md §4`.

## Path A — stdio daemon (recommended for non-Rust hosts)

Spawn the daemon once, keep its stdin/stdout pipes, and exchange one JSON object
per line.

```
$ autosuggest-daemon ./specs ./rules
```

Pseudocode host loop:
```text
proc = spawn("autosuggest-daemon", [specs_dir, rules_dir])
on keystroke:
    write_line(proc.stdin, {"v":1,"id":n,"op":"complete",
                            "line":buf,"cursor":cur,"cwd":cwd})
    resp = read_line(proc.stdout)        # {"v":1,"id":n,"items":[...]}
    render_ghost_or_menu(resp.items)
on command failure(script, stderr, code):
    write_line(proc.stdin, {"v":1,"id":n,"op":"correct",
                            "script":script,"stderr":stderr,"exitCode":code,"cwd":cwd})
    resp = read_line(proc.stdout)
    if resp.items: offer(resp.items[0].insert)
```

Rules: ignore unknown fields; match responses by `id`; the daemon writes exactly
one response line per request line.

## Path B — C ABI (recommended for ghostty / native hosts)

Link `libautosuggest_ffi.{dylib,so,a}` and include the generated header
`autosuggest.h` (produced by `cargo build -p autosuggest-ffi`).

The C surface is intentionally tiny and **stateless from the caller's view**: a
single entry point handles every op by taking one JSON request string and
returning one JSON response string. Internally a process-wide engine is built
lazily on first call from the specs/rules directories, so there is no handle to
create, pass, or free.

```c
#include "autosuggest.h"

/* Point the lazily-built engine at your data BEFORE the first call.
 * Defaults are "./specs" and "./rules" relative to the process cwd. */
setenv("AUTOSUGGEST_SPECS_DIR", "/opt/autosuggest/specs", 1);
setenv("AUTOSUGGEST_RULES_DIR", "/opt/autosuggest/rules", 1);

const char* req = "{\"v\":1,\"id\":1,\"op\":\"complete\","
                  "\"line\":\"git ch\",\"cursor\":6,\"cwd\":\"/repo\"}";
char* resp = autosuggest_request_json(req);   /* JSON string, caller frees */
/* parse resp -> {"v":1,"id":1,"items":[...]} ... */
autosuggest_string_free(resp);
```

The same `autosuggest_request_json` serves `complete`, `autosuggest`, and
`correct` — switch by the request's `op` field, exactly as the stdio daemon
does. Match responses by `id`.

Notes:
- All strings are UTF-8. The library owns returned strings; free each with
  `autosuggest_string_free` (passing null is a no-op; double-free is UB).
- The boundary never panics and never returns null on the normal path. Null or
  non-UTF-8 input yields a valid JSON `error` response string.
- If the specs/rules directories fail to load, requests return a JSON `error`
  response rather than crashing, so a host can degrade gracefully.
- Specs/rules directories are read once when the engine is first built; set the
  environment variables before the first request.

## History ownership

By default the **host owns history** and passes a window on `autosuggest`
requests (see `SCHEMA.md §3`). If you want the engine to persist history, build
with the `sqlite-store` feature and use the `history-store` API to record and
query entries; the daemon exposes `--history-db <path>` to enable it.

## ghostty-style recipe (sketch)

ghostty (Zig) links the C ABI: set `AUTOSUGGEST_SPECS_DIR` /
`AUTOSUGGEST_RULES_DIR` at startup, then call `autosuggest_request_json` with an
`op:"complete"` request on input change to draw a suggestion overlay, and with an
`op:"correct"` request after a non-zero exit to offer a fix line. Free every
returned string with `autosuggest_string_free`.

