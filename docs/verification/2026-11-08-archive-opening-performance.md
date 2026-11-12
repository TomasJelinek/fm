# Archive opening performance — 2026-11-08

The user reported slow entry into compressed archives. Measurements separated
captured-source reads, sandboxed worker Open, root listing, repeated listing,
pane indexing and first/cached member reads. No UI code or containment policy was
changed. The independent archive plugin still consumes only packaged public APIs.

## Findings and implementation

- ZIP already indexed the central directory without decoding members. It was not
  the eager-decompression problem addressed here.
- 7z previously decoded every file before returning Opened. It now parses the
  header and stream metadata first. Reading a member decodes only its containing
  block; all available member/block CRCs and actual lengths must pass before any
  staged bytes enter the retained cache. Solid siblings share that verified cache.
- TAR previously copied each regular payload into a separate buffer. A checked
  seek iterator now indexes ranges in one immutable buffer. It rejects skips past
  EOF, including truncated payload/padding. Compressed TAR still decodes its outer
  stream, then indexes the resulting buffer without duplicating member payloads.
- Review found that the pinned 7z BlockDecoder counts substreams rather than
  interleaved no-stream directory/empty-file records. Its private decoding view
  now contains stream ordinals only; listing IDs retain original member identity.
  No third-party source or public API was patched.

Salamander references inspected: `src/plugins/zip/list.cpp`,
`CZipList::ListArchive/List` (central directory iteration), and
`src/plugins/7zip/7zclient.cpp`, `C7zClient::ListArchive` (open and enumerate item
metadata, separate from extraction). The implementation uses the independently
licensed pinned Rust decoders rather than copying Salamander source.

## Measurements

Owned fixtures contain 64 deterministic 256 KiB files: 16 MiB decoded total.
TAR is 16,811,008 bytes; independent-block 7z is 8,723,656 bytes; solid 7z is
8,779,304 bytes. Both variants contain the same payloads. Plugin benchmarks
exclude fixture generation/source cloning and compare exact first/last contents.

Five optimized parser-only samples:

| Format | Before Open | After Open |
| --- | ---: | ---: |
| TAR | 4.95–5.55 ms | 0.055–0.150 ms |
| Independent 7z | 571–824 ms | 0.087–0.173 ms |
| Solid 7z | 569–660 ms | 0.086–0.255 ms |

The real Windows host benchmark uses the same debug host executable and release
archive workers, with native source validation, AppContainer startup and IPC.
Each mount is new. A separately timed full source read warms source I/O before
Open. The table reports medians of three samples, not a cold-disk or rendered-frame
benchmark:

| Format | Before host Open | After host Open |
| --- | ---: | ---: |
| Independent 7z | 601.1 ms | 33.7 ms |
| Solid 7z | 451.6 ms | 34.5 ms |
| TAR | 50.6 ms | 50.3 ms |

TAR's parser and retained-memory improvements do not yield a meaningful end-to-end
gain on this fixture: source transfer/worker startup dominate. The first launch of
the freshly installed worker took 1.54 s; it is recorded separately and excluded
from the warm measurements, not claimed fixed. An earlier cold baseline launch
took 1.61 s. The cause of that cold-start cost was not isolated in this change.

7z work is deferred, not eliminated. In the host measurement the first independent
block read took 9.2–9.5 ms; the first solid read took 405–420 ms to verify the whole
16 MiB block. Cached reads took less than 1 ms. Root listings for these fixtures
took less than 1 ms. ZIP regression probes opened/read a tiny archive, one stored
32 MiB member and a 10,000-file archive; no ZIP speed improvement is claimed.

## Validation and installed artifact

- Independent plugin optimized suite: 26 tests passed. Two timing benchmarks
  remain ignored in ordinary tests; the TAR/7z benchmark was explicitly run and
  passed separately.
- Tests include member-only/block-only CRC failure, short decode without CRC,
  no partial cache publication, unrelated corrupt blocks, cache reuse, duplicates,
  unsafe siblings, output budgets, bounded offsets, truncated TAR and a valid
  interleaved empty-file/directory/zero-length-stream 7z fixture.
- Host ordinary suite: six tests passed; native sandboxed worker integration test
  separately passed, including cancellation and duplicate-provider rejection.
- Plugin and host strict all-target Clippy passed; plugin rustfmt check passed.
- Real installed worker mount/list/read probes passed with empty discovery
  diagnostics. Linux/macOS runtime tests were not run in this Windows session.

Worker before: `2776D1926D2765EBC294737F3A635EAC2F17F6820C35C8A81A6BCAE464EDF233`.
Measured worker after: `A7A02219B78715F425E40DD7D1C9FF54F2F78337552FE4AC7C2E9CE5B40E4B1B`.
Final canonical-SDK build: `1A577D5361042AD3C09EE1EFE528213A8D969F4D742BB83275CD30778B9C55AA`.
The new release worker was installed into the registered private runtime used by
`_build/debug/me.exe` and into `_build/qa/ui-polish/runtime-stage`. The main app
was left running; the replacement applies to new archive mounts. Other plugins
and the application executable were not replaced.

Final public SDK bundle:
`_build/pdf-viewer/releases/worker-sdk-1794092695373018700/.cargo/registry.toml`.
Independent review caught that the initial raw Cargo package export included VCS
metadata and therefore did not match CI's canonical artifact hashes. The official
packager removes VCS metadata and normalizes tar/gzip metadata. Its public source,
manifest and lock contents match the initial bundle byte-for-byte. Cargo generated
the correct lock in an owned temporary export; it exactly matches the original
committed plugin lock. No public or third-party dependency change is included.
The canonical build repeated all 26 tests and real sandbox Open/List/Read probes;
warm 7z host Open medians were 37.8 ms (independent) and 39.9 ms (solid).

Evidence under `_build/qa/archive-speed/`: `host-before-with-reads.log`,
`host-after-with-reads.log`, `host-after-tar-zip.log`, `host-tests.log`,
`host-native-worker.log`, `host-clippy.log`, `plugin-tests-after.log`,
`plugin-performance-after.log`, `plugin-clippy.log`, `plugin-canonical-lock.log`,
`plugin-canonical-build.log`, `plugin-canonical-tests.log`, `host-canonical.log`.
Generated evidence is not
committed. The reproducible probes live in
`crates/midnight-filesystem-host/examples/measure_open.rs` and the independent
plugin's `tests/performance.rs`.

Resource limits, source revision checks and SHA-256 destination readback remain
unchanged. Listing is not a full archive integrity test. The worker still receives
the entire compressed source before indexing; streaming/range-broker ZIP opening
and cold worker startup are separate remaining performance opportunities.

Independent source review found no introduced regression. It identified an
inherited decoder limit: sevenz-rust2 bounds member readers to declared lengths
and does not probe beyond the declared block extent. Validation checks short
output and available CRCs for the declared contents; it does not independently
prove absence of extra decoded output or a malformed codec tail beyond that
extent. Tests and the term "verified block" above refer to those declared contents,
not to a stronger independent codec-tail validation guarantee.
