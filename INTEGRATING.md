# INTEGRATING.md — embedding `autosuggest-core` in a terminal

> Two supported paths: the **stdio daemon** (universal, any language) and the
> **C ABI** (native, in-process). Both speak the JSON shapes in `SCHEMA.md §4`.

## Path A — stdio daemon (recommended for non-Rust hosts)

Spawn the daemon once, keep its stdin/stdout pipes, and exchange one JSON object
per line.

```
$ autosuggest-daemon --specs ./specs --rules ./rules
```

Pseudocode host loop:
```text
proc = spawn("autosuggest-daemon", ["--specs", specs, "--rules", rules])
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

Link `libautosuggest_core.{dylib,so}` and include the generated header.

```c
#include "autosuggest_core.h"

AscEngine* eng = asc_engine_new("./specs", "./rules");

const char* req = "{\"v\":1,\"id\":1,\"op\":\"complete\","
                  "\"line\":\"git ch\",\"cursor\":6,\"cwd\":\"/repo\"}";
char* resp = asc_complete(eng, req);   // JSON string, caller frees
/* parse resp -> items[] ... */
asc_string_free(resp);

asc_engine_free(eng);
```

Notes:
- All strings are UTF-8. The engine copies what it needs; you free returned
  strings with `asc_string_free`.
- The boundary never panics (errors come back as a JSON `error` object).
- Thread-safety: create one `AscEngine` per thread, or guard with your own lock
  (v1 engine handle is not internally synchronized).

## History ownership

By default the **host owns history** and passes a window on `autosuggest`
requests (see `SCHEMA.md §3`). If you want the engine to persist history, build
with the `sqlite-store` feature and use the `history-store` API to record and
query entries; the daemon exposes `--history-db <path>` to enable it.

## ghostty-style recipe (sketch)

ghostty (Zig) links the C ABI: call `asc_engine_new` at startup, `asc_complete`
on input change to draw a suggestion overlay, `asc_correct` after a non-zero
exit to offer a fix line. Full worked example lands in M5.
