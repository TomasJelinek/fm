# Optional source-coordinate API verification

Source viewers can answer exact logical line/column lookups independently of
viewport delivery through the additive [Locate contract](../../api/SOURCE_POSITION.md).
Existing page payloads and C tables are unchanged. Older SDKs reject the optional
request as `UNSUPPORTED`; the host preserves that code and keeps viewing usable.

The independently packaged source plugin scans in reads of at most 32 KiB,
retains at most 256 sparse checkpoints, checks cancellation between bounded
decoder/counter batches, and leaves a plugin work slot available for viewing.
The work is asynchronous and does not read user preferences. Counters use `u64`
and count decoded Unicode scalars, with documented CRLF/BOM semantics.

Windows verification on 2026-11-08:

- Initial wire test failed on the old SDK's actual ABI `UNSUPPORTED` result.
- Public API: 23 tests passed, including exact offsets/coordinates above 4 GiB,
  one-based validation, existing CBOR fixtures and ABI descriptor checks.
- SDK: 33 tests passed. An integration failure exposed its output event range;
  a focused failing queue test was added before admitting the optional event.
- Standalone source plugin: all 27 tests passed against an immutable exported
  API/SDK bundle, without host-source dependencies. Tests cover UTF-8/UTF-16,
  combining scalars, emoji, tabs, CRLF, malformed input, short reads, reverse
  coordinate queries, exact checkpoints, source bounds and cancellation.
- A distant indexing request remains pending while another viewport reaches its
  page and terminal event. Cancellation stops further indexing reads and prevents
  stale coordinates. Checkpoint storage remains bounded after 100,000 inserts.
- Public API/SDK and source plugin strict all-target Clippy passed. The generated
  C header compiled, and fresh C/Rust layout reports matched every public struct
  size, alignment and field offset. The host's separate integration
  gate passed 49 tests (5 unrelated native tests ignored), including coordinate
  revision/mode/source-bound validation; host strict Clippy passed too.

Logs are under `_build/goal/viewer-controls-completion/source-position/`; host
logs are under `_build/goal/native-filesystem-host/`. The final SDK bundle is
`_build/plugin-sdk-1794112915607548500`. Only the internal API/SDK checksum pins
changed in the source plugin lockfile; no third-party dependency or feature changed.

The optimized Windows package for native UI integration is under
`_build/goal/viewer-controls-completion/source-position/package/midnight-viewer-source/`.
Its DLL SHA-256 is
`09ECFA256E1C509F8B4B08820456A50C31E7761315EED814DF98422CD33BEE68`.
Native viewer presentation and application persistence qualification are recorded
separately by the viewer-controls implementation; these API/plugin tests alone
do not establish deployed UI behavior.
