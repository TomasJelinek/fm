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

### 11 November MEDIA-01 follow-up design and regression sequence

Status: scoped implementation, unit/lint and installed-worker gates passed. Final
combined application verification belongs to the enclosing batch. The 10 November
evidence above is unchanged. The approved change is the protocol-3 cooperative member-staging
contract in FILESYSTEM_API.md, using independent public API packages only.
Passing a live viewport flag into the old read would kill the shared mount;
spawning a separate preview mount would consume extra process/memory capacity.
The chosen path cancels one negotiated staging request and drains its terminal
acknowledgement before releasing existing permits.

- [x] Public API: add explicit Protocol::V3 with unchanged v1/v2 bytes and v2
  streaming records, package API 0.4.0 normally, and consume the package from the
  independent archive worker. Test codec/version and package validation boundaries.
- [x] Host: add a default-compatible ArchiveMount::read_cooperative and staged
  cancellation state in session.rs; add request-scoped abandonment in staging.rs.
  RED/GREEN tests cover each staging checkpoint, wrong acknowledgements, terminal
  races, source mutation, cached/retained leases and scratch preservation.
- [x] Worker: preserve Cancelled from staging replies in broker.rs. ZIP and solid
  7z tests cancel actual preparation, assert no seal/partial publication, then list
  and prepare again on the same Archive. Include cancellation during solid sibling
  discard progress and ordinary reader Interrupted compatibility.
- [x] Thumbnail: expose a cancellation-aware Source read and pass both viewport
  and broker_stop causes. Test cooperative cancellation and legacy drain fallback,
  keeping both real worker slots occupied until read/cleanup completion.
- [x] Run public API, isolated plugin, host and thumbnail affected suites with
  pinned tools, fresh QA TEMP and explicit build roots. Root owns final combined
  build, deployment and installed-worker qualification. Record commands and real
  outcomes here; no native or Linux result is implied by unit tests.

Owned implementation files are core/archive.rs, thumbnail-host/{decode,
archive_source}.rs, filesystem-host/{session,staging,discovery}.rs and focused
new test modules; public filesystem-api wire/version/package metadata; independent
archive-worker broker/main/package metadata and tests. No UI code, native-dialog
code, third-party dependency or resource-limit relaxation was introduced.

### 11 November MEDIA-01 verification results

The explicit protocol-3 API is packaged as `midnight-filesystem-api 0.4.0`; legacy
helpers still emit protocol 1 and the protocol-2 drain path remains covered.
Only the first-party package version/checksum changes in the independent worker
lockfile. The normal SDK exporter created a new immutable bundle at
`_build/media-cancel/sdk/worker-sdk-1794390611415302600/`; the earlier pinned bundle
was not changed. The plugin still depends only on packaged public APIs.

The initial Windows RED gates observed two missing protocol-3 codec failures,
four shared-mount/cache-retention failures, two dropped cooperative-source
forwarding failures and one lost worker cancellation error. Ordinary ZIP/solid-7z
unwind/retry controls were already green. The resulting scoped gates are:

| Gate | Result |
| --- | --- |
| Public API full tests (`--manifest-path filesystem-api/Cargo.toml`) | 13 passed |
| Host + thumbnail all targets | 78 passed, 20 installed/benchmark opt-ins ignored |
| Host terminal/watchdog follow-up | 9 cooperative tests passed, including two additional checks |
| Independent archive plugin all targets | 41 passed, 4 fixture/measurement opt-ins ignored |
| Host/thumbnail and independent plugin strict all-target Clippy | Passed |
| Python worker-package validation | 8 passed, 1 existing skipped |
| Installed image helper: cooperative viewport/timeout, queued deadline, pixel control | 3 passed |
| Installed archive package: cooperative ZIP/7z, all formats, hard cancel, pathless source | 4 passed |
| Linux core/archive host/thumbnail/API unit gates | 51 / (26 + 2 follow-up) / 25 / 13 passed |

The final Linux host rerun passed all nine cooperative tests, including the two
new terminal/watchdog cases. Counting those two additions once gives 117 unique
passing Linux tests across the scoped gates. The host had one installed opt-in
ignored and thumbnail had five. Its first
thumbnail fixture attempt used checkout `_build` on DrvFS, which cannot satisfy
the private-cache mode-0700 contract. Repeating with the documented
`MIDNIGHT_NATIVE_TEST_ROOT=$TMPDIR` on private ext4 passed; production checks and
permissions were unchanged. These are unit and decoder-worker checks, not a Linux
desktop or native panel latency claim.

Windows commands used pinned Rust, `--locked --offline`, a fresh process-local
TEMP/TMP, host target `_build`, and independent plugin target
`_build/archive-filesystems/backend`. Exact command orchestration and logs are in
`_build/qa/media-cancel-20261111/windows-green.ps1`, `api-{red,green}.log`,
`host-thumbnail-{red,green,clippy}.log`, `host-terminal-{green,clippy}.log`,
`plugin-{red,green,clippy,package}.log`, `package-{red,green}.log`, and
`installed-decoder-{cancellation,pixels}.log`. Linux logs are
`_build/qa/native-operation-windows/linux-media-unit.log`,
`linux-media-thumbnail-final.log`, `linux-media-api.log`, and
`linux-media-host-final.log`.

The release archive package is `_build/media-cancel/package`, advertises protocol
3, and includes normal third-party notices. Its executable SHA256 is
`92EF3B1C52E945A28DDADC278373173FC69955866E8211D9AF7FBA172DD94C37`.
The same hash was verified after normal private deployment to
`C:/Users/tjelinek/MidnightExplorer-20261111-media-v3-92ef3b1c/filesystems/archive/`.
This unregistered qualification runtime retains the previous application EXE; only
the new host test executables ran against its installed workers.

`installed.ps1` sets `ARCHIVE_CAPTURE_INSTALLATION` to that private runtime and
`MIDNIGHT_ARCHIVE_FIXTURES=_build/media-cancel/fixtures`. The exported Stored-ZIP
and solid Copy-7z cases each cancel after 512 KiB of additional source reads and
assert that the complete 2 MiB payload was not drained. Both retain an unrelated
cached member without another source read, preserve the directory identities, and
successfully retry the canceled member on the same mount. A separate all-format
case validates 11 formats and rejects corrupt ZIP/7z fixtures; hard cancellation
still poisons its session, and captured pathless content retains exact authority.
Logs: `installed-cooperative-archive.log`, `installed-all-formats.log`,
`installed-hard-cancel.log` and `installed-captured-source.log` under the same QA
directory. These checks establish cooperative protocol behavior and bounded work
at existing checkpoints; they do not promise cancellation during an active OS call
or qualify native panel frame pacing.