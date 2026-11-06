# Midnight Markdown viewer 0.1.0

Independent `cdylib` package `midnight.viewer.markdown`, routing `.md`,
`.markdown`, `.mdown` and `.mkd` before ordinary source/text viewers. It emits
public `document-blocks.v1` models; the host supplies all window, theme,
selection and font rendering. Explicit Text mode returns raw decoded text.

CommonMark paragraphs, six heading levels, emphasis, strong text, code spans,
fenced/indented code, quotations, list items, rules, entities, escaped text,
links and image alternative text are represented semantically. HTML is literal
inert text. Images are alt text only. No HTML renderer, browser, JavaScript,
network request, image/font/style fetch or automatic link launch exists.
Metadata permits only HTTP, HTTPS, mailto and local fragments, with no controls,
whitespace or backslashes. The host must separately check any explicit action.
Tables, task checkboxes, footnotes and other optional Markdown extensions are
not enabled. Reference links resolve within the bounded parse window.

## Correctness and limits

UTF-8 and BOM UTF-16 LE/BE decode with exact source-byte anchors, including
malformed units and CR/LF/CRLF normalization. Unsafe controls become visible
characters. Offsets are checked u64 values. Removed Markdown markers mean a
displayed boundary can have more than one source interpretation; the model
retains one actual boundary, never an interpolated offset. Entity/code
normalization exposes exact event endpoints where interior boundaries cannot
be established. Copy uses displayed text, not source bytes including markers.
Uncached seeks look behind by four bytes after UTF-16 word alignment, then
select actual decoded boundaries. Backward pages select a displayed suffix
ending at or before the original source anchor. Text-mode search preserves
visible Markdown markers, just as the Text-mode viewport does.

Viewport reconstruction starts at a known complete top-level block boundary,
keeps at most 256 checkpoints and rescans at most 32 KiB before its anchor.
Known fence/quote context therefore survives nearby random or sequential
viewports. Distant uncached regions and parser-budget exhaustion use a plain
paragraph fallback. Large unsupported contexts show their source markers
literally instead of guessing a grammar state.
If invalid/control text expands beyond the decoded context budget, viewport
and search reuse a nearby fragment of the same bounded read and discard the
unproven parser context rather than skipping the requested content.

At most 64 KiB is read/decoded per operation. Emitted displayed text is at most
8192 bytes, further limited by requested lines. Parser traversal stops at
16384 events, nesting depth 16 or 256 blocks and falls back to plain text.
The combined wire-item budget is checked too: dense documents first discard
optional styles and link metadata, preserving displayed text and exact anchors
so their visible content and search remain consistent.
Only bounded input is handed to the upstream parser, whose individual parser
step has no cancellation API; cancellation is checked before construction,
between events, while decoding, and between SDK asynchronous reads. Four
active operations and 16 sessions bound plugin scratch/model memory below
16 MiB, in addition to the SDK's separately bounded queues.

Search uses the same semantic reconstruction as the viewport and searches
displayed text with source anchors. Streaming KMP retains only a query-sized
window; matches can cross reads. Insensitive search compares Unicode lowercase
expansions (not locale collation or full case folding). Backward search scans
from the beginning, retaining only the latest match before the anchor. It is
cancellable and has no whole-file size cap. Progress is at most four times a
second. Close drops plugin state; the host owns closing the content lease.

## Independent build, CI and installation

This directory is its own Cargo workspace and lockfile. Export it without
application sources, install Rust 1.99.0 or newer, and supply the released
public SDK `.cargo/config.toml` plus its local registry directory.
`prepare-sdk` writes a dependency config under BuildRoot, retaining the SDK
registry and using the normal Cargo cache for plugin-specific dependencies.
Populate that cache with `cargo fetch --locked --config GENERATED_CONFIG`
before the offline commands when necessary.

Windows PowerShell:

```powershell
./scripts/ci.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/markdown/_build
./scripts/build.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/markdown/_build -OutputDirectory C:/app/viewers/markdown
```

Linux (Python 3.11+ is used only to read the released SDK config):

```sh
bash scripts/ci.sh /sdk/.cargo/config.toml /qa/markdown/_build
bash scripts/build.sh /sdk/.cargo/config.toml /qa/markdown/_build /app/viewers/markdown
```

CI runs formatting, all tests, strict Clippy and native release packaging.
Build scripts write the actual native target and library filename. Installation
touches only this package directory; deleting it removes Markdown routing at
host startup. Tests include the public ABI, delayed/cancelled and short reads,
huge offsets, malformed encodings, hostile content, exact Unicode/entity maps,
cross-page search and Markdown fence context.

## Dependency review and licensing

`pulldown-cmark = 0.13.6` is pinned, with all default features disabled: no HTML
output feature or CLI dependency is linked. The reviewed upstream README and
`Parser::into_offset_iter` source describe its pull event/source-range model.
Only those events are transformed into the public semantic API; no parser or
renderer implementation is imported from the host or another plugin.

Plugin-owned code is MIT. Third-party versions and licence copies are in
THIRD-PARTY.md and notices/. The separate public project API/SDK release
currently declares no licence; the plugin's MIT licence does not license it.
