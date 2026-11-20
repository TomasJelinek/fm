# CSS and Makefile colours, PDF thumbnails and decoder reasons — 20 November 2026

Three gaps from the viewer acceptance record
([2026-11-20-acceptance-acc-viewers.md](2026-11-20-acceptance-acc-viewers.md)) and the
viewer speed record ([2026-11-20-viewer-speed.md](2026-11-20-viewer-speed.md)):

- **V7:** `code/style.css` and `code/Makefile` opened as plain text.
- **V8:** PDFs showed placeholder thumbnails; no thumbnailer handled them.
- **V4 follow-up:** a damaged image said only "This image is damaged or incomplete,
  or its format is not supported." The image worker protocol's `Failure` carried no
  text, so the decoder's own reason could not be shown.

## Changes

### CSS and Makefile profiles (source viewer)

`plugins/midnight-viewer-source` has two new bounded streaming lexers
(`syntax_css.rs`, `syntax_make.rs`), so it now has 23 lexical profiles.

- **CSS:** selectors (function colour), properties including `--custom` (type),
  at-rules and `!important` (keyword), comments, strings and unquoted `url(...)`,
  numbers with units, hex colours (number) and function names. A depth counter and a
  64-level bitmask tell rule blocks from declaration blocks, so `@media` holds
  rules, `@font-face` holds declarations and `@keyframes` holds keyframe selectors.
- **Makefile:** targets (function colour) and special targets such as `.PHONY`
  (keyword), assigned variables and `$(...)`, `${...}`, `$x` and automatic
  references (type), make functions inside references (function), directives
  (keyword), comments, recipe lines (string, with `@`/`-`/`+` prefixes as keywords),
  and `define` bodies.
- **Routing:** `.css`, `.mk` and `.mak` are extensions. `Makefile` and `GNUmakefile`
  have none, so `ViewerAssociations::suffix_match` now also matches an entry equal
  to the whole file name; the manifest lists `makefile` and `gnumakefile`. A side
  effect: a file named exactly like an entry, such as `MANIFEST` or `png`, now goes
  to that viewer instead of Basic.

### PDF first-page thumbnails (image thumbnailer)

`plugins/midnight-thumbnail-image` now draws page 1 of a PDF. It uses `hayro` 0.9.0
with the PDF viewer's features and rendering code (white page, the same refusal of
pages with renderer warnings). No new crate, version or license entered the project.
A new plugin would have touched build, notice and CI scripts, inventory scopes and
the About lists; every request already runs in its own sandboxed one-shot worker.

- The worker sniffs `%PDF-` in the first 1024 bytes; file names never reach it.
- It reads at most 64 MiB, once, and fits the page into the requested cell.
- Password-protected, damaged and unsupported documents fail, and the host keeps
  the placeholder. The host's existing limits are unchanged: 512 MiB, 8 CPU seconds,
  10 s and 256 MiB of reads per request.
- The manifest lists `pdf`, and its version names Hayro, so cached image
  thumbnails are regenerated once.
- The dependency records (`docs/dependencies/*`, [THIRD_PARTY.md](../THIRD_PARTY.md)),
  the plugin's notices and the About list name the image thumbnails beside the PDF
  viewer.

### Failure reasons (image viewer API 0.3.0, SDK 0.4.0)

`FailureReason` (tag 10) may follow `Failure` as a worker's last message. It holds
1–160 printable ASCII characters without `/` or `\`. SDK 0.4.0 sanitizes the
provider's error text into that form and sends it. The host waits after `Failure`
for the reason or the end of the stream: at most one second, under the session's
cancellation. It then stops the worker and shows "This image is damaged or
incomplete: <reason>", or the matching document, image and page sentences.

Compatibility needs no negotiation:

- Hosts before API 0.3.0 stop reading at `Failure`.
- Workers before SDK 0.4.0 send no reason, and the host keeps the plain sentence.
- A malformed trailer is ignored.

The wait matters: Rust's line-buffered stdout splits the SDK's write at the tag
byte 0x0A, so stopping the worker right after `Failure` could lose the reason. The
details are in the [image viewer API](../developer/IMAGE_VIEWER_API.md#failure-reasons).

Workers and plugins:

- The shared image decoder now reports "The file is empty" and "The content is not a
  recognized image format" itself, before trying SVG. A cut PNG reports
  "unexpected end of file".
- The PDF viewer names empty, invalid, encrypted, oversized and undrawable
  documents and pages in short, path-free texts.
- The image viewer (0.3.0) and PDF viewer (0.2.0) pin SDK 0.4.0.

## Tests

All commands ran in the project's build image with the workspace mounted read-only.
Plugin and host checks used private target directories; the worker SDK bundle was
packaged privately with `worker-packages`. Its unchanged packages kept their
checksums. API 0.3.0 is `130fc9ed…1e14cad`, SDK 0.4.0 `baf0111a…c11274c`.

| Check | Result |
| --- | --- |
| Source viewer `scripts/ci.sh` (format, tests, all-target Clippy `-D warnings`, debug package) | Passed; 153 passed, 2 ignored. Includes split-invariance, bounded-suffix, cancellation and random-input tests for both lexers, and the public plugin with `Makefile`, `GNUmakefile`, `rules.mk`, `rules.MAK` and `style.css`. Rerun independently: 153 passed. |
| `cargo test -p midnight-core --lib associations` and `-p midnight-viewer-engine --lib routing` | 6 and 12 passed, including whole-name matching. Clippy on midnight-core clean. |
| Image thumbnailer `scripts/ci.sh` (format, tests, Clippy, release build) | Passed; 18 tests, including 8 PDF tests: page 1 only, bounds, white page with text, password-protected, truncated and unsupported documents. |
| `thumbnail-notices.py`, `test_thumbnail_notices.py`, `test_default_thumbnailers.py`, `test_dependency_inventory.py`; `cargo test -p midnight-ui --lib about_licenses` | Notices staged with Hayro's, the fonts', CMaps' and `alloc-stdlib`'s; 17, 15 and 12 passed; 6 passed. |
| Image viewer API/SDK: `cargo test` and all-target Clippy in `image-viewer-api` | 16 passed: the wire form and its bounds, sanitizing, Failure-then-reason on decode, metadata, render and page errors. Clippy clean. |
| Image viewer `scripts/ci.sh` (shared decoder and plugin; format, tests, Clippy, debug package) | Passed. Decoder 13 + 3 + 9 (including empty and non-image content); plugin 18, including the reason texts. |
| PDF viewer `scripts/ci.sh` (format, tests, Clippy, debug package) | Passed; 7, including each rejection's text. |
| `cargo test -p midnight-image-viewer-host --lib --tests` and all-target Clippy | 37 passed, 4 ignored: trailer parsing, oversized or invalid trailers ignored before reading a body, old workers without a reason, and the composed texts. Clippy clean. |
| Host native tests (`--ignored`) against the new image and PDF packages | 3 of 3 and 3 of 3, three runs in a row. Text gives "…: the content is not a recognized image format", an empty file "…: the file is empty", and a cut PNG header gives a reason. Garbage gives "This document is damaged or incomplete: the PDF structure is invalid or incomplete". An undecodable inline image gives "This page could not be drawn: the page contains content this viewer cannot draw". |
| The same native tests against the previously installed SDK 0.3.0 workers | The new host keeps the plain sentences ("This image/document is damaged or incomplete, or its format is not supported."), with no protocol error, in 0.10–0.12 s. The other image and PDF tests pass, each run on its own. The two reason tests fail there, as expected. |

## Acceptance (smoke)

Command: `cargo test --workspace --locked --features midnight-acceptance/acceptance
--test acceptance --no-run` (debug, per-package `codegen-units` overrides only), then
`acceptance-<hash> --scale smoke viewers_files viewers_thumbnails`. The plugin folder
(`MIDNIGHT_ACCEPTANCE_PLUGINS`) held:

- the new source, image and PDF viewer packages;
- the installed basic and Markdown viewers;
- `native.linux` and the archive worker;
- the installed thumbnailers, with the new image thumbnailer and its manifest.

The HTML viewer was left out: it cannot start inside the build image.

Runs `20261120-081856` and `20261120-083430` (after removing the unused `Expect::Bad`;
the build had no warnings) gave the same results. `cargo clippy --workspace --locked
--features midnight-acceptance/acceptance --test acceptance -- -D warnings` passed
(exit 0).

| Scenario | Result |
| --- | --- |
| `viewers_files` | 40 of 42 checks passed. **Failed:** the two HTML pages ("not shown by the HTML viewer"), because no HTML viewer was installed. `code/style.css` and `code/Makefile` opened in Source mode with syntax colours (65 and 64 ms): selectors, properties and colours in CSS; the target, `@` and recipe in the Makefile. `images/truncated.png` showed "This image is damaged or incomplete: unexpected end of file" and `images/zero.png` "This image is damaged or incomplete: the file is empty" (80 ms each). |
| `viewers_thumbnails` | Passed. "Thumbnails in docs/: report.pdf"; the tile is a white page of 96 × 64 px with 67 dark text pixels. Every image format still showed its thumbnail. |

## Not verified

- **HTML pages:** not run in the build image, so `docs/page.html` and
  `docs/plain.htm` stay unchecked here.
- **Platforms and full scale:** Windows builds and the Windows thumbnail build
  scripts, the release profile and full-scale acceptance were not run.
- **Real documents:** the PDF thumbnail render times on large real documents, and
  the thumbnailer's 8 CPU-second limit with the debug build on slow pages, were not
  measured.
- **Inventory:** it was refreshed only for the image thumbnailer's scope, not
  regenerated in full.
