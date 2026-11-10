# Archive thumbnail integration probe — 10 November 2026

Status: **Windows installed-worker archive image/video and cache probe passed**.
The Windows debug application and probe build, and the registered application
starts successfully. The probe exercises the ordinary thumbnail service with
real installed native/archive/image/video workers. Physical native panel input,
Linux execution and release qualification remain separate.

## Scope and source

`crates/midnight-explorer/examples/archive_thumbnails.rs` generates a fresh owned
ZIP with a tiny authored BMP, using store-mode ZIP records and a local CRC32
fixture helper. It captures the ZIP through the installed native filesystem
provider, discovers the installed archive worker and opens it through
`midnight_filesystem_host::Mount::open_captured`. The resulting archive node and
captured source revision feed `ArchiveThumbnailSource`; its capture goes to the
ordinary thumbnail `Service` and real installed image decoder.

The provider attests persistent member IDs with `stable_entry_ids = true` in its
manifest. Older manifests default to false and continue browsing normally, but
need updated archive package deployment before thumbnail capture is available.
Missing attestation must fail closed. Cache authority includes the
container/provider/decoder/source revision and stable member identity, excluding
the transient mount session ID. The host also includes the manifest's bounded
implementation version. The thumbnail target uses the fixed opaque
`archive-preview-v1` identity. No displayed archive path is reconstructed as a
native member filename, and no extracted member is written to public TEMP.

The successful installed run checked the following:

- Exact authored BMP dimensions and RGBA pixels from the real decoder, with a
  persisted `.mtc` cache record and a retained reference to the first frame.
- Cancellation before capture, followed by ordinary successful capture of the
  same mount.
- An empty target view followed by restoration, receiving current-generation
  pixels without additional member reads. This models backend viewport removal
  and return; it is not a physical panel scroll/Slint retention test.
- A fresh service and reopened archive with a different mount ID, the same source
  version and disk record filename, and zero member-content reads on that hit.
- An actual rewrite of the owned ZIP: the old source rejects validation, reads
  and recapture, while a fresh mount decodes changed pixels under a new key.
- Optional selected-video samples zero through five, using a supplied qualified
  AVI fixture and the real installed video decoder. The final run also verifies
  that these frames contain different RGBA pixels, rather than six copies of
  the same static frame.

The read counter wraps the captured thumbnail source, so the cache-hit assertion
means no archive member reads; archive reopening/index discovery and validation
may still read the native container. Shared-mount cleanup remains asynchronous;
the probe waits within the existing two-mount admission limit rather than
replacing or canceling another mount's work.

## Reproduction

Use the existing protected Windows runtime described by the
[HTML qualification](2026-11-10-html-viewer.md), or an equivalent protected runtime
with matching native/archive/image/video thumbnail packages. The current archive
manifest and trusted package inventory must include the stable-ID attestation.
The matching archive manifest must be deployed before running the probe.

Build the example using the normal pinned toolchain/offline SDK configuration;
the application example requires the internal filesystem-host dev-dependency.
Then run the helper under `_build/qa/archive-thumbnails/run-probe.ps1`, passing the
built example executable and explicit protected runtime. No shared preferences or
runtime registration is changed by this helper. Native runs may need execution
outside the additional Codex sandbox for protected plugin trust checks, as in
the earlier [native media qualification](2026-11-08-native-media-capture.md).

The optional existing authored video fixture is
`_build/qa/native-media/fixtures/video.avi` (368 bytes), previously used in that
8 November qualification. Supplying it requests fresh archive/video-worker
qualification; that earlier native-file test does not qualify archive video.

Each invocation creates a unique `run-<pid>-<timestamp>` directory beneath the
explicit `_build` fixture root. It retains the ZIP, private cache, `decoded.ppm`
and, only after all requested assertions pass, `report.txt`. The helper retains
stdout/stderr and the process exit code without deleting existing runs.

## Results and remaining gates

The following 10 November Windows checks are recorded under
`_build/qa/archive-thumbnails/`. These are scoped results, not a claim of a clean
whole-workspace or release qualification.

| Check | Result | Evidence |
| --- | --- | --- |
| Filesystem/thumbnail host suites | 66 passed; deployment/worker opt-in tests remain ignored | `backend-tests.log` |
| Filesystem/thumbnail host all-target strict Clippy | Passed | `backend-clippy.log` |
| Complete UI library | 673 passed | `ui-all-final.log` |
| Application thumbnail bridge | Nine passed, one existing opt-in native test ignored | `app-tests-final.log` |
| Affected UI/app all-target strict Clippy | Passed with `--no-deps`; 13 existing engine dependency warnings remain | `ui-app-clippy-final.log` |
| Final native probe strict Clippy | Passed | `probe-clippy-final.log` |
| Debug application and native probe compilation | Passed | `app-build-final.log`, `probe-build-final.log` |
| Registered debug application startup | Exit 0; five volumes and 30 entries in each fixture panel | `app-smoke-final.stdout.log`, `app-smoke-final.stderr.log`, `app-smoke-final.exit.txt`, `registration-final.log` |
| First installed archive-thumbnail probe | Failed source validation before cache creation | `logs-20261110-005353-1483/stderr.log` |
| Final installed archive-thumbnail probe | Exit 0; exact BMP pixels, visible-target restore, persistent reopen with zero member rereads, changed-source rejection/new pixels and six distinct selected-video frames passed | `logs-20261110-011212-3287/{stdout,stderr}.log`, `exit-code.txt` in that directory |

The independent archive plugin additionally passed 37 ordinary tests, with three
opt-in tests ignored, and all-target strict Clippy. Worker/native/network package
validation passed 19 Python tests, including the optional capability's type
checks. Those runs were recorded in this task's tool output; no raw log files
were retained for them. The package tests use
`python3 -m unittest scripts.tests.test_worker_packages scripts.tests.test_native_filesystem_package scripts.tests.test_network_filesystem_package`;
the plugin checks use its independent manifest and the normal pinned offline SDK
configuration with output under `_build/archive-filesystems/backend`.

The registered debug executable's SHA-256 is
`49c290ef6486d25b6a616487b87f22c65dd2e151b7bd28b379930c653fd16f36`.
The later [tree-search integration](2026-11-10-tree-prefix-search.md) rebuilt and
registered a subsequent main executable against this same protected runtime;
the hashes and probe results here describe this archive-thumbnail baseline.
The final probe uses that application's actual protected runtime at
`C:\Users\tjelinek\MidnightExplorer-20261110-html`, including its existing HTML
viewer, filesystem and thumbnail packages. The native filesystem DLL is unchanged.
Startup proves application/runtime discovery and listing. Archive-thumbnail
qualification instead comes from the separate successful installed probe, which
uses a fresh ordinary service without a preliminary direct decoder call. Its
report and decoded 2 by 2 PPM are in
`fixtures/run-31184-1794269536225338800/{report.txt,decoded.ppm}`. The first and
reopened mounts had session IDs 1 and 2; their image cache key remained
`74/74888f3fc0062adf19fc102264ea952f2bff962b65ba6bca9fa1066f14cb7a55.mtc`.
The report records zero member rereads on restoration and fresh-service reopen,
new decoded pixels after the deliberate ZIP rewrite, and selected-video samples
`{0, 1, 2, 3, 4, 5}` with six distinct decoded images. Matching executable and
probe hashes are retained in `build-hashes.json`.

The final scoped Rust formatting and whitespace checks passed. Both Windows and
Linux source-size helpers passed for 1,609 handwritten files after adding the
independent web-viewer API workspace to their coverage. The temporary archive
probe runtime and its duplicate staging directory were removed after the real
runtime passed; fixtures, logs, cache evidence and the registered application
remain available.

The initial `Captured native content changed` rejection was correct. The trace
`metadata-trace-20261110-010037-9646/metadata.json` shows unchanged primary-data
SHA-256 and LastWriteTime while ChangeTime advanced and the `sec.endpointdlp`
alternate stream appeared approximately 0.6–1.5 seconds after fixture creation.
Only stream names/lengths were enumerated; no alternate-stream contents were read.
The probe now waits at most ten seconds for its freshly authored fixture's
installed-provider revision to remain stable for two seconds, then verifies every
authored byte before qualification. It performs this fixture-only preparation
after creation and after its deliberate mutation; it does not retry a source
revision change during normal decoding. Production revision checks, read
validation and cancellation behavior were not weakened.

Review found and corrected two integration issues. Release packaging now accepts
the optional strictly boolean `stable_entry_ids` capability while retaining
false for older manifests. Archive network-policy changes replace pinned source
tokens, and retained pixels carry a boolean policy stamp. Pixels with an obsolete
stamp hide immediately, including offscreen retained images; visible snapshots
invalidate their old pixels and coverage before backend readmission. Stamps are
inserted only after pixel-budget admission and are removed by eviction and clear,
so this map cannot grow independently of retained images and holds no mount leases.
The populated-image policy-revocation regression passes in the full UI suite.
Command icons keep their native capture path and fixed physical size; ordinary
scroll, selection and resolution changes preserve unchanged source tokens.

Identity review found deterministic archive-order ID assignment in ZIP, TAR and
7z, including first-encounter implicit directories and distinct duplicate files;
raw compressed streams use a synthetic member. Hash-map iteration does not assign
IDs. Full persistent authority is hashed before entering cache record plaintext,
and captured source revision checks remain mandatory. This is source review and
unit evidence; real all-format reopen/cache qualification remains separate.

### Cancellation, work and publication limits

Thumbnail sources accept at most 256 KiB per checked read. Decoder brokering
retains its 8,192-request and 256 MiB source-read budgets; surfaces remain bounded
to 512 by 512 RGBA pixels. The service admits at most 256 visible targets, retains
two actual decoder worker slots and has a four-reply ready queue. UI retention
remains bounded to 4,096 image records and 54 MiB of pixels. Native/archive source
validation, generation/token checks and current policy checks fence publication.

The decoder's ten-second timeout is not a hard deadline for an archive source
read to return. The first selected ZIP member or solid 7z block may require full
staging and integrity verification. Archive reads deliberately drain with an
independent cancellation token to preserve the shared browsing mount. The archive
watchdog expires after 20 seconds without genuine progress; successful byte I/O
or monotonic decoded work renews it, so it is not a 20-second operation deadline.
Decompression, temporary disk use and the occupied source/I/O slot can therefore
continue after viewport cancellation or decoder timeout until the admitted read
returns. An OS call can delay cleanup further. Canceled work cannot publish pixels
or release capacity early. The real service regression in `backend-tests.log`
blocks two archive reads, cancels their view, proves both worker slots remain
occupied until the calls drain, and checks the shared mount survives.

Protocol 2 has no artificial archive-source, member or extracted-disk byte ceiling.
Full-member or solid-block verification can consume the available private TEMP
storage even for a thumbnail request; storage failure aborts staging and cannot
publish partial content. Archive worker memory, metadata and decoder-window
limits remain those in the [filesystem contract](../developer/FILESYSTEM_API.md).
The tiny ZIP/AVI fixture does not qualify large-content latency or disk capacity.

Linux execution, physical native panel scroll/selection/view-switch behavior, archive
source changes during in-flight decode, installed release qualification and
aggregate performance/budget stress require their own evidence. The final
integrated lint/rebuild and matching deployment hashes passed separately from
this installed probe. Successful backend cache assertions alone do not establish
full UI behavior, all-format reopen qualification or generic remote-provider previews.
