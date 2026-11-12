# Optional source coordinates

`REQUEST_LOCATE` (6) accepts `messages::Locate { source_anchor }`. An updated
source viewer can return `EVENT_TEXT_POSITION` (7) with `viewer::TextPosition` and a
normal terminal event. Successful completion without a position means exact
coordinates were not available within this request's work budget. It is a
page-specific absence, not a session-wide unsupported feature. Existing `Open`, `Opened`, `TextPage`, requests, events and
C function tables are unchanged. A host must treat `UNSUPPORTED` as absence of
this optional feature, keep normal viewing available, and stop probing that
session. No new event is sent unless the host explicitly requests it, so updated
plugins remain usable by older hosts.

`source_anchor` and the returned `source_offset` are `u64` source byte offsets.
The result is the first complete decoded boundary at or after the requested
anchor. A BOM is excluded from coordinates. An anchor inside a UTF-8 sequence,
UTF-16 surrogate pair, or CRLF can therefore return a later byte offset; consumers
must use exact `TextPage.anchors` and never interpolate byte positions. The host
validates the source revision, file bounds, and text/source viewing mode.

Line and column numbers start at one. Columns count decoded Unicode scalar
values, including each tab, combining scalar and malformed-sequence replacement
as one. They do not count graphemes, display cells, UTF-16 words or bytes. CRLF
and lone CR normalize to LF. LF, U+2028 and U+2029 begin logical lines. Other
controls retain the source decoder's visible one-scalar substitution, so their
coordinates match the displayed source. Wrapping has no effect on logical lines.

Locate is independent of Viewport. The source plugin keeps at most 256 exact
scalar-boundary checkpoints per session and coarsens them as the scanned range
grows. Each continuation reads at most 32 KiB and decodes at most 64 KiB; the
decoder and counter check cancellation between bounded batches. No complete file
or line-offset table is retained. At least one plugin work slot remains available
for viewport requests when location scans run.

The source implementation limits each Locate to **256 KiB of gathered source
bytes** from its nearest exact checkpoint. The entire bounded gather is reserved
before its first read, including lookahead and bytes later reread because decoding
could not consume them. Short replies fill that same reserved gather: repeated
requests for an unreturned suffix do not mean those bytes were delivered twice.
This is a conservative bound on delivered bytes, not the sum of requested lengths
when a provider returns short reads. Open's separate 512-byte detection read is
outside this per-Locate budget.

Targets farther than the budget from a known checkpoint complete without reading.
Exhaustion while decoding also completes without a position, retaining only fully
decoded exact checkpoints. Content delivery never waits for coordinates. Hosts
show unknown coordinates for that page and do not repeatedly resubmit the same
declined request. A new page can try again, so returning near a known checkpoint
recovers exact positions. Consumers cancel obsolete lookups and discard stale
revision/request results; they must never invent line one at a distant seek.
