# Bounded large-file text viewing, 2026-11-10

This record covers the independent basic, source and Markdown plugins, the host
capture/read-path audit and a real 10 GiB native-file Service probe. It does not
establish a universal first-paint time or whole-application memory limit.

## Contract and audit

Opening or seeking text/code prepares a bounded visible window plus small decode
or parser context. No whole-file buffer, mapping, parse or line index is required.
File length remains `u64`; this change introduces no file-size ceiling.

| Route | Plugin Open read | Viewport source input | Retained state |
| --- | --- | --- | --- |
| Basic text/binary | 4 bytes | At most 8 KiB per gather | At most 1,024 checkpoints; bounded page/search state |
| Source text/code | 512 bytes | At most 64 KiB context; at most 8 KiB output text | At most 256 syntax and 256 coordinate checkpoints |
| Markdown text/document | 512 bytes | At most 64 KiB context; bounded semantic crop | At most 256 checkpoints, 16 cropped pages/1 MiB cache, one bounded parser-context page |

Default F3 basic-viewer routing can additionally sniff up to 4 KiB in the host to
distinguish text and binary. The four-byte Basic Open figure is not total startup
I/O; both detection stages have fixed bounds independent of file length.

The source and Markdown decoders cap decoded scratch text at 64 KiB; scalar
anchors and parser/style structures are also bounded by that input. These are
work-unit limits, not a claim that the process heap consists only of those bytes.
Metadata/page objects, SDK buffers and temporary copies also occupy memory.

The host native reader checks its retained identity/revision and reads only the
requested range. Generated-text capture allocates only the bounded requested
range and validates its immutable length. Archive capture also asks for ranges,
but the archive provider can first materialize and integrity-check a **complete
member into private disk storage** before exposing random access. ZIP members,
TAR members and 7z/solid decoding can therefore delay first view in proportion to
preparation work. This uses bounded transfer buffers, not a whole-member RAM
buffer; it is not covered by a native-file millisecond latency claim. Media/PDF
decoders and browser HTML rendering are separate routes.

Explicit search may read the complete file in cancellable bounded chunks,
including a backward search that scans an earlier prefix. Search is a user demand,
not startup or scrollbar prefetch. No automatic whole-file indexing is added.

## Corrected coordinate lookup

The prior source Locate implementation could scan from zero to a 9 GB seek just
to supply line/column metadata. Content could already appear, but the subsequent
automatic read chain was unbounded in total I/O.

Locate now completes without coordinates if its nearest known checkpoint is too
far away. For a nearer lookup, each source gather reserves from a cumulative
**256 KiB delivered-byte upper bound**, including boundary lookahead and bytes
reread after decoder truncation. Short replies refill the same reserved gather;
their repeatedly requested but unreturned suffix is not counted as delivered I/O.
Only complete decoded boundaries become checkpoints. Exhaustion returns no guessed
line number and does not disable later nearby lookup. Hosts retain this absence
only for the current page and retry appropriately after navigation. The optional
completion behavior uses the existing ABI; no schema, SDK, dependency or lockfile
changes were made. The installed source manifest also includes the separately
requested display-name improvement; its identity and extensions are unchanged.

See [source-coordinate semantics](../../api/SOURCE_POSITION.md) and
[presentation behavior](../developer/VIEWER_PRESENTATION.md).

## Evidence

Each plugin's `tests/large_file.rs` runs through its actual public plugin ABI
against a virtual **10,000,000,000-byte** source with four materialized fixture
bytes. It records actual read requests for first view, a 9 GB seek, return near
the start and a near-end seek. A test-only allocator measures incremental live
Rust allocations; every scenario asserts less than 16 MiB. That guard is test
evidence, not a new runtime memory quota. These figures exclude OS page caches,
code mappings, stack and native renderer/font memory.

Measured Windows debug test values:

| Route | Open-to-first-page | 9 GB viewport | Viewport bytes requested | Peak incremental Rust allocations across visits |
| --- | ---: | ---: | ---: | ---: |
| Basic text | 10.394 ms | 10.393 ms | 8,180–8,184 | 575,874 bytes |
| Basic binary | 0.105 ms | 0.032 ms | 1,280 | 10,551 bytes |
| Source text | 4.384 ms | 4.212 ms | 32,776–36,872 | 765,396 bytes |
| Source code | 7.170 ms | 4.062 ms | 32,776–36,872 | 775,854 bytes |
| Markdown text | 4.903 ms | 5.174 ms | 32,776–36,872 | 1,237,774 bytes |
| Markdown document | 8.163 ms | 4.513 ms | 32,776–36,872 | 2,521,991 bytes |

These timings cover the in-process ABI harness and virtual content, not opening
an application window or cold physical storage. Linux debug runs also passed;
its measured first source-code page was 22.568 ms in the final run. Timing is
recorded rather than asserted as a machine-independent deadline.

Logs under `_build/qa/large-text/`:

- `source-red-linux.log`: the old distant Locate starts indexing instead of
  completing without I/O; the bounded viewport test already passes.
- `source-budget-red-linux.log`: a 256 KiB NUL/control target reads **382,309 bytes**
  under the initial distance-only guard, proving decoder overlap must be charged.
- `source-final-green-{linux,windows}.log`: **30 tests passed** on each platform,
  including zero-read distant lookup, nearby recovery, cumulative overlap budget,
  127-byte short replies, cancellation, Unicode and exact source boundaries.
- `basic-green-{linux,windows}.log`: **28 tests passed** on each platform.
- `markdown-green-{linux,windows}.log`: **56 tests passed** on each platform.
- `source-final-clippy-windows.log`, `basic-clippy-windows.log` and
  `markdown-clippy-windows.log`: strict all-target Clippy passed for all three.

The initial Linux attempt used a stale registry selection and stopped on a
checksum mismatch; it is not behavior evidence. The successful runs selected the
already released public SDK matching each unchanged Cargo.lock. No checksum was
overridden. Before-images are retained under `_build/qa/large-text/baseline/`.

## Real Windows 10 GiB files: bounded-content checkpoint

The production Service and installed plugins opened three native sparse files,
each **10,737,418,240 bytes** logically and **524,288 bytes** physically allocated.
The probe renders a 1000×600 first frame, seeks to 90% with an acknowledged input,
waits 250 ms while consuming optional metadata updates, and returns to the top.
Unicode markers, mode, position, bounded text and frame sizes are asserted. All
three runs pass; owned fixtures and temporary probe executables are removed.

Evidence: `_build/qa/large-text-viewer/run-a84f22362492406faf0c648383bd3856/`.
Environment: Windows 11 Enterprise 26200, Core i7-13800H, approximately 31.6 GiB
visible RAM, debug host and installed release plugins. Process I/O includes other
reads, not just source bytes; private memory is sampled every 10 ms. These figures
are not an exhaustive heap proof or cold physical-disk bandwidth measurement.

| Route | Cold first frame | 90% seek | Return | Total process reads | Peak sampled private memory |
| --- | ---: | ---: | ---: | ---: | ---: |
| Basic text | 497.143 ms | 15.229 ms | 15.470 ms | 31,439 B | 10,678,272 B |
| Source/JSON | 485.311 ms | 15.043 ms | 16.758 ms | 101,639 B | 12,374,016 B |
| Markdown | 487.723 ms | 5.932 ms | 4.487 ms | 68,863 B | 6,692,864 B |

No whole-file scan or allocation occurs, including during the distant idle dwell.
The first process/session opening is still about half a second at this checkpoint;
the millisecond plugin-only figures above must not be substituted for first paint.
Startup-stage attribution and any subsequent optimization are recorded separately.

### Startup attribution

The `startup_phases` example measures real installed catalog discovery and
rendering in fresh processes, both in Service order and with rendering first.
Baseline logs: `_build/qa/large-text-viewer/startup-{default,render-first}-before.log`.
Font enumeration takes under 3 ms. Text catalog discovery takes 48–154 ms,
image/PDF discovery 142–209 ms and Chromium discovery 282–365 ms. A warmed font
layout plus initial raster takes about 18 ms. Thus the large cold-session cost
is primarily installed-plugin discovery, not reading the 10 GiB input.
These measurements precede any discovery optimization and retain all permission
checks; later evidence must distinguish an actual fix from a warmed cache.

### Repeated Windows service lookup

The installed catalogs contain roughly 1,400 path checks: each resolves the same
local `NT SERVICE\TrustedInstaller` SID before reading current ownership and ACLs.
A read-only native profiler isolates these costs on a protected fixture. Before
the change, 50 fresh identities take 3.680 ms and 50 complete seven-ancestor
checks take 13.920 ms. With successful fixed-service SID resolution cached, they
take 0.168 ms and 5.366 ms respectively; ACL traversal itself remains unchanged
(5.861 ms before, 5.653 ms after for 50 walks).

Only the successful SID byte array is retained. Current process tokens,
descriptors, path decisions, reparse checks and ACLs are not cached. A failed SID
lookup remains retryable. Concurrent first callers can perform duplicate initial
resolution but publish one successful value. No trust permissions are relaxed.

The causal cache regression initially records five resolver calls instead of two
(first failure, first success), then passes after the change. Windows plugin-host
tests pass **57 cases**, with five existing opt-ins plus the manual profiler
ignored. Repeated successful validations followed by file or ancestor ACL
revocation still fail; restoring protected ACLs restores success. The initial
warm-ACL test used an ordinary Windows path where the validator requires its
canonical extended spelling; fixing the fixture leaves production validation
unchanged. Logs are `trust-cache-red.log`, `trust-cache-full-green.log` and
`trust-profile-{before,after}.log` under `_build/qa/large-text-viewer/`.

Strict all-target plugin-host Clippy passes (`trust-cache-clippy.log`). The rebuilt
startup profiler retains the same 3 text, 2 image/PDF and 1 web providers without
diagnostics. Its warm Service-order catalog/render pass falls from 524.905 to
314.268 ms; render-first from 497.036 to 312.415 ms. These are stage-profiler
measurements, not a substitute for the real native-file Service probe below.

### Final real-file Service measurement

After rebuilding with the Windows optimization, all three real 10 GiB scenarios
pass again through the installed plugins. Evidence is retained in
`_build/qa/large-text-viewer/run-a900972b5f6d45968e667417bb9b6a78/`; the same
bounded sparse fixtures, assertions, dwell and cleanup policy apply.

| Route | Fresh-session first frame | 90% seek | Return | Total process reads | Peak sampled private memory |
| --- | ---: | ---: | ---: | ---: | ---: |
| Basic text | 372.011 ms | 17.454 ms | 15.771 ms | 31,439 B | 9,498,624 B |
| Source/JSON | 358.343 ms | 17.883 ms | 18.191 ms | 108,939 B | 10,645,504 B |
| Markdown | 340.732 ms | 7.512 ms | 4.444 ms | 77,198 B | 6,176,768 B |

First-frame time falls about 25–30% against the preceding real-file checkpoint.
This demonstrates bounded I/O and memory regardless of the 10 GiB source length,
not universal 1 ms opening: fresh-session catalog validation still dominates.
Seeking an already open file takes 8–18 ms here. Measurements use a debug host,
release plugins and cached OS resources; they are observations, not timing
guarantees for arbitrary storage, hardware or archive extraction.

## Final artifact and review

The debug application was rebuilt and deployed to the existing protected runtime;
both `_build/debug/me.exe` and the installed `me.exe` have SHA-256
`C05ADAF9BABD742FA00E95069D9F72D840AEE26F3646C28844DED4F103DBC66A`.
The runtime registration binds that build hash to the protected installation.
The installed source plugin is SHA-256
`2E9A0700D88013E787B217B963B8363F04EF1BD38DAB6CDA5509B5473D5AF9AB`.
Startup and actual native UI smoke both pass on the final host. The isolated
software-renderer startup snapshot is
`_build/qa/size-warnings/native-startup-682ea3acca1a42ca9dae6a2b44379019.png`;
saved user preferences were not modified. The fixtures and owned probe processes
are cleaned up. The normal app is not left running.

Independent review found no whole-input read/allocation path for native basic,
source/JSON or Markdown viewing, and no cached authorization decision in the
Windows optimization. The review corrected the Basic detection-read distinction
above. Workspace formatting and diff checks pass; all 1,714 handwritten source
files satisfy the 100,000-byte source limit. No dependencies or plugin ABI changed.
