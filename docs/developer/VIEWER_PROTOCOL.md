# Viewer protocol reference

The schemas below describe public API 0.1.0, C ABI 1.0. Source definitions are
[messages.rs](../../api/midnight-plugin-api/src/messages.rs),
[viewer.rs](../../api/midnight-plugin-api/src/viewer.rs) and
[document.rs](../../api/midnight-plugin-api/src/document.rs). See
[ABI and lifecycle](ABI_AND_LIFECYCLE.md) for ownership and scheduling.

## Envelope and CBOR

An ABI `Request` carries `id`, `session`, `generation`, `kind`, zero `flags`,
borrowed CBOR `payload` and optional host-owned raw `content`. An `Event` echoes
`request_id`, `session`, `generation`, `kind`, zero `flags`, and separately owned
payload/content buffers. Identity is in the envelope, not repeated in every CBOR
model. Source revisions are additional content identities, not generations.

Call `api::codec::{encode, decode}` for typed models. Decoding first validates the
entire wire envelope without allocating model collections, then decodes and
validates the model. Encoding validates the model, writes through a bounded writer
and validates the resulting wire representation too.

Version 1 uses definite CBOR maps with unsigned numeric field tags in strictly
increasing order. Duplicate/unsorted keys, indefinite containers, CBOR tags,
floats, unsupported simple values, malformed UTF-8 and trailing bytes are rejected.
Integers, strings, byte strings, booleans, null and definite arrays/maps are
supported by the bounded wire grammar. Unknown optional fields may be skipped
only after the same complete validation. An unknown field still consumes byte,
item and depth budget. All strings in the schemas are UTF-8; source bytes need
not be UTF-8.

The tables list `tag: field` in wire order. `?u64` denotes `Option<u64>`; use the
shared codec's encoding rather than inventing a different representation for
missing optional values.

## Requests and host services

| ABI kind | Payload | Fields |
| --- | --- | --- |
| `REQUEST_OPEN` (1) | `Open` | `0: reader u64`, `1: length ?u64`, `2: revision u64`, `3: mode u16`, `4: display_name String` |
| `REQUEST_VIEWPORT` (2) | `Viewport` | `0: source_anchor u64`, `1: backwards bool`, `2: max_bytes u32`, `3: max_lines u32` |
| `REQUEST_SEARCH` (3) | `Search` | `0: query String`, `1: source_anchor u64`, `2: backwards bool`, `3: case_sensitive bool` |
| `REQUEST_CLOSE` (4) | No typed close model | Empty payload is allowed; session/generation are in the envelope |
| `REQUEST_SERVICE_REPLY` (5) | `ReadReply` | `0: revision u64`, `1: offset u64`, `2: bytes u32`, `3: eof bool`, `4: status u32` |
| `SERVICE_READ_AT` (1001) | `ReadAt` | `0: reader u64`, `1: revision u64`, `2: offset u64`, `3: length u32` |
| `SERVICE_CLOSE_READER` (1002) | `CloseReader` | `0: reader u64` |

Reader IDs are nonzero opaque host handles. An unknown length is distinct from
zero. The current local broker supplies a known length and one reader per
instance; plugins must preserve the general schema's optional length semantics.
`display_name` is display-only and does not authorize OS access.

`ReadAt.length` is nonzero and bounded; `offset + length` must not overflow `u64`.
Successful `ReadReply.bytes` must equal the separate raw buffer's length and be
no greater than the requested length. Its revision and offset must match the
request. A short read is not EOF unless `eof` is true. A nonzero status requires
`bytes = 0` and `eof = false`. Handle read errors before decoding bytes.

The SDK intercepts service replies; they do not reach `Viewer::handle` as user
requests. `Context::read` is the public Rust read helper. `CloseReader` and its ABI
service kind exist, but the SDK currently exposes no `Context::close_reader`
method. Normal viewer plugins leave reader closure to host session teardown.
The raw host service closes its one reader and replies using `ReadReply`; it is
not a filesystem-provider API.

Viewport anchors are source-byte positions. `backwards` requests content before
the anchor; a decoder must choose valid decoding boundaries, not arbitrary UTF-8
indexes. Respect byte/line bounds and retain only bounded checkpoints. Search
semantics belong to the plugin, so document case matching, encoding and backward
search behavior; do not claim full Unicode case folding from a lowercase-only
implementation. Read and search progress must remain cancellable.

## Events

| ABI kind | Payload | Fields / interpretation |
| --- | --- | --- |
| `EVENT_OPENED` (1) | `Opened` | `0: mode u16`, `1: length ?u64`, `2: initial_anchor u64`, `3: estimated_lines ?u64` |
| `EVENT_TEXT_PAGE` (2) | `TextPage` | Text or Source mode; schema below |
| `EVENT_BINARY_PAGE` (3) | `BinaryPage` | Binary mode; raw bytes in `Event.content` |
| `EVENT_DOCUMENT_PAGE` (4) | `DocumentPage` | Document mode; semantic blocks |
| `EVENT_SEARCH_MATCH` (5) | `SearchMatch` | `0: source SourceRange`, `1: preview String`; source length is nonzero |
| `EVENT_PROGRESS` (6) | `Progress` | `0: completed u64`, `1: total ?u64`, `2: phase u16`; completed cannot exceed known total |
| `EVENT_COMPLETED` (100) | Empty | Successful terminal for this request |
| `EVENT_CANCELLED` (101) | Empty | Cancelled terminal for this request |
| `EVENT_FAILED` (102) | `Failure` | `0: code u32`, `1: message String`; failed terminal |
| `EVENT_QUIESCENT` (103) | Reserved lifecycle notification | Current SDK/host use `destroy`/`NOT_QUIESCENT`, not this notification |

`Opened.mode` must equal the requested mode; its known length must agree with the
captured content and initial anchor must fit a known length. A page revision must
equal the opened revision and every source range must fit the captured file.
Only Binary pages carry nonempty raw content. Opened, pages, search matches and
progress are nonterminal; each accepted request also needs a terminal. Current
host dispatch ignores unknown event kinds after safely releasing their buffers;
that does not negotiate support for a new output model.

## Modes and semantic models

| Mode | Numeric value | Manifest mode | Required model |
| --- | --- | --- | --- |
| Text | 1 | `text` | `text-page.v1` |
| Source | 2 | `source` | `text-page.v1` |
| Binary | 3 | `binary` | `binary-page.v1` |
| Document | 4 | `document` | `document-blocks.v1` |

The host supplies theme colors/fonts, layout, wrapping, glyph rendering, focus,
selection and clipboard behavior. Plugins supply plain text plus semantic styles
or document blocks. There are no plugin-provided pixels, shell commands, scripts,
web views, network-loaded fonts or executable markup in these models. Copy uses
displayed decoded text; Binary copy is handled as binary presentation by the host.

### Text and Source

`TextPage` fields are `0: revision u64`, `1: source SourceRange`, `2: text String`,
`3: spans Vec<Span>`, `4: anchors Vec<Anchor>`, `5: has_before bool`,
`6: has_after bool`, `7: partial_line bool`.

`SourceRange` is `0: offset u64`, `1: length u64`; its exclusive end is computed
with checked addition. `Span` is `0: start u32`, `1: end u32`, `2: role u16`,
`3: flags u8`. Spans are nonempty, ordered, disjoint and inside valid UTF-8
boundaries in `text`. Flags are bit 0 bold, bit 1 italic, bit 2 underline; all
other bits are invalid for this model version. `partial_line` records that a
large logical line continues outside the fragment. Before/after flags describe
whether more content exists outside this page, not an array of loaded pages.

Published style roles are:

| Value | Constant | Meaning |
| --- | --- | --- |
| 0 | `STYLE_PLAIN` | Plain text |
| 1 | `STYLE_KEYWORD` | Language keyword |
| 2 | `STYLE_STRING` | String literal |
| 3 | `STYLE_COMMENT` | Comment |
| 4 | `STYLE_NUMBER` | Number |
| 5 | `STYLE_TYPE` | Type |
| 6 | `STYLE_FUNCTION` | Function |
| 7 | `STYLE_OPERATOR` | Operator |
| 8 | `STYLE_HEADING` | Heading |
| 9 | `STYLE_LINK` | Link text |
| 10 | `STYLE_CODE` | Code |
| 11 | `STYLE_INVALID` | Visible invalid source representation |

Role numbers are semantic labels, not palette indexes. The validator does not
reject unknown role numbers, but that is not a guarantee of a distinct renderer
style. Use the published roles for portable output and let the host choose its
theme. Source viewers must also provide accurate text when styling is omitted
or exhausted by a budget.

### Source mappings

`Anchor` is `0: text_offset u32`, `1: source_offset u64`. Every text page has first
and last anchors mapping text offset 0 to `source.offset`, and `text.len()` to
the source range's exclusive end. Empty text/range uses a single endpoint anchor.
Text offsets strictly increase and are UTF-8 character boundaries; source offsets
are nondecreasing and remain within the source range.

Only listed anchors are exact. Never interpolate an unlisted boundary, even when
source and text lengths happen to agree. UTF-16 decoding, malformed byte escapes,
newline normalization, removed Markdown markers and entity expansion can all
change offsets. For example, a UTF-16 character may consume two source bytes and
three UTF-8 bytes; `text_offset + source.offset` is then incorrect. Emit an anchor
at every exact boundary you need, within the aggregate item/anchor limits.
Where a transformation cannot establish an interior mapping, retain real
endpoints and do not invent precision. Search results use exact source ranges;
screen coordinates and selection rendering stay in the host.

### Binary

`BinaryPage` fields are `0: revision u64`, `1: source SourceRange`,
`2: has_before bool`, `3: has_after bool`. Its raw content length must equal
`source.length`, at most 256 KiB. The host formats addresses/hex/character display;
the plugin sends raw bytes separately from CBOR. All source offsets remain `u64`,
including positions beyond 4 GiB.

### Document

`DocumentPage` fields are `0: revision u64`, `1: source SourceRange`,
`2: blocks Vec<Block>`, `3: has_before bool`, `4: has_after bool`.
Each `Block` has `0: kind u16`, `1: depth u8`, `2: heading_level u8`,
`3: text TextPage`, `4: links Vec<Link>`.

Block kinds are paragraph (1), heading (2), code (3), quote (4), rule (5), and
list item (6), exported as `BLOCK_*` constants. Depth is at most 16. Headings have
level 1 through 6; other blocks require heading level 0. Block text revisions must
equal the document revision. Their ordered source ranges cannot overlap and must
fit inside the document range; gaps for syntax markers are allowed. Every block's
text still obeys all TextPage span/anchor rules.

`Link` has `0: start u32`, `1: end u32`, `2: target String`. Link ranges are ordered,
nonempty, disjoint UTF-8 boundaries in their block text. Targets are bounded and
may not contain control characters. The public schema does not itself whitelist
URL schemes; plugins and any explicit host action must apply appropriate intent
and scheme policy. A link target is inert metadata and is never an automatic
launch, fetch or shell action. The current Markdown plugin additionally limits
schemes, renders HTML as literal text and images as alternative text. Those are
plugin behaviors, not HTML/browser facilities in the public API.

## Limits

All byte limits are UTF-8 or raw byte counts, not character counts. Per-field
limits and aggregate CBOR limits apply together; a model at every individual
maximum can exceed the overall item/byte budget and fail encoding.

| Item | Limit / validation |
| --- | --- |
| CBOR payload | 1 MiB; 65,536 total values/map keys; nesting depth 16 |
| Text page | 256 KiB decoded text, 4096 spans, 16,384 anchors |
| Raw Binary page | 256 KiB, exactly its source range length |
| Read request/reply | 256 KiB; request length at least 1; checked `u64` range |
| Viewport request | `max_bytes` 1 through 256 KiB; `max_lines` 1 through 4096 |
| Search query | 1 through 4096 bytes |
| Search preview / Failure message | At most 4096 bytes each |
| Open display name | At most 256 KiB; display-only |
| Document page | At most 512 blocks |
| Document block depth / heading level | Depth at most 16; heading 1 through 6 |
| Document totals across blocks | 256 KiB text, 4096 spans, 16,384 anchors, 4096 links, 64 KiB target bytes |
| Links per block | At most 4096, also subject to page totals |
| Individual link target | 1 through 8192 bytes; no control characters |

Wire item accounting includes array/map containers and map keys; anchors alone
can consume several items each. Use `Output` constructors or `codec::encode` to
check the combined representation and reduce page size/optional metadata when
necessary. Do not spin retrying an intrinsically oversized model as if it were
temporary queue pressure. See [transport budgets](ABI_AND_LIFECYCLE.md#limits-are-separate-layers)
for SDK and host queue/lease limits, which are additional to this table.
