# Media follow-up — 19 November 2026

The active checkout remains `codex/post-merge-validation-20261118`, based on
`544b48db`. This checkpoint supersedes the earlier staged-only Media status.
The verified boundaries now include compressed VP9/Opus decode through the
restricted worker SDK and a separate silence-only native audio endpoint test.
Broader installed playback and release qualification remain open. No merge
to main or overall completion is claimed.

## Integrated source

Root applied the reviewed 64-file Media reconciliation patch with SHA-256
`6d8d14f74fe756ddf54a7145647dca8bff15230393a17c6444216ad37ffad622` and
verified all 64 resulting hashes against the proposal immediately after applying
it. Subsequent fixes are later source changes. Reconciliation inventory and
isolated replay remain under `_build/media-v3-reconciliation/`; its
`verification.json` describes the earlier isolated replay, not active-source
application or runtime acceptance.

The integrated v3/r3 host/worker wire uses separate video/PCM credits and stereo
signed 16-bit PCM. An older r1 worker with shared credit and F32LE PCM cannot
qualify this host. The matching r3 worker is now built and tested within the
scopes below.

EOF handling preserves the final frame and remaining playback interval. A
persistent seen-frame flag distinguishes an initially empty stream from a
completed preview; initial empty EOF reports an error. Empty seek completion
resets pending PCM and can restart through a real seek, including when paused;
ordinary pause keeps its existing behavior. The implemented UI lifecycle work
uses a guaranteed retirement mailbox slot and exact-session pixel/control
fences. Media fitting preserves aspect while paused and refits on resize;
control synchronization fixes also address the Slint binding loop.

Root subsequently reviewed and applied the 17-file media frame-fence patch,
SHA-256 `129cbe85011d9c813a7d92d98eb49bc5ecc05824fe5827288789a655f8588b76`,
checking every preimage and resulting hash against
`_build/seek-frame-fence/manifest.json`. Video frames now carry an optional typed
applied `MediaControlId`. UI delivery requires the exact active session and a
control between its latest accepted content-changing intent and latest accepted
command. Accepted seeks and observed Ended-to-Play replay advance that floor;
ordinary play/pause and refused submissions do not. A pending seek is not
misclassified as replay because an older Ended snapshot remains visible. The
same fence runs before prepared-buffer replacement or copy-error publication.
Non-video frames carry no media control; missing/future video controls fail
closed. The independent plug-in wire format is unchanged.

## Host, output, Engine and UI evidence

| Boundary | Latest result | Evidence under `_build/media-session-eof/` |
| --- | --- | --- |
| Media host library | 34 passed, 0 failed | `empty-green.jsonl` |
| Media output, including actual default endpoint | 6 passed, 0 failed, 0 ignored | `audio-clock-green.jsonl` |
| Engine video cases | 7 passed, 0 failed | `engine-video.log` |
| Engine retirement mailbox | 1 passed, 0 failed | `engine-retirement.log` |
| Viewer routing, including video/process indices and user policy | 12 passed, 0 failed | `video-routing.jsonl` |
| UI control synchronization | 4 passed, 0 failed | `ui-controls-final.jsonl` |
| UI media controls | 3 passed, 0 failed | `ui-media-controls.log` |
| UI paused-video aspect/resize | 1 passed, 0 failed | `aspect-green.jsonl` |
| UI navigation/timeline | 2 passed, 0 failed | `ui-media-navigation.log` |
| Backend strict Clippy, two packages | Successful | `backend-clippy.jsonl` and stderr |
| Integrated strict Clippy, eight packages/all targets, warnings denied | Successful, zero diagnostics | `clippy-replay.jsonl` and `clippy-replay.stderr.log` |
| Plug-in Options replay | 17 passed, 0 failed | `plugin-options-replay.jsonl` |
| Prepared media frame fence and direct image/UI boundaries | 31 passed, 0 failed | `seek-pixels-final.jsonl` |
| Applied media control producer/routing boundaries | 8 passed, 0 failed | `seek-producer-green.jsonl` |
| Windows backend policy | 1 parent test passed; covers 12 isolated backend/flag configurations | `renderer-policy-green.jsonl` |
| Frame/renderer strict Clippy, four targeted packages/all targets, `--no-deps`, warnings denied | Successful, exit 0 | `frame-renderer-clippy.jsonl` and stderr |

The earlier UI groups contain **10 unique cases** in their Media checkpoint; the Web
policy and mode cases also overlap earlier UI evidence. Engine's seven video
cases and one retirement case are eight distinct cases. These scoped counts
are not added to earlier Find/main-UI totals or repeated runs. The routing
group is recorded separately rather than added to those totals. The later
31-case UI replay includes five control-sync, 21 image-presentation and five
media-control cases. It overlaps earlier UI evidence and is not added to it.
Likewise the later eight-case producer replay overlaps earlier Engine and
routing groups; it is not eight additional distinct cases.

The final control synchronization run includes actual physical Ctrl+F with the
normal default `ShortcutMap`: Find remains blocked while media is active and
opens after retirement. Fixtures require exact `LayoutReady` and a positive
canvas before publishing frames. Earlier control failures from missing fixture
phases and a `None` key resolver remain in the earlier receipts; the corrected
final four-case result supersedes those failures.

The final plug-in Options replay passes all 17 cases. Root visually reviewed
the seven-viewer captures and the expanded 20-row catalog: all seven bundled
viewers fit at the tested normal sizes, and keyboard navigation reveals the
last expanded-catalog row above the pinned Rescan action without an inner
catalog scroll cap. The tests exercise the actual Presenter catalog, visible
validation feedback, editable thumbnailer extensions, draft retention and Reset.
The earlier `plugin-options.jsonl` **15 passed, 2 failed** and subsequent borrow
compilation failures remain retained. Their stale catalog-error/description
assertions were replaced with current visible extension validation and editable
field checks; no production behavior was weakened.

### Retained failures and their resolution

- Initial EOF RED: **3 passed, 4 failed** in `red.jsonl`; the first correction
  passed 31 host cases in `green.jsonl`. Later empty-stream/seek checks exposed
  two failures in `empty-red.jsonl`, then three in `empty-pcm-red.jsonl`
  (**7 passed, 3 failed**). The final host run passed all 34 cases. These runs
  overlap rather than establish additional coverage.
- The original default output run passed two cases and ignored the silence-only
  native endpoint case. Explicitly running that case failed in `native-audio.log`
  and `native-audio.stderr.log`. Numeric diagnostics exposed a valid zero-latency
  stream: frequency 192,000, capacity 2,400 frames, latency 0. The dedicated
  regression in `audio-clock-red.jsonl` passed three and failed one.
  The fix accepts zero latency while rejecting negative latency, zero frequency
  and conversion overflow. The final six-case output run includes the actual
  native endpoint and passes with no ignored cases. The earlier native failure
  is historical, not the current blocker; silence-only output does not qualify
  audible video playback or whole-application A/V synchronization.
- The aspect RED in `aspect-red.jsonl` showed portrait video stretched to
  888×496 in a 900×600 window. Its corrected paused-video resize case passes in
  `aspect-green.jsonl`.
- `seek-pixels-red.jsonl` reproduced a real prepared-batch race: after a newer
  accepted seek, frame 11 from the earlier seek was published instead of
  retaining frame 10. The typed fence correction also checks missing/future
  controls, stale conversion errors, refused seeks, replay and the sole sought
  preview surviving later Play/Pause. The first wider replay,
  `seek-pixels-green.jsonl`, passed 29 cases and failed two existing Runtime
  fixture preconditions: the normal window-manager guard had already closed
  the session intended to remain live until Runtime cleanup. The fixtures now
  clear only their manager registry after hiding the window and retaining the
  session. They retain weak-handle checks, the live-palette precondition,
  post-Runtime closed checks and the original panic assertion. The final replay
  passes all 31 cases; the earlier failures remain historical evidence.
- `renderer-policy-red.jsonl` failed on the absent-override default, observing
  DX12 where VULKAN | DX12 was required. The minimal production mask correction
  then passes its one parent test and all 12 subprocess configurations: absent,
  DX12, Vulkan and combined overrides, each with absent/off/on debug-validation
  flags. Indirect-call validation remains forced on even when its environment
  value is 0. These are policy checks, not twelve native GPU acceptance tests.

## Python tooling checks

| Group | Passed | Skipped | Discovered |
| --- | --- | --- | --- |
| `test_default_thumbnailers.py` | 2 | 13 | 15 |
| `test_installed_video_package.py` | 1 | 0 | 1 |
| `test_prepare_thumbnail_ffmpeg.py` | 11 | 5 | 16 |
| Total | 14 | 18 | 32 |

Evidence is `_build/media-session-eof/test_*.stderr.log`, with companion stdout
logs. Skips include Linux-only paths, Windows symlink privileges and the GPG
release-verification prerequisite; skips are not passes. The installed-package
Python case checks the companion-executable/resampler profile, not native
playback.

## Matching r3 worker and compressed SDK proof

The parent checkout's `_build/qa/media-playback-r3/handoff.json` consolidates
**45 distinct functional passes**, zero final failures and zero ignored cases:
decode 4, playback 10, library 27 and playback-worker 4. Its referenced final
library and retained independent results preserve the scope. Initial missing
fixture fields and stale MMV1 test headers were repaired in tests; earlier
37-pass/8-fail results are retained, not added to the final count. Both worker
helpers have their import closures checked against the five recorded FFmpeg
DLLs: avcodec, avformat, avutil, swresample and swscale.

`_build/qa/media-playback-r3/compressed-handoff.json` separately consolidates
one passing compressed native SDK scenario. It uses actual trusted
`VideoInstallation`/`VideoProcess`, SDK-ready source capture and a restricted
worker with the Chromium VP9/Opus WebM fixture. Results:

| Decode phase | VP9 RGBA frames | Opus PCM frames | Nonzero PCM samples | EOF |
| --- | --- | --- | --- | --- |
| Before seek | 82 | 131,328 | 260,072 | Yes |
| After 1-second seek | 52 | 83,328 | 164,131 | Yes |

The seek changes epoch 1→2; the first subsequent video PTS is 1,001,000 µs.
Retirement is confirmed with no owned processes remaining. The runtime module
check verifies the loaded paths and SHA-256 of all five FFmpeg DLLs. Evidence:
`compressed-run-20261119-attempt2/native-terminal.json` and
`verified-result.json` beneath the same QA directory.

The first attempt correctly rejected a QA Windows descriptor lacking the
production builder's `.exe` normalization. Only that QA descriptor was repaired;
no production trust guard was weakened. The fixture and its BSD-3-Clause
provenance/license records remain under QA in
`fixtures/chromium-86a73f2bf8d3/`; they are not added to the application package.
After confirmed retirement, `cleanup.json` records removal of the owned C-drive
package, a 28-file D-drive mirror, unchanged SHA/ACL guards and no changes to the
normal runtime.

The native FFmpeg profile still has **54 warnings** and remains explicitly
unqualified. The DLLs were not rebuilt by these checks. The compressed SDK proof
qualifies its restricted decode/seek/EOF/retirement scenario, not installed F3
playback, audio-device playback through F3, native MP4 or Linux.

## HTML controls and binary scope

The earlier trace-OFF opening failure remains recorded in the parent checkout's
`_build/qa/html-native-probe-win-v1/timeline-control-hidden-20261119-001112-8cddf5ec/`;
ON did not run and that control qualified no timing runs. The subsequent
`diagnostic-probe-hidden-20261119-000236-6e8658ad/diagnosis.json` records the same
transport error and CEF's explicit path-creation error, **0xCE**, with a
232-character session path. Shortening only the owned profile/temp prefix then
allowed the same diagnostic binary to complete. No trust or sandbox guard was
weakened. Successful CEF session profiles self-delete, so retained post-cleanup
paths do not measure their peak live length.

Four earlier successful controls completed **eight native runs**, all exit 0. Each pair used
one pinned binary and the same real 5,296-byte document, SHA-256
`7c8fc8ecb134103d5aa4480114b83ecd3f4221bdb26e834fa4bcc94d3f0da515`.
Each run covered first open, reopen, 12 individual wheels and a 24-post burst;
all 24 burst inputs were received. Fresh short profiles were used. The two
backend comparisons record a 1377×477 physical canvas at scale 2.25.
Receipts below are `handoff.json` and `analysis.json` beneath the parent
checkout's `_build/qa/html-native-probe-win-v1/`:

| Control directory | Verified result and scope |
| --- | --- |
| `short-profile-control-20261119-000942-821911f6/` | OFF→ON completed. ON retained 4,008 timeline rows with none dropped. Pinned Slint adds a clear flush/submission when the notifier is installed, so ON changes the rendering path. |
| `basic-profile-control-20261119-002747-96451956/` | OFF→BASIC completed. BASIC retained 3,522 rows, none dropped, and zero rendering-notifier events. Its maximum redraw wall span was 766.157 ms, so long redraw intervals also occur without the notifier. |
| `software-dx12-off-control-20261119-003231-ff4a3ba6/` | Both traces/notifiers OFF. Maximum publication→draw was 21.736 ms for software versus 224.389 ms for DX12; maximum changed-draw burst gap was 42.262 versus 780.888 ms. Software still had a 461.366 ms publication delay. |
| `vulkan-dx12-off-control-20261119-010253-5b05703f/` | Both traces/notifiers OFF. Maximum publication→draw was **18.398 ms for Vulkan versus 562.788 ms for DX12**; maximum changed-draw burst gap was 68.391 versus 528.250 ms. Both opens in both runs confirmed FemtoVG-WGPU and the respective single-backend mask, with no software fallback. Vulkan still had a 459.625 ms publication delay. |
| `combined-default-control-20261119-013848-d028fa54/` | Two native runs passed: combined-default OFF timing, then separate ON adapter identity. OFF first/reopen draw was **2125.929 / 1792.499 ms**; wheel publication→draw maximum **18.909 ms**, with a real upstream publication delay of **498.268 ms**. ON directly observed Vulkan in both opens, but installs the notifier and is timing-ineligible. |

The binaries differ between probe generations; results are paired only within
each control:

- Short OFF/ON binary:
  `fb5b39963aea65b49513a7db9d4f8cb4ce0978dee19d3ea08b4ccf6555c3bc22`.
- BASIC and software/DX12 binary:
  `bc06985e45fdb4f1d8e4e362587c6ba8574feb4672af3b95f01a7a305726ddc6`;
  host HTML probe source `a9923ccd929770c374733f4358c86394bb8d0d427b16843eeac40b1b72c0aa9a`.
- Vulkan/DX12 binary:
  `c03ef15e3c7191d62319253510b3cbe831fae630e2d2a7638fa8a50bdf35579e`;
  host HTML probe source `fd0c88953bfd6dda223e2acd3e0b58cd96cc8219c8cf368cac6cf336ac2675ab`.
- Combined-default OFF/ON binary:
  `5caa539ebd7ce4060dedc30c6de034c5fc25c0bbc433a10741fc5c630acbf8af`;
  host HTML probe source `6ad6008b1c60808a55f5ec432f567886b0aeac96aa93162823a37761db6e14d4`.
  These bytes precede the later media frame fence; this is not evidence for the
  latest overall application binary.

Host/probe provenance does not identify the HTML provider. Build confirmed
that the single-backend and combined-default controls executed the **baseline**
runtime `%USERPROFILE%/MEHtmlProbe-96673981`, whose 803,328-byte provider has
SHA-256 `9159cf312869754f9054e97a83889edefeaf1dc77a77d00f810730731879d669`.
The candidate `%USERPROFILE%/MEHtmlProbe-e1593972`, with an 851,968-byte
provider identified as `4a1bdd…3787`, was guarded but not executed in those
controls. The
executed-provider provenance receipt
records that distinction; the
combined-control analysis
records the baseline executable/runtime path. Accordingly, source `6ad6008b…`
and binary `5caa539e…` identify the measured host/probe, **not current HTML
provider source**. The approximately two-second opening and 498.268 ms upstream
delay are fixed-baseline-provider observations, not exact-current-provider
qualification. This preserves the controlled host-renderer comparison while
leaving provider-source acceptance open at that checkpoint. The later
current-provider control below supplies separate, bounded evidence.

The completed pairs support a host-renderer-path contribution to the observed
latency tails on this machine; they do not isolate a particular GPU wait or
establish an exclusive cause, general speedup or FPS qualification. Each is
one order-fixed pair. Publication precedes physical display; the draw callback
runs at the next observer/event-loop boundary. `AfterRendering` precedes
surface presentation. Redraw wall spans include rendering, presentation,
scheduling and other work rather than measuring CPU busy time. Burst input
span includes synthetic-post pacing. Visible/non-minimized samples and absent
occlusion events do not establish compositor visibility. No exact CEF-paint to
host-publication join or GPU/device timing was captured.

The receipts preserve all 534 original runtime rows for each protected runtime
and unchanged executable/ACL guards. Owned siblings were removed, no owned
processes remained and sampled compiler counts were zero. The latest pair also
records configured absence of `WGPU_ADAPTER_NAME` and `SLINT_WGPU_CPU`; parent,
user and system environment values were not changed.

The Windows default mask **VULKAN | DX12** is implemented and its scoped policy
checks pass. Explicit `WGPU_BACKEND` overrides and existing validation flags are
preserved. Pinned WGPU prefers Vulkan
for equal adapter preference ranks and retains DX12 availability when Vulkan
initialization or surface compatibility fails; this is not device-creation or
runtime-error retry. The combined-default OFF run confirms FemtoVG-WGPU with
the requested combined mask and no notifier; it does **not** directly identify
the selected adapter backend. Vulkan identity belongs only to the separate ON
run and cannot be transferred to OFF as proof of its selection. ON retained
9,555 host timeline rows with none dropped. The latest receipt verifies all
534 original rows for each protected runtime, unchanged executable/hash/size/
mtime/attributes/ACL and root-ACL guards, stable fixture/source, removed owned
siblings and no remaining owned processes. All 18 sampled compiler counts were
zero; each actual run window has six samples. No normal runtime or ambient
parent/user/system overrides were changed.

The 498.268 ms interval is an observed enqueue-to-publication interval, not an
artificial settle timeout. The later probe-ordering audit below supersedes its
interpretation as per-input response latency. It remains upstream of the approximately 17–19 ms
wheel publication-to-draw interval and is unresolved. The exact fixture's first
window construction completed in 245.029 ms and reopen in 224.296 ms; first
nonblank document publication was 2097.052 / 1762.674 ms. These observations do
not yet attribute the remaining opening time to package checks, host work,
process startup or CEF.

### Current normal-provider control

The subsequent
candidate OFF handoff
records one successful native run using the 851,968-byte current provider
`4a1bdd85832e4c9df5d4ec514feacdcb375d09fae67d35c3c8421fab51a73787`.
Its configuration-equivalence receipt
matches all 49 source pins, all three SDK pins and the exact CEF receipt/archive.
Features are normal `default`/`native-cef`, without `diagnostic-timing`; the dev
profile uses optimization level 2 and no debug information. The host remains
the preserved `5caa539e…` probe, not the latest overall application binary.

| Measurement | First open | Reopen |
| --- | --- | --- |
| Window construction | 767.594 ms | 206.970 ms |
| First document publication | 1574.717 ms | 1134.192 ms |
| First document draw callback | 1589.258 ms | 1165.933 ms |

Across 12 individual wheels, the historical enqueue→publication median/max is
**46.866 / 538.527 ms**, while publication→draw max is **18.768 ms**.
The burst receives all **24/24** posted inputs. The combined requested backend
mask and FemtoVG-WGPU are confirmed; OFF does not directly identify the selected
adapter backend. No notifier or timeline is installed. All nine sampled
compiler counts are zero, including five inside the run. Both protected
534-row packages and executable/security guards remain equal, owned siblings
are removed and no owned processes remain.

This reproduces the publication tail with the normal active-source provider,
separately from the earlier old-baseline observation. It is one control, not a
randomized or contemporaneous before/after comparison, a generalized speedup,
physical-display timing or a cause attribution. Publication remains upstream
of the host draw callback; no exact CEF-paint/pixel join was captured here.

### Font microprobe and causal diagnostic status

The font-cost decision
retains one successful five-sample, one-process QA microprobe. The first
`TextRenderer` constructor takes **1.861 ms**, followed by 0.319, 0.288, 0.278
and 0.266 ms. First valid layout takes **90.959 ms**, then 0.635, 0.601, 0.540
and 0.565 ms. These measurements do not support attributing seconds of HTML
opening latency to eager constructor work, so **no lazy-font refactor** was made.
OS/font caches may already be warm; destruction and console output are outside
the measured intervals. The **2862.328 ms** enclosing process wall interval
contains unattributed time outside construction/layout. It establishes no
Drop, OS, antivirus or other explanation and is not a whole-viewer measurement.

One active-source `diagnostic-timing` provider/BASIC host run has now executed.
The lease-release receipt
confirms causal capture completed, the reserved `MEHtmlProbe-0884f470` runtime's
original 548-file inventory was restored and checked, both protected runtimes
remain unchanged, the owned host sibling was removed and no owned process
remains. Bytes, LastWriteTime, attributes, file IDs and ACLs were restored;
NTFS ChangeTime advanced and is recorded separately. The
completed first causal handoff
links the independent wheel/native-gap and pinned CEF thread-contract analyses.
Two slow wheels dispatch in 0.033/0.019 ms and return from their matching initial
command/pump batch in 0.107/0.138 ms, yet the next paint marker arrives
516.670/478.722 ms later. Host timers continue through both intervals. The
first trace's PaintBegin marker follows its surface lock, but the source/thread
audit places every ordinary surface owner on the same provider CEF UI thread;
it does not support an independently released cross-thread lock explaining the
delay. These markers do not measure the later idle pumps; the sparse follow-up
below adds that boundary.

Opening is a separate concern: the first causal trace retains a 2127.807 ms
non-frame batch wait before UI delivery and a large reopen native-loop gap.
The saved ON lifecycle analysis
uses earlier notifier-altered streams to partition their own startup intervals:
combined-ON custom-about-wait→RenderingSetup takes 232.381/733.551 ms for
first/reopen, followed by 2.431/4.189 ms to BeforeRendering and
23.511/38.735 ms to AfterRendering. Setup occurs after resource initialization
and surface acquisition; it is not the beginning of GPU initialization. The
combined Setup marker also follows an unmeasured identity log write. The
notifier adds clear submission work, and AfterRendering precedes presentation.
These older-provider ON intervals do not attribute the later BASIC gaps or
qualify normal OFF timing.

### Sparse pump follow-up and selected next control

The diagnostic/model handoff
records **19 standalone tests passed**, retaining their expected RED failures.
These exercise the recorder/model rather than the complete native provider.
The full native gate
then records successful diagnostic-feature strict Clippy and a fresh build with
zero warnings. The initial strict-Clippy failure exposed an existing navigation
lazy-evaluation lint; its narrow semantic-equivalence correction and successful
replay remain recorded. Source/SDK/CEF guards hold. This gate covers the
50-file diagnostic source snapshot; normal-feature preservation uses the
recorded conditional-hunk reversal and reviewed lint fix, not a new complete
normal-feature test run.

The sparse coverage receipt
retains first/reopen CSVs with 19/1943 emitted rows. All 12 isolated wheel rounds
have complete linked summaries. The 24-post burst contributes 15 further
summaries; nine wheels have no individual group under latest-wheel
replacement/coalescing. The producer has no dropped-row counter: a row count
below the 2048 cap alone does not prove zero omitted summaries. The host retains
11,389 timeline rows with zero reported drops and no rendering notifier.

In the independently checked slow isolated round, sequence 83 takes
**507.466 ms** from wheel to saved pre-lock valid-paint callback entry while
**34 pumps** start, with maximum start-to-start gap **16.692 ms**, maximum
completed pump work **0.355 ms**, completed navigation **0.003 ms** and current
pump age **0.146 ms** at entry. No earlier paint marker appears in that window.
This narrows that observation beyond an unmeasured idle-pump stall; it does not
identify renderer, compositor, Viz capture, driver or OS scheduling as the
underlying cause, or acknowledge exact wheel-derived pixels.

The independent sparse-pump analysis
retains the raw-line checks, cadence/animation distinctions and pinned-source
limits for that observation.

The sparse-run restoration receipt
confirms the original reserved runtime's 548 files, protected packages,
executable metadata/security and fixture guards, removed owned sibling and no
remaining owned processes. ChangeTime advances are separate from restored
bytes/LastWriteTime/attributes/file IDs/ACLs.

The Architect decision
selects a bounded diagnostic comparison of CEF's `disable-gpu-compositing`
path. The earlier env-toggle handoff
retains **20 standalone tests passed** after the expected selector RED failure.
Its diagnostic native-provider strict Clippy/build and default cfg-off check
also passed. That configured A/B was **never staged or run**: the Windows
worker environment allowlist at lines 162–173 and Linux `env_clear` strip
`MIDNIGHT_HTML_DIAGNOSTIC_SOFTWARE_COMPOSITING` before the provider receives it.
The Build stop receipt
preserves the unexecuted packet and source-boundary finding. These compiler/unit
passes do not qualify the env control as an effective native A/B.

The replacement compile-feature handoff
freezes four source files and records **19 standalone tests passed**, retaining
the pump/recorder tests while removing only the obsolete env selector/test/hook.
The comparison now requires two provider binaries built from the same frozen
source: A uses `diagnostic-timing`; B uses the nondefault
`diagnostic-software-compositing` feature, which enables diagnostics and appends
only the fixed CEF switch. The host binary, Slint backend, real fixture,
geometry, policies and 12-wheel/24-post checks remain common. No SDK environment
allowlist or security boundary was widened, and normal defaults are unchanged.

After successful CEF initialization, `compositing_configured` records the
compiled feature and actual read-only global CEF switch value. Every first-open
and reopen CSV must show A `0/0` or B `1/1`; missing/invalid global state is `2`
and fails the gate, as does a mismatch. This observes accepted configuration,
not the internal GPU execution branch. The
completed feature-pair handoff
now records successful strict builds and one native A0/B1 pair. Both first-open
and reopen gates show exactly A `0/0` and B `1/1`. All protected-runtime guards
and original 548/534-file restoration checks pass; B1's separately recorded
owned-D profile residue cleanup is not automatic SDK retirement acceptance.

Pinned CEF source selects a different offscreen capture path when GPU
composition is disabled. A synthetic 500 ms wheel-end timer exists, but CEF
routes the current wheel immediately after phase handling; the timer's
numerical similarity is not a demonstrated paint-delay cause.

The independent A/B chronology
retains A0's 545.928 ms maximum enqueue→publication interval and B1's 459–579 ms
long-draw observations plus 531.957 ms burst gap. B1's approximately 0.1 ms odd
rounds are invalid input-response measurements: their credited publications
precede native delivery of the associated wheel. The next round also started
immediately after a blocked redraw because the old settle rule considered only
publication age. The valid chronology places long B1 gaps after the custom
handler returns and before the next event-loop observation; it does not identify
a GPU driver or individual Slint rendering call. B1's provider reaches its
2048-row cap, leaving the last three burst wheels uncovered; the host has zero
reported drops. Root rejected a production software-compositing default as an
unjustified net regression. These are operational observations, not exact
input-to-pixel acknowledgments or a general performance comparison. No
smooth-scroll switch, invalidation policy or loop rewrite was selected.

### Corrected probe and raster-only controls

The probe correction handoff
retains the meaningful RED (**5 failed, 1 positive passed**) and GREEN
(**9 passed, 0 failed**) example tests, plus successful example check/build with
zero compiler diagnostics. Publications and redraw credit now require native
wheel delivery to the current viewer, and a pre-delivery publication remains
excluded if its redraw returns later. The next isolated round waits 300 ms from
both the latest changed publication and observed changed-redraw return, with no
undrawn publication; burst first-draw/settle logic has the same ordering guard.
The corrected `94539ab2…` probe has **not received a native timing rerun**.
These guards exclude impossible attribution; they do not establish an input/pixel
acknowledgment or retroactively repair historical measurements.

The separate raster-only native handoff
records three successful local processes: unchanged image plus request-redraw,
alternating precreated CPU images, and fresh RGBA buffers. Each retains two
warmups and twelve measured serials, at identical 1440×720 window / 1377×477
raster geometry and scale 2.25, with zero trace drops and retired owned processes.
The binary is `a6f740c5…`, source `3fe6a507…`; check, strict Clippy and build pass.
All request Vulkan and confirm the FemtoVG-WGPU class; the actual adapter/backend
is unobserved. The maximum redraw-observer remainder is 398.195 / 401.841 /
374.680 ms respectively. Even the unchanged-image control retains a long
remainder, so new raster replacement is not required for that observation.
These are single sequential operational passes without input, CEF, plug-ins or
a rendering notifier. Precreated CPU handles are not resident GPU textures;
observer return is not physical display completion. No cause, speedup or
production change follows from these three passes.

## Current native filesystem prerequisite

The current native-filesystem handoff
records **150 passed, 0 failed, 2 ignored**: ABI/SDK 90 passes plus one ignored,
and native Windows plug-in 60 passes plus one ignored. The loaded DLL is the
fresh 862,208-byte compiler artifact, SHA-256
`2200e210375747e8ec3442116dbd6a0f3f6b8537c7e1fb16733274d9c972cbdc`,
preserved with its package under the QA directory. Its
frozen source closure
and DLL-specific compiler dependency paths identify the active checkout and
remain stable after tests; the stale packaged `target/debug` DLL was not reused.
The actual loaded-DLL case checks descriptor-buffer ownership and selected-file
content through the public ABI. The two ignores require a caller-selected cloud
file and an intentionally disabled timing harness; neither is a pass.

Compilation has zero warnings/errors, no desktop runs occurred, no user runtime
or settings were changed and owned fixtures retired. This prerequisite does
**not** qualify installed trust-catalog acceptance or whole-application F3.

## Current application build

`current-app-build.jsonl` records the earlier successful frame-fence `me.exe` compile.
Root confirmed wrapper exit 0 and the 115,517,440-byte artifact at
`_build/merge-main-20261118/_build/merge-explorer-check-v1/target/debug/me.exe`,
SHA-256 `1fa0c8d8d78635dc08cc9650d9a8fedacd8346ba9c4eb8461f50647ff962b109`.
It includes the frame fence and Runtime fixture repair. This does not qualify,
deploy or launch that executable; private end-to-end, packaged/installed F3 and
publishing acceptance remain open.

The earlier accessibility-enabled QA build
passes with zero compiler diagnostics. Its preserved 116,560,896-byte
`me-accessibility.exe` has SHA-256
`27dd0c655c2340dbfa3640242272b673969a3fa26eeb82084c3869152b2f1898`.
The actual compiled-feature proof
confirms all five required Slint/core/backend-selector/Winit/Parley accessibility
features and all four Windows AccessKit packages. The earlier `1fa0c8d8…` artifact
is retained. This is a QA build, with no normal-runtime promotion or F3 result.

The later normal app/HTML-probe build
passes (exit 0); its artifact records
identify normal `me.exe` SHA-256
`981b85838b11b46cc64c088d31b30314e08a96158b50389f1f6ac56b921eddff`
(116,562,432 bytes) and HTML probe
`17bd47318b70ea0d63484b4dc10805bca718a8d6740758618c138ff0a1523891`
(38,178,304 bytes), both `profile.test=false`. The subsequent palette-fix build
below replaces only the normal app; the later resize-fix build supersedes it again.
These builds supply neither normal-runtime promotion nor native timing/playback
qualification.

## F3 attempt and accessibility blocker

The corrected F3 attempt
fails before any F3 key or playback action. All eleven UI Automation queries
qualified the correct private desktop and returned empty row lists; the exact
fixture could not be selected. The current host/native-filesystem paths and
hashes qualified, but playback-worker/F3 acceptance did not. The normal process
retired, the private desktop closed and the owned child was removed; protected
runtime/preferences/security guards passed, with no retry. The earlier helper
failure remains retained rather than counted as a playback test.

The compiled-feature audit
confirms that the actual `1fa0c8d8…` application compiled Slint/core/Winit without
`accessibility` or `accessibility-text`. This identifies a real accessibility
configuration blocker, rather than evidence of successful F3 playback. Enabling
Slint's existing default accessibility feature is now approved; its eighteen
additional pins and exact notices are tracked in the [dependency inventory](../THIRD_PARTY.md).
The corrected QA application build now exists as `27dd0c65…` above. The separate
native canary below supplies a scoped accessibility/capture prerequisite; it
does not establish normal-app playback. R3v2's later stage fails before writes
because the reserved `0884` runtime is already absent; no R3 native run occurs.
The older `0884`/`966`/`e159` roots were observed absent before R4, without any
cleanup attribution to this task. The subsequent authorized fresh R4 stage,
failed detection and verified cleanup are recorded below. No normal-runtime
promotion, completed F3 playback, audible A/V or installed acceptance is claimed.

## Native accessibility and capture canary

The first native terminal receipt
and actual canary result
record **PASS, exit 0, 8.36 seconds** under the normal nonadministrator owner.
This executes the separate `native_accessibility.exe` (`22862565…`) using
**Winit/software**, a private desktop and a synthetic 100,000-row model; it is
not the normal application or default WGPU renderer. The actual provider reports
43 raw UIA nodes before and after the finite scripted phases. Plain Value/Text
patterns return the positive control. Password nodes report `IsPassword=true`
and bullets; the queried attributes and Value/Text descendant data contain none
of the private-use test codepoints U+E100–U+E107.

The model counters
retain **57 unique rows in each of three phases**, within the 232-row envelope
for 19 visible rows. Calls are 76 in the cold phase and 57/57 afterward; each
phase reaches row 0 and row 99,998. Both recorded activation barriers read zero
model rows. The UIA helper exits before the final phase; this does not imply
that the provider returns to an inactive state or that no other OS/client
activation occurs. Both owned processes retire with exit 0, the desktop closes,
and thread desktop/DPI context are restored; no global input, physical-desktop
switch or forced stop occurs.

Capture is qualified per method, not merely by API success. `WM_PRINTCLIENT`
returns API success but all nine sampled pixels are black: **that method fails**.
`OWNED_CLIENT_GETDC_BITBLT` returns the exact RGB **17/83/149** at all nine points
of the owned 180×135 physical-pixel canary at **216 DPI / 2.25 scale**, with
matched window/process/desktop/DPI proofs. Root visually reviewed that crop.
This qualifies that owned-client canary method, not physical display, normal-WGPU
capture or F3 video playback.

The earlier example check passes in 303.61 seconds. The feature-aligned full
App/UI build passes in 33.74 seconds and reuses UI bindings (`fresh=true`);
two owned feature-mismatched continuations were canceled without a source or
semantic-code failure. The aligned strict Clippy result
now passes in 245.31 seconds for the App/UI `me` plus `native_accessibility`
selection, with `--no-deps` and warnings denied. These are build receipts, not a
speed comparison. Clippy ran concurrently with the native canary, and its cold
phase includes window/font startup: **no isolated CPU-cost, latency, memory,
startup-zero-overhead or speedup claim follows**.

The independent Perf review
finds no source-supported model-length traversal or row-history lifecycle leak
blocking this scoped functional dispatch. It explicitly retains preactivation
`UiaLookupId` work and leaves broader per-jump identity/actions, memory, latency
and isolated accessibility-overhead measurements open. The finite functional
counts are not a whole-app, installed or general performance qualification.

## R4 app attempt, detection limit and cleanup

The fresh R4 stage receipt
records exit 0 in 22.69 seconds for a new private 36-file package, with seven
explicit new-leaf read/execute grants. The first run stops **before native
launch** when guards observe an additional local SID-1001 read/execute ACE on
47 objects (root, ten directories and 36 files), plus ChangeTime differences.
Payload hashes, sizes, identities and other protected fields remain unchanged;
the exact semantic comparison
finds only that ACE addition, with unchanged Home SDDL. Its cause is unknown.
After the guard/review stop and explicit user authorization for the private F3
run and cleanup, a D-only baseline records the observed state as `7a829d66…`,
retaining the original `763eee7e…` record; rebaselining changes no C-drive ACL.

The actual R4 handoff
and native result
record a **failed wait for the filename-captioned viewer**:
`Normal F3 did not open exact file: None`. The observation list and qualified
video captures are empty, and no playback-worker proof was obtained. The host is
`27dd0c65…` in software-renderer mode and loaded native-filesystem `2200e210…`
is verified; those positives
do not qualify playback. The immutable handoff's phrase “before viewer opened”
is stronger than the evidence: this controller recorded no unmatched owned-window
inventory, so whether a generic/error viewer opened remains unknown.

Read-only source review explains that uncertainty without demonstrating a
production routing failure. The R4 controller ignores the focus helper's boolean,
does not confirm View-enabled state after its click, and filters viewer titles
by the filename. The [viewer definition](../../crates/midnight-viewer-ui/ui/viewer.slint)
starts with a generic caption; the [UI frame/status paths](../../crates/midnight-viewer-ui/src/lib.rs)
set the filename caption from an accepted frame while a Status update changes
only status. The [engine opening-error path](../../crates/midnight-viewer-engine/src/worker.rs)
can publish an error without a frame, also represented by the existing
`opening_reports_plugin_open_and_initial_viewport_errors_without_a_frame` test.
A filename-only wait can therefore miss an error viewer. This is a source-supported
diagnostic limitation, **not a demonstrated production F3 routing defect or fix**.
That R4 checkpoint precedes the R5/R6 diagnostic outcomes below; neither
establishes a production routing fix or promotion.

The host retires normally with exit 0, no UIA helper remains, and thread desktop,
DPI and private desktop are restored/closed. An immediate Restart Manager
code-use guard prevents cleanup; later read-only observations show no users or
processes. Unchanged Verify and Cleanup then both pass. The
cleanup receipt
confirms removal of exactly the 36 new leaves and empty directories/root;
`%USERPROFILE%\MEF3-20261119-27dd0c65` is absent. Known preferences,
Home SDDL, old-root absences and integrity guards pass; no existing-runtime ACL
was changed. Nine between-guard ChangeTime deltas are audited separately:
**this is not an all-timestamps-unchanged claim**. Prior app build, inventory,
Clippy and accessibility-canary passes retain only their original scopes;
whole-app A/V, installed controls, MP4 and Linux remain unqualified.

## R5/R6 diagnostic follow-up

The R5 handoff
records one normal-host attempt with focus confirmed, then an inventory assertion
**before click, F3 or UIA queries**. Its “private desktop changed” message
conflates a null `GetThreadDesktop`, failed user-object query or actual name
mismatch; partial inventory and the distinguishing Win32 details were not saved.
It does not demonstrate an application F3 failure. Verify/cleanup and all three
post-payload integrity audits pass; the new C root is removed, with Home SDDL,
known preferences and old-root-absence guards preserved.

The positive API calibration
finds the owned `STATIC` window in its explicit desktop. Overall enumeration
still reports **false** because its callback rejects an additional window:
owned-window membership is a positive boundary, not a whole-preflight PASS.
No application binary or F3 action executes in that calibration.

R6 then **fails Stage with zero native runs** on fresh-copy metadata/ACL
instability. The subsequent read-only partial-payload proof records the known
SID-1001 RX-ACE delta with matching content/identity/link fields; its cause is
not assigned. The exact partial cleanup
passes: **15 copied leaves, five empty subdirectories and the empty root** are
removed nonrecursively, under proof `d80bd989…`. Existing ACL writes are zero;
Home/preferences/old absences remain unchanged. The initial failed session's
cleanup status is superseded only by that later explicit cleanup receipt.
No native F3, playback, installed, MP4, Linux or performance qualification follows.

Current selected viewer checks are tracked in the
parallel verification checkpoint of that date (in the Git history of ACTIVE_WORK.md).
They retain their own failures and scopes rather than qualifying the F3 attempts.

## R7–R10 native attempts and VIDEO presentation fixes

The R7 handoff
records one F3 action on the silent synthetic AVI with normal app `981b8583…`.
A real generic Viewer exposes Play, position and `0:00 / 0:08` across twelve
queries, but the filename/content gate times out. No pixel capture or playback
action qualifies; the worker/module observation is absent, not proof of absence.

The separate palette-gate regression and fix
identify a production defect: VIDEO legitimately sends `palette=None`, but the
UI palette gate rejects its paused first frame despite ready controls/time.
The real regression fails with frame **1 instead of 10**; adding VIDEO's
authored-pixel exemption gives **24/0 selected GREEN** across eight serial
software processes (appearance 5, preparation 9, media 5, controls 3, layout 2).
Affected strict Clippy passes with zero diagnostics, and the normal build passes
in 33.93 seconds. Its preserved artifact
is 116,562,432 bytes, `profile.test=false`, SHA-256
`442d307bc96320b72263103055a51ec519be18b96c8583e3b3c257b370e51f42`.
This later build does not rebuild HTML probe `17bd4731…`.

The R8 handoff
records progress with that app: the exact filename caption, visible `Video`
label, matching unnamed Image and enabled Play/time controls are observed.
The attempt still **fails**: its stale `File content` selector returns zero,
so capture and playback actions are not reached. A raw CIM playback-worker
candidate has the extended `\\?\C:\…` prefix and is skipped by the observer's
literal root filter; no accepted worker/FFmpeg loaded-code proof follows. These
are QA selector/observer limitations, not another established product defect.

Both R7/R8 pass strict Stage, Verify and exact Cleanup; owned C roots are absent,
hosts exit normally with code 0, helpers retire and desktop/DPI restore. They use
a private desktop/software renderer and silent AVI: no pixel, playback, audio,
physical-display, installed or performance acceptance follows from those attempts.

The R9 handoff
qualifies the authored initial **frame 0 at 289 sample points**, plus actual host,
restricted playback worker and all **five FFmpeg DLLs** under the owned package.
The overall attempt still **fails playback progression**: all four captures
remain frame 0 while UIA changes to Pause/position `0.097509176`. That published
state alone proves neither fresh pixels nor decoder progress. Strict Stage,
Verify and exact Cleanup pass; the root is absent, host exits 0 and owned
helpers/desktop/DPI retire or restore. Pause/resize/seek/EOF/replay/malformed
workflow gates remain unreached; no audio/performance acceptance follows.

The VIDEO resize-fence regression/fix
then gives a real **RED 0/1: frame 10 instead of 11** after a status-row resize,
despite valid layout/media/palette checks and a Playing snapshot. The VIDEO-only
branch cancels text-position intent during image-style scaling while keeping the
media session stable; Text/Source retain their newer-generation requirement.
Production `lib.rs` is `5e047003…`, with regression tests `c2d887fa…`.
Selected GREEN
passes **22/0/0 in 22 exact serial software processes**, including advancing video
revisions and preserved Text/Source fences. Affected strict Clippy passes with
zero diagnostics, and the normal app build passes. The current artifact
is **116,562,944 bytes**, `profile.test=false`, SHA-256
`2f5f7f25572ac32bc4e49efd1170f6cc68d89d90e7a2a58443c34af53e517dc7`.
The first actual R10 native session
**passes scoped silent-AVI F3 functional verification**, following an earlier
resource stop before Stage/native. Its **25 authored-pixel observations** include
initial 0, Play **4→9 in 0.844 s**, paused/resized 15, seek 19, near-end 44→47,
EOF 47 and replay 0; malformed AVI produces a visible decoding error. The actual
host/NativeFS/restricted-worker exact SID and five FFmpeg DLL path/hash proofs
pass with no observer errors. Strict Stage/Verify/exact Cleanup pass, the C root
is absent, the host exits 0, helpers retire and desktop/DPI restore.

Root reviewed three downsampled PNG copies: dynamic color/timeline changes and
fitting resized layout are visible. Visual provenance
retains unchanged original capture hashes; pixel qualification uses the originals.
This is **one silent synthetic-AVI/software/private-desktop session** with normal
app `2f5f7f25…` and the minimal 36-file runtime. It does not qualify audio/device
clock, physical display, full catalog, MP4, Linux, general performance or normal-
runtime promotion. Later Source/Office and HTML outcomes below retain separate
functional, staging and cleanup boundaries; they do not broaden this R10 proof.

## Source corpus and leading-context follow-up

The pre-fix Engine producer
`f64045ac…` and Source DLL producer
`e25acba8…` retain their captured-input scope. R2 failed Stage without a native
run and removed its empty root; R3 failed QA raw-path startup validation before
a viewport. Canonical-path R4
reached the actual corpus but failed **0/1/0** at C++ `0.82` rapid/reference
comparison after 44,962 ms. The warm reference omitted a 38-scalar leading-row
prefix. Its separate metadata-only strict ChangeTime failure and independent
cleanup pass remain preserved; no R4 pass is claimed.

The meaningful behavioral RED
and first correction replay
retain **83 unique cases: 79 passed/4 failed/0 ignored**. The later
R2 compile failure
executes no tests; a minimal `pub(super)` helper fix follows. The production
correction preserves `partial_line` and recovers bounded prefix context before
acknowledgment; short-byte fallback, real endpoint checks and UTF-16 fixture
geometry are covered. Current R3 replay
passes **87 unique cases**, strict Engine Clippy and all 370 unchanged guarded
inputs, with Engine `eadb7ae0…`. This supersedes the four earlier test failures
without rewriting their receipts.

Native R5
passes C++ `0.82` cold/warm exact pixels, then **fails overall 0/1** on C++ `0.43`
warm syntax. Guards, EOF, lease release, Restart Manager and exact cleanup pass.
The scalar diagnosis
finds zero valid highlight tokens and correct f32 endpoint metrics at that target;
user source hashes remain unchanged with no C++ source export. No production
defect is established. The QA-only pinned C++ `0.43` policy now requires zero
chroma; every other positive target retains chroma at least 8 and every comparison
retains exact pixels. Its four rendered oracle checks
and strict Clippy pass with 370 inputs unchanged, producing 17,016,320-byte Engine
`3b691123…`. Three cases overlap the 87; those runs are not summed.

Actual R6
**fails overall 0/1** after five complete destinations: C++ `0.82/0.43/0.94` and
Lua `0.43/0.82`. At Lua `0.999`, Source comparisons pass before plain-Text
calibration acknowledgment fails after **45 s**; that destination is incomplete
and YAML is unreached. Guards and exact cleanup pass. Diagnosis finds a real
Text coverage failure: the original window's high endpoint `0.99842` and shifted
window's `0.99850` both miss `0.999`, causing viewport failure/generation reset.
This is not a mode-oracle defect.

The narrow Text mapped/open-edge correction retains the opposite anchor and
doubles the line budget once, capped at 512 within the same 12 KiB request:
at most two reads. Legacy byte/mapless/512-cap shifting and Source recovery are
unchanged. The meaningful RED
compiles, then fails its intended window-retention assertion (`92224` versus
`92140`). The final targeted GREEN
passes **91 unique cases/0 failures/0 ignored**, strict Engine Clippy and the
unchanged 371-input guard, producing 17,050,112-byte Engine `48dea0a9…`. The 91
include the earlier 87, fourth oracle and three Text cases; overlapping epochs
are not added. Independent Perf review reports no material bounded-work finding,
not a measured speedup. Actual native R7
now **passes 1/0/0**, 516 filtered, in 23.19 s, with **all nine destinations**
complete: three C++/three Lua/three YAML, including Lua `0.999` Text calibration.
The Engine exits 0 without timeout; all owned processes reach EOF. Stage/native/
strict guards, lease release, unfiltered Restart Manager and exact cleanup pass;
the owned C root is absent and 49 frozen entries are verified. This qualifies
this CPU-path Source viewport/syntax/recovery corpus with the recorded 2f/SDK
authority. It does not establish normal-F3, gutter/retained-state or performance
acceptance. R4/R5/R6 failures remain preserved at their original epochs.

## Normal-app Source/Office initial content and Find

The Source/Office session
uses normal app `2f5f7f25…`, Source `e25acba8…`, A0 HTML and NativeFS. Automation
passes **2/8**; six Office Find flows stop before typing at the private-desktop
focus guard. Root's actual eight-image review
links PNGs to original BMPs: Lua keyword/number/string/comment colors and YAML
keys/scalars/true/numbers/URL fragment are correct. DOCX/XLSX/PPTX/ODT/ODS/ODP
show their authored initial markers before Find/navigation; XLSX A1 wraps over
two visual rows. **Blank first-content acceptance passes for these six authored
software/private-desktop fixtures.** Find and full fidelity remain open. The
original strict ChangeTime failure remains separate from its passing cleanup.

Office Find R3
fails the first DOCX ownership query with `WinError 5` before content/query/input:
**0/6**, five unattempted, zero captures/UIA helpers. The app exits 0 and private
desktop/DPI restore. Exact failing API is unknown. The controller conflates
window ownership and process lifetime; this establishes no production input
defect. Safe recovery R4
now **passes**: 750 overlapping leases, final held critical/strict-original checks,
retained owned-controller kernel EOF and tool-session EOF, all lease releases,
unfiltered Restart Manager **566 files/0 users**, and exact **566-leaf** cleanup.
The owned root is absent, independently confirmed by Root. Earlier failed
packets remain failed; **Find stays 0/6 and unqualified**, with no UI retry.
Cleanup and six-format initial-content proof do not establish a Find pass.

## Current HTML validation probe and native controls

Debug-only, default-off `provider.validate` clock `c1628473…` has 18 actor passes,
strict Clippy and release check exit 0.
Fixtures do not prove native trace emission; release checking does not link an
executable. The new probe build
is 38,180,352-byte `d5cc2128…`, `profile.test=false`: 8,009 captured inputs unchanged,
5,351 consumed inputs without capture gaps. Its failed package-level **13/14**
feature gate is preserved. The actual-unit decision
separates compiler Windows `7fb0…` and runtime `f306…`; it permits new functional/
phase attribution, not whole-app/generated-UI/binary/timing parity. The retained
monitor addendum
has 128 samples at actual mean/max gaps 1,471.8/2,107.6 ms; its sampled working set
is not a certified peak. R2
failed Stage with zero native runs/timing/CSV and passed independent cleanup;
that failed receipt is not rebased.

The later leased R3
fails WGPU device creation (`Parent device is lost`) before HTML, with no Source
cause established; guards and cleanup pass. The separate software R4 control
**passes once** with actual software renderer on both opens, all **36 wheels**
(12 isolated/24 burst) delivered and two complete CEF CSV EOFs. `provider.validate`
records **421,519/503,338 microseconds** for first/reopen; those phases exclude
queueing, browser spawn/handshake, first publication/draw and retirement. A0 CEF
mode 0 stays unchanged; foreign compiler context remains recorded. No hardware,
idle-system, general-performance or comparative-speedup qualification follows.

All 548 critical-file captures, native EOF, lease release, unfiltered Restart
Manager and exact cleanup pass; the owned root is absent. Continuous primary
read leases qualify these QA C copies' content, identities and security while
retaining ChangeTime as diagnostic with unknown cause. D originals retain strict
ChangeTime checks. Application loader/trust policy and normal-runtime promotion
remain unchanged.

## Current rendering and application boundary

The unified software UI replay
passes **10 selected cases**, plus one existing theme-selection case. The theme
checks each exercise 52 internal combinations, not 52 unit passes. Root viewed
three converted copies (compact PDF/font32, Onion/font32 and compact green
identical text), retaining original PPMs; Build viewed two theme BMPs. The actual
Cargo TEST artifact for `me` has `profile.test=true`: it is a test harness, not a
new normal executable, native app run or default-WGPU qualification.

The earlier two Source CPU-camera/gutter failures and a direct annotation RED
are retained. The accepted separate `f64` annotation-camera/label-origin path
preserves the old body viewport. The targeted correction replay
passes **70/0**, and full renderer integration passes **48/0 with 3 ignored**.
Those 118 functional passes describe the production-code epoch before a final
test-only iterator-style correction. The original strict Clippy failure remains;
final strict Clippy
and the exact three annotation replays pass after `9bfd…`, with all three
production hashes unchanged. The complete 118 were not rerun after that test-only
change, and ignored native/performance cases remain ignored.

The normal builds `981b8583…`, palette-fix `442d307b…` and resize-fix `2f5f7f25…`
are separate from the earlier Cargo TEST artifact; HTML probe `17bd4731…` is not
rebuilt by the two VIDEO fixes. The direct user-ZIP observation and selected
results retain their scopes in the
parallel checkpoint of that date (in the Git history of ACTIVE_WORK.md);
concurrent compilation prevents isolated speedup claims. R9's failed progression
remains historical; R10 qualifies the scoped silent-AVI pixel/control workflow
above. Source/Text now has 91 targeted passes after R6 exposed a real Text
coverage failure; native R7 now passes all nine scoped CPU corpus destinations.
HTML software R4 qualifies one functional/validation-phase control. Office initial content passes six fixtures and recovery
cleanup passes; Find remains unqualified.
Broader F3/A-V/catalog, corrected HTML timing, native PDF/MP4,
Linux, normal-runtime promotion and general performance remain open.

The final normal app build
passes after the Text fix: **116,569,088 bytes**, `profile.test=false`, SHA-256
`5b8b6c76058ab1495e363feddff2bf70b466f4c9aec28a8a038bbc67d8fd4bb5`.
Cargo and source guards exit 0 with no abort, all 14 feature unions match, and
main Slint bindings are cached. This supersedes `6894ceb4…`. No fresh full GUI,
whole-workspace suite or normal-runtime promotion is established.

Root's closing scope is to commit and push `codex/post-merge-validation-20261118`,
then stop, with no main merge. Remaining acceptance below is not new work
authorized by that closing instruction.

## Remaining acceptance

- Resolve the native profile's 54 warning entries and 15 missing-tool failures
  through a separate controlled rebuild; see the
  [native diagnostic classification](2026-11-19-ffmpeg-native-diagnostics.md).
  Whole-application F3 audio/video, native MP4, installed controls and Linux
  qualification remain open.
- The compile-feature compositor comparison is complete and rejected as a
  production default change. The later software validation-clock control passes
  its functional/phase scope; hardware and speedup remain unqualified. The raster
  controls establish no cause or speedup.
- Earlier native failures retain their limits. The VIDEO resize fix passes its
  real regression, 22 selected cases, strict Clippy and normal build. R10 now
  passes one silent-AVI/software/private-desktop F3 workflow with authored pixels
  and actual loaded-code proof. Broader catalog/A-V, device-clock, physical-display,
  MP4/Linux and normal-runtime promotion acceptance remain open.
- Source/Text has 91 targeted passes and native R7 passes its nine-destination CPU
  corpus; normal-F3/gutter/retained-state/performance acceptance remains separate.
  Office cleanup passes while Find stays unqualified; cleanup and initial-content
  proof do not close Find.
- Full suites, packaged executable qualification and all 54 retained requirements
  remain open. The overall goal is not complete, and no main merge is claimed.
