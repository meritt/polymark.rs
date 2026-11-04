# Architecture

## Scope

polymark converts text between plain text, Markdown, HTML, rich clipboard text and Telegram MarkdownV2 through one closed document model. `transform` reads `plain`, `markdown`, `html` or `rich` and writes any of them or `telegram`, returning the result, a plain-text twin for `rich`, and a count of every loss by kind. `clean` reduces pasted HTML to what the model holds. Output is deterministic and pinned by `testdata/cases.json`.

## Components

```text
                    polymark-core        pure functions; no I/O, no unsafe
                          │              pulldown-cmark for Markdown, html5ever for HTML
                    polymark-ffi         C ABI; the only unsafe code
                ┌─────────┴──────────┐
  polymark.wasm + polymark.js    PolymarkFFI.xcframework + Polymark (Swift)
```

Dependencies point down only. Bindings add no policy: they marshal arguments, map status codes and decode the report.

## Pipeline

```text
source ─► validate ─► parse ─► normalize ─► losses ─► write ─► report
            │           │          │                     │
            │           │          │                     └─ Markdown is parsed back and compared
            │           │          └─ one canonical form per mark
            │           └─ plain lines, CommonMark + GFM, browser-like HTML DOM; rich goes through clean
            └─ non-empty, no NUL, LF line breaks
```

- **Parse.** Markdown is CommonMark with GFM tables, task lists, strikethrough, footnotes and autolinks. HTML is built into a flat DOM as `template.innerHTML` would build it; styles, scripts, forms and hidden blocks are dropped.
- **Normalize.** Adjacent nodes merge, a mark never nests in itself, and spaces move outside its edges. Before Markdown, marks that Markdown cannot open or close in place are narrowed or removed.
- **Losses.** Everything the target cannot hold, and everything found in the source, is counted in `Warnings` by key.
- **Write.** Markdown is verified: the result is parsed again and compared with the normalized tree. On a mismatch it is rewritten with strict escaping, then without marks and with `unmarked`; if that still differs, the result is `representation`. Written Markdown therefore always reads back as the same document.

Character classes, `trim` and lengths follow JavaScript, so limits and results match what a browser computes.

## Bounds

| Limit | Value | On excess |
|---|---|---|
| `LIMIT` | 30 000 UTF-16 units of input | `length` |
| output | 5 × `LIMIT` UTF-16 units | `output` |
| model | 5000 nodes, 16 levels | `structure` |
| HTML input | 2000 tags, 5000 nodes | `htmlLimit` |
| `clean` | 2 000 000 UTF-16 units, 50 000 tags, 100 000 nodes | `length`, `htmlLimit` |
| Telegram | 4096 visible characters | `length` |

## C ABI

- Text is UTF-8 in memory from `polymark_alloc`, released with `polymark_dealloc`.
- `polymark_transform` and `polymark_clean` fill a `polymark_report`: a status and, on `POLYMARK_OK`, JSON released with `polymark_report_free`. The JSON is the same on every platform: `{ result, plain?, warnings }` or `{ error }`. The core writes it without serde.
- The status reports a call error only: input that is not UTF-8 or a format index out of range. A refused conversion is `{ error }` inside the report.
- Calls keep no state between them and run on any thread.
- A panic never crosses the boundary. Native builds return `POLYMARK_INTERNAL`; in wasm32 a panic traps the instance.
- `polymark_version` identifies the build. Bindings refuse a library whose version differs from their own.
- cbindgen generates `include/polymark.h` from `polymark-ffi`; Miri runs the ABI tests.

## Bindings

**WebAssembly.** `polymark.wasm` is the `cdylib` of `polymark-ffi` with no generated glue, optimized for size with `wasm-opt -Oz`: it is downloaded before the first conversion, which takes milliseconds at any level. `polymark.js` is one ES module without static imports, so it runs in a page, a worker and Node; it reads a file only for a `file:` URL. A lone surrogate cannot be written as UTF-8, so `transform` and `clean` answer `{ error: 'unicode' }` before the call. Results are read from memory only after the call, which may have grown it.

**Swift.** `PolymarkFFI.xcframework` holds one universal static library for `arm64` and `x86_64` with the header as module `PolymarkFFI` under `Headers/PolymarkFFI/`, which keeps its module map distinct from other Rust libraries in the same app. `Package.swift` uses a local build when present, otherwise the release binary pinned by URL and checksum. `Polymark` decodes the same JSON and throws a typed `PolymarkError` with the same codes.

## Conformance

`testdata/cases.json` lists conversions and cleanings with the exact report each must produce: every `from` → `to` pair, warnings, tables, code boundaries, attributes, images and every refusal. The Rust, WebAssembly and Swift test suites all replay it.

## Distribution

| Artifact | Consumer |
|---|---|
| `polymark-core` crate | crates.io |
| `polymark-vX.Y.Z-npm.tgz`, package `@meritt/polymark-wasm` | npmjs.com |
| `polymark-vX.Y.Z-xcframework.zip` | SwiftPM, through `Package.swift`, macOS 26 |

The release set is reproducible: the tarball and the zip have fixed entry order, times and modes. The checksum pinned in `Package.swift` before tagging is therefore the checksum of the tagged build.
