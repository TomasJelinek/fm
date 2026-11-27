# Midnight Markdown viewer 0.1.0

Independent `cdylib` package `midnight.viewer.markdown`, routing `.md`,
`.markdown`, `.mdown` and `.mkd` before ordinary source/text viewers. It emits
public `document-blocks.v1` models; the host supplies all window, theme,
selection and font rendering. Explicit Text mode returns raw decoded text.

CommonMark paragraphs, six heading levels, emphasis, strong text, code spans,
fenced/indented code, quotations, list items, rules, entities, escaped text,
links and image alternative text are represented semantically. Unordered list
items carry the host's bullet. Ordered list items display their number and the
author's delimiter (`7.`, `8)`), counted from the list's start, as paragraph
text mapped to the source marker, so numbers survive search and copy. Further
paragraphs of one item are paragraphs at the item's depth without a second
marker; an item that opens with a code block, heading, table, quote or nested
list shows its number on a line of its own and an unordered one shows no bullet.
HTML is literal inert text. Images are alt text only. No HTML renderer, browser,
JavaScript, network request, image/font/style fetch or automatic link launch
exists. Link metadata carries every target of 1 to 8192 bytes without control
characters, in its source range; an email autolink (`<someone@example.com>`) gets a
`mailto:` target. The host's single link policy decides on an explicit user action
what a target does, and names what it refuses
([viewer links](../../docs/developer/viewer-preferences.md#links)).
GitHub-style pipe tables, task checkboxes and strikethrough are enabled. Tables
carry per-cell left/center/right alignment, header metadata, inline styles and
source anchors; the host supplies column layout and borders. Empty cells and
escaped pipe characters are supported. Task markers are read-only `[x]` / `[ ]`
text so they remain readable in user-selected fonts without emoji/ballot glyphs.
Footnotes, mathematics and diagrams are not enabled. Reference links resolve
within the bounded parse window.

## Correctness and limits

UTF-8 and BOM UTF-16 LE/BE decode with exact source-byte anchors, including
malformed units and CR/LF/CRLF normalization. Unsafe controls become visible
characters, whether they occur in the source or come from numeric character
references such as `&#27;`: C0 controls other than tab/newline become control
pictures (`␛`, `␍`), DEL `␡`, and C1 controls and the explicit bidirectional
embedding/override/isolate controls (U+202A–U+202E, U+2066–U+2069) U+FFFD, so
neither terminal escapes nor reordered ("Trojan Source") text reach displayed
or copied text. Implicit direction marks (LRM, RLM, ALM) remain for RTL prose.
Offsets are checked u64 values. Removed Markdown markers mean a
displayed boundary can have more than one source interpretation; the model
retains one actual boundary, never an interpolated offset. Entity
normalization exposes exact event endpoints where interior boundaries cannot
be established; a code span maps each displayed scalar exactly, including line
endings it displays as spaces. Copy uses displayed text, not source bytes
including markers. Uncached seeks look behind by four bytes after UTF-16 word
alignment, then select actual decoded boundaries. Semantic pages retain
complete paragraphs and tables around the requested anchor when they fit the
byte/block budgets and the public anchor/span/link/item wire budgets; a page
stops before a unit that would exceed them, so ordinary prose keeps its styles,
links and table geometry. The host places the requested source position within
that stable layout. A unit the read edge cuts is never presented as complete:
anchors inside it get a bounded fragment, and a later read parses it whole.
Oversized blocks use bounded scalar-safe fragments. Text-mode search preserves
visible Markdown markers, just as the Text-mode viewport does.

Viewport reconstruction starts at a proven complete top-level block boundary
(a checkpoint) and rescans at most 32 KiB before its anchor. Checkpoints come
from the rendering parser traversal, at least 1 KiB apart per read, so the
same context is not parsed a second time just to collect them. A session keeps
at most 256: when full it drops the checkpoint whose removal leaves the
smallest gap, weighted by distance from the latest viewport anchor, and always
keeps the beginning and the furthest one. Coverage stays dense where the user
reads and replayable behind them: after a 10 MiB forward read every gap within
the last MiB stays below the 32 KiB replay distance, and documents up to a few
hundred KiB stay replayable throughout. Searches never change these
checkpoints. A read keeps the checkpoint decision made when it was planned.
Each session retains an LRU of up to 16 cropped semantic pages, with a 1 MiB
retained-allocation budget. Revisiting a cached source anchor, direction and
viewport budget needs no content read, decode or Markdown parse. Cache identity
is the opened session/revision; close/reopen drops it, and cancellation still
suppresses cached replies. A further single parsed context, bounded to 1 MiB,
reuses already-read data for nearby anchors with different directions/budgets.
Replacing that context evicts it; there is no whole-file preload or file-size
limit. Ordinary Markdown source line endings become spaces; explicit hard
breaks remain displayed newlines. Known fence/quote context therefore survives
nearby random or sequential viewports. Regions more than 32 KiB past every
retained checkpoint use a plain paragraph fallback.
A parse proves a top-level boundary only when the line after the unit is
complete too, so a read that ends inside the next line's indentation proves
none there. A top-level fenced block, GFM table or HTML block of CommonMark
types 6 and 7 that is longer than one read cannot prove its end inside a
parse. Plain reads that continue contiguously from where the parse stopped
find that end (the closing fence, or the first blank line after the table or
HTML block) and prove the checkpoint after it, so sequential scrolling and
search continue semantically after long fenced logs, generated tables and HTML
dumps. A table line before that blank line that could start another block (a
heading, quote, list item, fence, rule or HTML, after any indentation) ends the
search without a checkpoint. A seek past such a block before it was scanned
shows source text with literal markers until the viewer has read through the
block from its start. Any other single top-level block longer than a 64 KiB
read (a long list or block quote, an HTML block of types 1-5, or a table
interrupted that way) ends checkpoint discovery: pages more than 32 KiB past
the last checkpoint before that block show source text with literal markers
for the rest of the session. A block directly followed by a line longer than
the rest of its read, such as a 40 KiB paragraph followed by a 30 KiB
single-line list item, can end checkpoint discovery the same way, because a
read that ends inside that line does not prove the boundary between the two.
If invalid/control text expands beyond the decoded context budget, viewport
and search reuse a nearby fragment of the same bounded read and discard the
unproven parser context rather than skipping the requested content.

At most 64 KiB is read/decoded per read step. Emitted displayed text is at most
the caller's budget, capped at 64 KiB, and the public block/anchor/wire budgets.
Scalar and plain fallback pages retain an 8 KiB text cap. Requested lines bound demand around
the source anchor; completing an enclosing paragraph/table may add context.
Paragraphs, headings, list items, rules and table rows each count as a displayed
line even when their text has no literal newline.
A top-level unit with more than 16,384 parser events, nesting deeper than 16,
more than 64 table columns or a row with extra cells is shown as its literal
source text with exact anchors. Earlier and later units of the same read stay
semantic, and the unit's end is still recorded as a checkpoint. After 65,536
events in one read, the remaining units are shown as one literal block while
their boundaries are still recorded, so later reads render them semantically.
Complete tables that fit the bounded parsed context keep every row
for stable column widths. Tables exceeding the caller/read or exact-anchor/wire budgets use bounded
wrapped row text without partial table geometry. The same readable fallback applies to
distant uncached seeks. Older hosts ignore the optional table/strikethrough metadata and
show the row text without rich decoration.
Adjacent text with one style shares one span, which keeps dense code within
the span budget. Only a single unit too dense for the public wire budgets first
discards optional styles and link metadata, preserving displayed text and exact
anchors so its visible content and search remain consistent; if it still does
not fit, it becomes the 8 KiB fragment, cropped from the decorated unit so the
fragment keeps the decoration that fits.
Only bounded input is handed to the upstream parser, whose individual parser
step has no cancellation API. Constructing the parser runs pulldown-cmark's
complete block pass over the bounded window, which cannot be cancelled.
Cancellation is otherwise polled every 256 parser events, every 1,024 decoded
scalars, every 4,096 searched scalars and between SDK asynchronous reads; each
poll may call the host. Budgets are per plugin instance, and the host creates
one instance per viewer: at most four requests decode, parse or match at once
(a request waiting for a content read holds no work slot; the SDK bounds
pending reads), and at most 16 sessions. Each session's page LRU and parsed
context retain at most 1 MiB each, in addition to one request's 64 KiB read,
its per-scalar decoded anchors (16 bytes each), parser/event scratch and the
SDK's bounded queues. Open reads only the first four bytes, to detect a BOM.

Search parses each 64 KiB read once and matches the displayed text of every
top-level unit proven complete in it, then continues at the last proven
boundary, so each source byte is read and parsed about once (1.01× of a 3.8 MB
file measured). It keeps those boundaries privately and never changes the
viewport's checkpoints. Without a proven boundary within 32 KiB it matches
decoded source text, as the plain viewport fallback displays it. Blocks are
joined by a line break, also across reads, so a match never joins the end of
one paragraph to the start of the next. Streaming KMP retains only a
query-sized window; matches can cross reads. Insensitive search compares
Unicode lowercase expansions (not locale collation or full case folding).
Backward search scans from the beginning, retaining only the latest match
before the anchor; its latency is proportional to the preceding content. It is
cancellable and has no whole-file size cap. Progress is at most four times a
second. Close drops plugin state; the host owns closing the content lease.
Requests and Close of another envelope generation of a session ID are stale:
they are rejected, and a stale Close leaves the live session open.

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
./scripts/ci.ps1 -SdkConfig <sdk>/.cargo/config.toml -BuildRoot <folder>/_build
./scripts/build.ps1 -SdkConfig <sdk>/.cargo/config.toml -BuildRoot <folder>/_build -OutputDirectory <install>/viewers/markdown
```

Linux (Python 3.11+ is used only to read the released SDK config):

```sh
bash scripts/ci.sh <sdk>/.cargo/config.toml <folder>/_build
bash scripts/build.sh <sdk>/.cargo/config.toml <folder>/_build <install>/viewers/markdown
```

CI runs formatting, all tests, strict Clippy and native release packaging.
Build scripts write the actual native target and library filename. Installation
touches only this package directory; deleting it removes Markdown routing at
host startup. Tests include the public ABI, delayed/cancelled and short reads,
huge offsets, malformed encodings, hostile content, exact Unicode/entity maps,
cross-page search and Markdown fence context. Scroll regressions cover mixed
headings, fenced code, lists and tables in both directions, identical revisited
slices, no source rereads on cache hits, LRU eviction, changed viewport budgets,
cancellation and new revisions after close/reopen. Further regressions cover
the host's production page budget, paging Text mode to EOF, scrolling back
through a read document, searches that leave viewport context intact, bytes
read and host cancellation polls per byte searched, over-budget units and long
fenced logs between ordinary sections, ordered lists, controls from character
references, stale generations and superseded requests waiting for reads.
Heap-metered test binaries serialize their tests through the meter's lock.

## Dependencies and license

`pulldown-cmark = 0.13.6` is pinned, with all default features disabled: no HTML
output feature or CLI dependency is linked. The reviewed upstream README and
`Parser::into_offset_iter` source describe its pull event/source-range model.
Only those events are transformed into the public semantic API; no parser or
renderer implementation is imported from the host or another plugin.

The plugin's own code is under its GPL-3.0-or-later [LICENSE](LICENSE). Third-party versions and
license copies are in [THIRD-PARTY.md](THIRD-PARTY.md) and `notices/`.
