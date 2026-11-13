# Active work and requirements evidence — 13 November 2026

The user has resumed the goal: implement all accepted app features, test each
deeply and polish the UI. This is the current coordination and requirements
ledger, alongside [Feature status](FEATURE_STATUS.md). It reconciles the older
[requirement IDs](IMPLEMENTATION_BACKLOG.md) without treating their stale status
text or stage ordering as current evidence.

Root reports D `main` and `origin/main` at
**`02b2f2f1dcde5a4fa19f3ef657fc872a97856bad`**. This merge integrates cap128
`2b28cc7` and native host Task 4A `eb2eca8` + core/host Task 4B `cf64a6c`.
The current remote CI result remains unconfirmed. The earlier prewarm checkpoint
**`fd94d8568f83cf10fe61729bec4f66556aa5c353`** committed/pushed 17 reviewed
source/test files and two docs, with a clean tree at that checkpoint. The earlier
cap16 tree candidate retains 95 functional passes and its **failed measured
scaling aggregate**. The later [cap128 gate](#recursive-tree-cap128-gate) passes
25 targeted functional cases and one synthetic debug scaling aggregate; native
filesystem cost, UI latency and visible smoothness remain unqualified.

Earlier `fe8a4177` includes the `338eb0a` presence/Find source/test/docs checkpoint,
CI engine-warning corrections and the Clippy-step reorder, retaining its own
recorded gates.

Earlier committed/pushed D `main` base: **`1546030`**, 12 files and verified clean by
the Orchestrator at that checkpoint. It integrates the qualified link-maintenance source,
clipboard helper correction and documentation.
It includes recursive expansion `ce9eff3`, thumbnail correction `8e205c0` and
portable panel scroll regression tests `3bee1dc`, alongside the earlier API/SDK, CMD-02 and viewer work.

The current Windows launch-qualified private EXE is **`c6c371bc`**,
**101,975,552 bytes**, built from committed `fd94d85` with all 17 frozen source
files matching committed blobs and zero drift. Normal debug compilation and
private engine/native UI startup pass; this runtime contains prewarming source
but predates the tree-performance candidate. The extra auxiliary-PID cleanup
observation and startup-only qualification remain scoped below; full viewer
workflows, manual CSC/UAC access and Linux desktop UI remain unqualified.

Recursive Windows/software UI passes **69/0**. Backend's broad run was **158/1**
from 159 selected; its corrected, overlapping Windows junction/Delete rerun is
**11/0**, not 11 additional unique tests. Thumbnail correction is **14/0** and the
portable panel regression-test gate is **4/0**. These results do not establish native
scroll latency, manual CSC/UAC access, physical Linux desktop qualification or a green
current remote CI run. All open implementation and qualification work below remains
open unless its exact source/environment boundary is supported by receipts.
Loaded `DirectoryLink` manual/automatic refresh is implemented and Windows
qualified: **51 selected / 51 passed / 0 failed / 0 ignored**, compile 0 and zero
source drift across the eight applied files.
Its accepted contract preserves separate logical aliases while deduplicating native
notification slots by known physical browse identity and validating retargets.
The preceding RED was 2 passed / 3 failed / 0 ignored. GREEN passes disjoint
provider 10 + refresh 25 + watch 15 + real Windows junction 1 groups. The updated
Linux native fixture passes **1/0/0** with the frozen overlays; the local Linux UI
gate now passes **13/0/0**, with compile 0 and zero source drift. The `269f3419`
debug build and both registered private startup smokes pass for that source scope.
The later [native-link presence and durable Find gate](#native-link-presence-and-find-gate)
passes **18/0/0**; the later `26ff61f2` debug build and registered private
engine/native UI startup also pass with zero source drift.

The initial assignment names integrated `main` commit `369b161`. The Orchestrator
accepted the Markdown slice as `cc4857e` and checkpointed the build port/README as
`b5b3b7b`. Upstream `origin/main` then advanced to
`a9f31d7472bc6ad12cf9856fd513061fef940b50`, incorporating `8e084b3` Docker/native
Rust bootstrap work. Merge `28cc79add548eed0d6c9e465a6fe26a5d141f9b7` is complete
and includes that upstream head. It reconciles Docker/native bootstrap with
reviewed cleanup/dry-run/native-setup guards and the full Docker build chain,
whose caches belong under `.tools/docker/`. The merge, including README, is pushed.
The seven-document checkpoint is committed as
`5ab0e86f40c0164f4700da96e1ff09a3b5f65de5`.
The checkout is shared and
later role edits need their own source/build identity.
This documentation reconciliation inspects source and existing reports; it does
not execute application tests or qualify another role's changes.

## Authority and evidence

Current explicit user requirements and scoped assignments take precedence.
Later accepted refinements supersede conflicting earlier descriptions:

- Cross-volume/merged Move verifies and removes each successful file before
  advancing, then removes empty source directories postorder. Earlier completed
  moves remain after later cancellation/failure; there is no whole-batch rollback.
- [Transfer verification and native Move](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
  keeps full SHA-256 as the default, permits explicit size plus first/middle/last
  64 KiB verification, and uses native same-volume rename without content reads.
  Files up to 192 KiB are compared completely in quick mode. Each job freezes its
  policy; source identity, revision, sync, consent and publication guards remain.
- The [11 November dialog policy](superpowers/plans/2026-11-11-dialog-polish-followup.md)
  supersedes blanket native-modal migration. Modeless tools move independently;
  ordinary dialogs are embedded modals with focus/input gating and no taskbar
  entries. The current source includes operation/Copy/Move drafts, Panel Filter,
  Hot Paths, Make File List and blocking notices in that presentation; existing
  Options/comparison/metadata modals remain embedded. Independent modeless tools
  retain native hosts.
- [Archive streaming](superpowers/specs/2026-11-09-archive-streaming-design.md)
  supersedes the old fixed source/member byte ceilings for the negotiated
  streaming protocol. Memory, entry/name/decoder limits, cancellation, integrity
  and private-storage failures remain bounded and explicit.

Source establishes an implementation boundary. A test definition, checked plan,
enabled control, old binary or green subset does not establish current end-to-end
success. Dated reports retain their exact source, environment and scope. Work
below distinguishes **implementation gaps**, **source review**, **performance**
and **qualification** rather than converting every missing receipt into missing code.

## Current role boundaries and exclusive ownership

The [agent workflow](developer/AGENT_WORKFLOW.md) and [project rules](../AGENTS.md)
govern assignments and handoffs. The current documentation reservation covers only
this ledger, [Inline trees](developer/INLINE_TREES.md) and
[Tree panel](developer/TREE_PANEL.md). The image API doc remains frozen in
`fd94d85`; earlier Tree/ledger changes are committed in `338eb0a`.
Other rows record ownership and remaining focus, not a claim
that completed probes are still running or permission to begin unreserved work.
The Orchestrator records concrete source paths and acceptance before shared edits.

| Owner | Recorded work / remaining focus | Ownership and coordination boundary |
| --- | --- | --- |
| Orchestrator | Coordinate the full goal; independently critique/review proposals, changes and evidence; own integration | Owns file reservations, API/UI integration decisions, Cargo target-directory/native-input leases and Git checkpoints. Commit/merge/push remain Orchestrator-owned. |
| Architect | Review engine/UI integration against the accepted Task 4B disposal contract | C native Rename passes selected Linux 38/0/0 and Windows 22/0/0 audit gates. Task 4A's exact five-file C candidate is accepted by Root/Architect and applied with matching hashes; its direct C host foundation gate passes 44/0/0 with compile/Clippy 0. Task 4B's finite finish/owned prepare-failure implementation is committed as `cf64a6c` within 11 C core/host paths; its [preceding 79-case and final narrow gates](#native-rename-audit-gates) retain separate source scopes. Task 4A's five-file qualification stays separate; engine/UI cutover is unimplemented and native candidates are undeployed. The review and handoff do not establish deployment or qualification. Recursive batching, exact Commit, rollback/identity and capacity contracts are implemented in `ce9eff3`; source review is not native/platform qualification. The DirectoryLink contract/applied source are reviewed and Windows-qualified at 51/0/0; the updated Linux fixture passes 1/0/0 on WSL/tmpfs, with desktop/platform acceptance separate. Native Rename/host cutover, provider watches and archive nesting retain separate acceptance. |
| Backend agent | Retain integrated Task 4B core/host evidence; engine/UI cutover remains separate | The native-only presence correction passes 12/0/0; durable Find regressions pass 6/0/0 in the same 18-case gate. The earlier diagnostic gate passed 13/0/0 with the exact user fixture unchanged and no differing identity/revision; no production Find correction is demonstrated. Preserve replacement and stale-session rejection. C Linux recovery-locator audit fixes pass the selected 38/0/0 gate on real WSL/tmpfs; Windows passes 22/0/0 after the fixture-only exact-restoration correction, retaining the preceding 21/1 history. Keep the [native audit receipts](#native-rename-audit-gates) under separate C reservations and build leases; Task 4A is accepted/applied under its five-file C reservation with the direct C host foundation gate passing 44/0/0, compile/Clippy 0 and zero drift; Task 4B's finite finish/owned prepare-failure implementation is committed in C as `cf64a6c`, with [scoped preceding and final gates](#native-rename-audit-gates); Root integrated that core/host checkpoint in D merge `02b2f2f`. Engine/UI cutover is unimplemented and native candidates are undeployed. API/SDK Task 1/docs are integrated at `e5cba5d`. Recursive Backend refined Windows gate passes 11/0 overlapping the broad 158/1 result; CSC Engine boundary passes 5/0 with mocked helper outcomes. Native Rename candidates are excluded from the retained runtime; the original Linux recursive symlink fixture is verified 1 passed / 0 failed / 0 ignored under WSL/tmpfs at `1714d99`; manual authorization, physical Linux/native-provider acceptance and host cutover remain open. Loaded DirectoryLink maintenance passes the Windows 51/0/0 gate with zero drift; its updated Linux fixture passes 1/0/0 on WSL/tmpfs; the new registered private Windows startup is qualified. C work keeps separate source identities and leases. |
| Viewer agent | Retain implemented prewarming/shutdown lifecycles and scoped evidence with Perf | The source-only [image-spare shutdown gate](#image-spare-shutdown-gate) passes 36 headless + 3 native cases at `3e2238d`; deployment and generic natural-process shutdown remain unqualified. The final 17-file R3 freeze passes compile/strict workspace Clippy with zero drift. R3 host14 plus the retained other30 deterministic successes give 44 unique cases; preserve R2's earlier 43 and initial 34/9 history. Five native headless controls pass 5/0/0 with zero drift and the pinned runtime unchanged. One same-candidate BMP pair records cold 253.919 ms and ready 139.410 ms including 71.182 ms claim validation; preparation is 204.803 ms separately. Final fixture/lint qualification passes; no general image-opening or smooth-scroll improvement is established. The earlier headless host control passed 1/0/0 and measured a 7.192-second process-creation interval before image bytes; that single control is not a prewarm comparison. The two native attempts failed foreground admission and establish no qualified performance baseline. The one-off Source probe completed 1,098 operations/732 exact replays without mismatches and is archived under `_build/`; it is not a reusable repo target or native C++ scrollbar diagnosis. Markdown source/A-B and HTML instrumentation retain their scopes. Full viewer/native scrolling and matching packaged-worker evidence remain separate. |
| UI agent | Retain the applied copy-dialog test alias and share Viewer/Perf presentation evidence | The Root-approved alias-only correction for `tests/copy_dialog.rs:666` `type_complexity` is applied as `82f0a5a3…edd26`, preserving all bodies/assertions; final fixture/lint qualification passes with strict workspace Clippy 0. Own input/presentation phases within the shared investigation; retain the completed local Linux helper gate: clipboard 5 + copy 2 + move 6, all passing. Recursive/tree/CSC UI is 69/0; copy dialog 8/0, selected Native Jobs 2/0 and native-operations UI 4/0 pass on Windows. CSC warning retention is source-corrected; manual CSC/UAC access is unproved. CMD-02 retains its earlier 37/0 scope. The correction in `tests/support/tree_filters.rs` passes the local 13/0/0 Linux runtime gate. Startup does not close full layout/input/platform acceptance or Find Open. |
| GitHub build system and build tools developer | Retain cap128, Rename and image-spare gates; coordinate later validation | Task 4B's [final narrow gate](#native-rename-audit-gates) passes two exact cases per platform with compile/strict Clippy 0; retain the preceding 79/79 separately. The [image-spare shutdown gate](#image-spare-shutdown-gate) passes its scoped 36 headless + 3 native cases. Linux Rename's earlier 38 cases and Windows Rename's 22 cases pass with leases released. The first D prewarm compile/Clippy attempt exited 101 before tests, with zero drift and leases released. R2 compile passes with zero drift and 43 unique deterministic cases pass after the same-binary trusted-fixture host rerun. Five native headless controls pass 5/0/0 with zero drift and the pinned 1,382-file runtime unchanged. Strict workspace Clippy previously exited 101 on the unrelated copy-dialog test type-complexity warning; R3 now passes compile/strict Clippy with zero drift and host14/0/0. Its 44 unique deterministic cases retain the other30 successes; native5 remains the R2 receipt. The `fd94d85` debug/private `c6c371bc` startup is qualified separately, with the unresolved auxiliary-PID observation retained. The earlier cap16 candidate retains 95/0/0, Clippy 0 and its failed timing aggregate. The integrated [cap128 gate](#recursive-tree-cap128-gate) passes 25 functional cases, strict Clippy 0 and one synthetic debug aggregate; no baseline rerun or native/UI claim. C Task 4A passes 44/0/0 with compile/Clippy 0, 252 captured inputs without drift and the test child exited/leases released. The CI engine gate passes Clippy/compile and 31/0/0; workflow validation is structural only. Retain the earlier presence/Find 18/0/0, Linux UI13 and historical `26ff61f2` startup scopes. Own subsequent build, target-directory and native-input leases; coordinate shared Viewer/UI/Perf evidence. Preserve runtime/preferences/shortcut and dated CI failures; selected local gates do not establish remote/workspace GREEN. |
| Perf agent | Qualify native tree/viewer performance and retain scoped measurements | The later [cap128 aggregate](#recursive-tree-cap128-gate) completes its four timed synthetic opt0 fixtures within the unchanged 60 s cutoff. It is one aggregate without a new baseline or native/UI measurement. Preserve the earlier cap16 failed aggregate and functional95/Clippy0 separately. Share the Viewer/UI trace and evidence, with no separate competing capture or build. `3bee1dc` is a portable panel scroll regression-test commit (4/0), not a production correction; `8e205c0` has 14/0 thumbnail tests. Deterministic software counts/retention do not establish painted flicker, native frame pacing, SSD latency or RSS. Root confirmed the original watch callback-lock P2 closed against applied `watch/tree.rs` hash `a17e9279…b96d5`. The earlier 7.192-second CreateProcessW interval is a single headless startup observation, with no qualified prewarm speedup or smooth-scroll claim. Source scrollbar input-to-paint, recursive scaling and native Rename costs still require their own measurements. |
| Documentation guru | Record Task 4B and image-spare contracts with scoped evidence | Own the staged `NATIVE_FILESYSTEM_API.md`, `ENGINE_LIFETIME.md`, `TREE_PANEL.md` and `ACTIVE_WORK.md` candidates. Keep developer contracts durable and dated gate/runtime evidence in this ledger. Preserve prior scopes, requirement IDs and open acceptance; Root reviews/applies the candidate and owns Git. No source, Cargo, CI or application runs. |

Native Windows Task 2 and Linux Task 3 remain WIP in the existing
`C:/Users/tjelinek/.codex/worktrees/main-deep-audit/FM` checkout and are excluded
from D integration. Build owns Cargo leases across C and D. Task 1 `f3f8c59`
and public docs `852ee0c` are now merged into D `main` at `e5cba5d`.
`_build/verification/build-20261112/api-sdk-integration-result.json` records
exact equality of all 33 incoming files and zero dirty overlap. Source gates
include API/SDK tests, corrected Rename tests, strict Clippy, direct Windows/network
consumers and the C host all-targets check, retaining each original receipt's scope.
This integrates public ABI/API/SDK source and docs; no native prepared-Rename
capability/deployment or host/UI cutover is established. The retained private EXE below has its own `3619070` source and startup receipts;
native candidate deployment and host cutover are still excluded.

The user authorized selecting useful historical branch changes, discarding obsolete
changes and deleting branches. The Orchestrator accepted the archive/thumbnail
source-equivalence reports and deleted their exact local/remote tips and clean
worktrees.
The `origin/fix/cargo` tip `8e084b3339ea3bf6cbc72567d7691af1dfb4fa7d` is now
incorporated by upstream `a9f31d7` and completed local merge `28cc79a`, preserving
Docker fallback/native Rust bootstrap alongside the reviewed guards.
The reconciled contract keeps `FM_BUILD_BACKEND=auto|native|docker` and
`./build.sh [debug|release|clean] [--install-deps] [--dry-run]`; explicit package
installation selects native setup and conflicts with explicit Docker. Docker uses
the full app/elevation/filesystem/viewer chain and `.tools/docker/` caches.
The Orchestrator verified the exact Cargo tip is an ancestor of upstream and
deleted remote `fix/cargo` under that exact-tip lease. No local `fix/cargo` branch
existed; the reported remote branch list now has no unmerged branches relative
to `origin/main`. The already merged
`main-deep-audit` worktree is retained because the Orchestrator's host cwd uses it.
No role independently switches/merges/deletes branches. No new build or native
desktop input starts without the coordinated lease. Independent headless tests use
bounded parallelism; desktop/input and performance probes run serially.

Merge ownership was exclusive: Documentation resolved `README.md`; Build resolved
`build.sh`. The Orchestrator completed the merge. Other frozen documentation
changes are preserved in checkpoint `5ab0e86`; this ledger retains the cleanup
status.
Staging, commit and push remain Orchestrator-owned.

## Integrated baseline and recorded evidence

<a id="current-checkpoint"></a>

### Current Windows launch-qualified runtime `c6c371bc`

The fresh debug receipt (`_build/verification/image-prewarm-debug-20261113/debug-build-result.json`)
records exit 0, zero drift and **101,975,552 bytes** for `_build/debug/me.exe`,
SHA256 `c6c371bc313b15913ea2584bf5adc88a3ad165702d9bd64a5b95e40d012511ac`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-024953-c6c371bc/me.exe`.

The qualified startup receipt (`_build/verification/image-prewarm-debug-20261113/qualified-debug-result.json`)
records source `fd94d8568f83cf10fe61729bec4f66556aa5c353`, all 17 frozen sources
matching committed blobs, zero drift and both established smokes exit 0:
engine **17.445 s**, native UI **12.867 s**. The visual-review receipt records
normal fixture panels/toolbars/status/footer. All 1,382 runtime file hashes were
verified; fixture/preference/settings hash-and-mtime checks are unchanged. The
previous `26ff61f2` runtime and desktop shortcut are preserved. Native Rename candidates were not
deployed and no tree-performance candidate is included.

Owned job cleanup completed. The extra blanket pre-cleanup PID assertion failed
on PID 29940; its identity/path/parent lifetime remains unclassified. Natural
shutdown of all auxiliary processes remains unqualified. The later
[image-spare shutdown correction](#image-spare-shutdown-gate) is committed and
passes its scoped gates, but is absent from this runtime. PID 29940 remains
unattributed. No smoke retry or new-runtime performance harness run occurred. Startup
does not qualify large-tree speed, complete viewer workflows or physical Linux UI.

### Image-spare shutdown gate

Root committed the seven-file source correction as
`3e2238da6b26ddd61dab5cddb91660158a710b76`. The
[lifetime contract](developer/ENGINE_LIFETIME.md#image-viewer-spare-shutdown)
describes spare stop/terminal acknowledgment before runtime cleanup and retry.
The combined receipt (`_build/verification/viewer-20261113/image-startup/shutdown-gate-result.json`)
records **36 headless + 3 native passes**, default/diagnostics compile and strict
Clippy **0**, zero source drift, released source/target/native-input leases and
removal of the owned staged executable. The three native cases cover specific
Windows worker boundaries, including suspended-child Job membership and cleanup
after the last inner handle closes. This qualifies source-free spare retirement
and those worker boundaries only. PID 29940 is unclassified; general natural
auxiliary-process lifetime and app deployment remain unqualified. The current
private EXE remains `c6c371bc` and predates this correction.

### Previous Windows launch-qualified runtime `26ff61f2`

`_build/verification/backend-folder-presence-links-20261113/debug-build-result.json`
records exit 0, zero source drift and **101,794,816 bytes** for
`_build/debug/me.exe`, SHA-256
`26ff61f2de03e13b86e82e66bf3ddad1087fc21d47bdd445e9dedb4d10d9ad59`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-004830-26ff61f2/me.exe`.

The qualified startup receipt (`_build/verification/backend-folder-presence-links-20261113/qualified-debug-result.json`)
and `private-entrypoint-smoke-results.json` in that directory record engine
`--smoke-test` exit 0 (**17.779 s**) and native `--ui-smoke-test` exit 0
(**6.883 s**), qualified at **00:51:43 UTC on 13 November**. Source is baseline
`1546030` plus the frozen presence/Find files, with zero drift; production Find
is unchanged. All 1,382 runtime file hashes were verified, fixtures/preferences
are unchanged, the previous `269f3419` runtime is retained and the desktop
shortcut is unchanged. Root independently inspected the normal two-panel startup
snapshot. Native Rename candidates were not deployed. This qualifies Windows startup, not protected CSC/UAC access, full viewer workflows, Linux
desktop UI or native performance.

### Previous Windows launch-qualified runtime `269f3419`

`_build/verification/backend-tree-link-refresh-20261113/debug-build-result.json`
records exit 0, zero source drift and **101,794,816 bytes** for
`_build/debug/me.exe`, SHA-256
`269f34195139865e4fb68430fdc396eeb384469dc9cc28a7222612b6f89658eb`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261113-269f3419/me.exe`.

`private-entrypoint-smoke-results.json` and `private-qualified-windows-result.json`
in that directory record both engine `--smoke-test` and native `--ui-smoke-test`
exit 0, zero source drift, and baseline `1714d99` plus frozen source overlays.
The previous `d2643b49` runtime, preferences and desktop shortcut are preserved;
candidate native Rename plugins were not deployed. This qualifies Windows
startup, not full viewer workflows, manual CSC/UAC access or Linux desktop UI.

### Previous launch-qualified runtime `d2643b49`

`_build/verification/build-20261112/final-debug-build-result.json` records a
normal debug build, exit 0, zero source drift, **101,768,704 bytes**, SHA-256
`d2643b49917c29b8f5f3b0171dc075bb30058817de6f0b6b1212d8361c0f658d`.
The registered private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261112-224859-d2643b49/me.exe`.

`final-private-qualified-debug-result.json` records source
`3619070f6e54eb442ddfa13fcd7b17d9282225f4` and qualification at **23:51:48 CET on
12 November**. Engine `--smoke-test` and native `--ui-smoke-test` both exit 0;
fixtures are unchanged. Root inspected the startup snapshot: two panes, toolbars
and footer rendered without a startup error. The previous private runtime,
preferences and desktop shortcut are preserved; native Rename candidate plugins
were not deployed. These are startup results, not manual CSC/UAC, full viewer
functionality, Linux or native performance qualification.

### Native link presence and Find gate

The native flat-panel admission correction accepts `DirectoryLink` only for an
exact captured native root; archive batches still reject links and files. One
native link previously discarded the entire viewport request. Empty or denied
links now retain individual hints alongside ordinary siblings, with no recursive
probe or provider/UI/API change. Frozen `folder_presence.rs` SHA256 is
`af9cdba4549caace4bfedf02eaaf34577fa273dee9a13320b52553bfe90b6c95`.
The actual RED receipt is
`_build/verification/backend-folder-presence-links-20261113/build-red-20261113T003717Z/result.json`:
compile 0, **6 passed / 2 failed / 0 ignored**, zero drift.

The final Build receipt (`_build/verification/backend-folder-presence-links-20261113/build-green-find-20261113T004248Z/result.json`)
records **18 selected / 18 passed / 0 failed / 0 ignored**, both compiles 0 and
zero source drift. Presence is **12/0/0**: admission 8 + native provider 4.
Durable Find regressions are **6/0/0**: Engine 5 + Windows native 1. All owned
processes exited and the source lease was released. The fresh `26ff61f2` debug
build and registered private engine/native UI startup pass with zero drift, as
recorded in the [26ff61f2 runtime receipts](#previous-windows-launch-qualified-runtime-26ff61f2); the previous
`269f3419` runtime retains its earlier source scope.

The earlier Find diagnostic receipt (`_build/verification/backend-find-result-revision-20261113/build-owned-20261113T002043Z/runtime-result.json`)
records **13/0/0**, including read-only probes of the exact reported
`D:/dev/ov-physics-trunk/ovclash/source/DynamicMatrixCacheLimit.h`. Its hash, size
and mtime were unchanged; raw path, kind, identity and all six revision words
matched, and repeated validation was accepted. The original Open report remains
unreproduced: its old build and captured metadata are unknown. No production Find
fix or proven false-stale cause follows. The final six durable regressions
overlap that diagnostic scope, with no user-fixture rerun in the final gate;
they are not six additional unique diagnostic cases.

<a id="tree-scaling-candidate-gate"></a>

<a id="tree-scaling-candidate-gate"></a>

### Tree scaling candidate gate — aggregate failed

The corrected candidate build (`_build/verification/tree-performance-20261113/candidate-build-result.json`)
exits 0 with `fresh=false` (Cargo rebuilt the artifact), zero drift and controller SHA256
`16b725e3e997da4459e6ee581761904ae2b9ee251178cb7c587b8035ba83cb1e`.
The functional receipt (`_build/verification/tree-performance-20261113/candidate-functional-result.json`)
passes **95 unique cases / 0 failed / 0 ignored**: library 91, including 10 new
invariants, plus controller 4. Candidate Clippy (`_build/verification/tree-performance-20261113/candidate-clippy-result.json`)
exits **0**, zero drift. These functional scopes remain valid separately from
the failed timing aggregate and are not additional passes for older gates.

The admitted candidate aggregate (`_build/verification/tree-performance-20261113/candidate-scaling/result.json`)
reports **FAILED**, exit **101**. Timings are from a debug **opt-level 0** backend
controller with a synthetic provider: no filesystem I/O or UI paint is measured.
`N` includes the invisible root; `K` is the number of settled cached directory
sizes. The existing baseline (`_build/verification/tree-performance-20261113/baseline-scaling/result.json`)
was retained without a repeat:

| Fixture | Existing baseline | Corrected candidate |
| --- | --- | --- |
| Wide `N=1,000`, depth 1, `K=0` | 720.696 ms; completed | 496.748 ms; completed |
| Wide `N=10,000`, depth 1, `K=0` | Censored at 60 s; 7,935 callbacks | 50.83 s; completed |
| Wide `N=10,000`, depth 1, `K=1` | Not measured | 50.01 s; completed; candidate-only |
| Broom `N=10,000`, depth 32, `K=0` | Not measured | Censored at 60 s; 9,936 callbacks / 9,920 published entries; incomplete; candidate-only |

The completed 1k comparison is one unrepeated pair, without a statistical claim.
The 10k baseline and candidate broom are censored observations, not completed
durations; the broom has `failureNone=false`. `K=1` and the broom have no baseline
comparison.
The initial compiler artifact was stale because copied source timestamps preceded
Cargo dep-info. It was rejected before timing and retained as
`stale-fingerprint-*` evidence. Only the freshly rebuilt corrected candidate was
admitted; the existing baseline was not rerun. This cap16 aggregate remains failed.
The later cap128 gate below has its own source identity and recorded scope; it
does not rewrite this result. The current `fd94d85`/`c6c371bc` application predates
both source increments.

### Recursive tree cap128 gate

Cap128 source `2b28cc7` is integrated in D merge `02b2f2f`. The recursive batch
bound is **128 returned results or 50 ms**, with elapsed time checked only after
an actual provider callback; it is not a callback timeout. The
[contract](developer/TREE_PANEL.md) preserves exact Commit, cancellation and
captured-action safeguards.

The recorded cap128 build/functional gate passes **25/0/0**, compile **0** and
strict Clippy **0**, with zero source drift. The cap128 scaling receipt records
one passing **opt-level 0/debug synthetic aggregate**: an untimed warmup and four
timed fixtures, the same workloads and unchanged **60 s** cutoff, no retries and
no new baseline run. `N` includes the invisible root; `K` is settled cached sizes.

| Fixture | Wall time | Publications | Returned results / callbacks |
| --- | ---: | ---: | ---: |
| Wide `N=1,000`, depth 1, `K=0` | 110.1866 ms | 8 | 999 / 999 |
| Wide `N=10,000`, depth 1, `K=0` | 7,140.3042 ms | 80 | 9,999 / 9,999 |
| Wide `N=10,000`, depth 1, `K=1` | 6,904.8673 ms | 80 | 9,999 / 9,999 |
| Broom `N=10,000`, depth 32, `K=0` | 8,731.4107 ms | 79 | 9,999 / 9,999 |

All four timed fixtures complete with `failureNone=true`. The aggregate is one
harness case (**1/0/0**), not four additional functional tests. The retained cap16
binary is unchanged; source/target/performance leases were released and owned
job cleanup completed. Exact evidence is under
`_build/verification/backend-tree-recursive-cap128-20261113/` (`result.json`,
`cap128-build-result.json` and `cap128-scaling/result.json`). This measures
synthetic backend work, with no native filesystem I/O or UI paint. It does not
qualify release performance, SSD latency, physical Linux UI or visible smoothness.
The existing `c6c371bc` runtime remains unchanged and excludes this source.

### Integrated source and targeted gates

| Area | Accepted source and exact recorded scope |
| --- | --- |
| Recursive expansion | `ce9eff3`: Ctrl+expander recursive inline browsing, including links/junctions, cancellation, ancestor identity/cycle checks, bounded batches/Commit, rollback and Refresh guards. Windows/software UI is 69/0. Initial Backend gate: 159 selected, 158 passed/1 obsolete legacy assertion failed; refined gate: 11/0 (10 Delete captures plus one real Windows junction), unchanged production and zero drift. This rerun overlaps the broad gate. Loaded DirectoryLink maintenance is implemented/Windows-qualified at 51/0/0 with zero drift; its updated Linux fixture passes 1/0/0 with zero source drift. The original Linux native symlink fixture is verified 1/0/0 under WSL/tmpfs at exact `1714d99`; compile and test exit 0. Physical Linux desktop qualification and larger-than-model paging remain open. |
| CSC warning retention | UI `20f507c` originally passed 17/0; Engine `3826298` passed 5/0. The integrated 62-case presenter group includes tree 44, folder presence 9 and elevation 9; disclosure 5 and tree panel 2 give 69 total, not 69 plus the earlier CSC counts. Mock helper denial/cancellation/transport and event retention do not prove successful privileged CSC access. |
| Current interaction gates | Windows copy dialog 8/0, selected Native Jobs notice cases 2/0, native-operations UI 4/0 and viewer image/pixel preparation 26/0 pass in the integrated receipt. This does not close all native input/layout/viewer/platform acceptance. |
| Thumbnail correction | `8e205c0`: later thumbnail gate 14/0/0, zero drift. It supersedes the earlier 12-pass/2-RED-failure result only for that group; no latency/SSD claim follows. |
| Portable panel regression tests | `3bee1dc`: portable panel scroll regression-test commit, recorded 4/0; it changes no production panel behavior. The retained panel diagnostic measures bounded row/model behavior and a synthetic pressure case, not current native input-to-paint or universal retention. |
| CMD-02 | `21c9c7a`: corrected Windows gate 37/0/0, zero drift. The initial 31-pass/5-tree-setup-failure gate is historical. The current private startup does not replace remaining Linux/raw-name/native command-workflow acceptance. |

[Recursive/tree and final startup verification](verification/2026-11-12-recursive-tree.md)
and [CSC verification](verification/2026-11-11-directory-authorization.md#12-november-csc-warning-retention-regression)
retain the behavior and boundaries. Build receipts under
`_build/verification/build-20261112/` are
`recursive-ui-viewer-thumbnail-runtime-result.json`,
`tree-recursive-green-runtime-result.json`,
`tree-recursive-refined-runtime-result.json`,
`thumbnail-green-runtime-result.json` and
`final-private-entrypoint-smoke-results.json`. Panel source/test scope is separate
from the original `panel-scroll-costs-baseline.json` diagnostic.

The Linux receipt is
`_build/verification/build-20261113/linux-tree-native-1714d99/result.json`,
with its raw `test.log`: one real symlink test passed on WSL/tmpfs. The wrapper's
CRLF-only exit 2 was corrected without rerunning the test; Linux desktop UI remains
outside this qualification.

The **updated** Linux link-maintenance fixture also passes **1/0/0**, compile/test
exit 0 and `source_drift=[]`, with the frozen source overlays. Its receipt is
`_build/verification/backend-tree-link-refresh-20261113/linux-engine-only-20261112T235114Z/result.json`.
This extends WSL/tmpfs filesystem coverage, not Linux desktop UI qualification.

The Source revisit probe is complete and archived:
`_build/verification/viewer-20261112/source-revisit-probe/status.json` records
1,098 operations, 14 reference streams, 732 exact replay comparisons, no mismatch,
a 53,144-byte maximum request and zero reads for the logical 10 GB distant-Locate
control. It made no production change and does not explain the reported native
drag delay on the 134-line C++ file. Its machine-specific harness is no longer a
repository test target; native input-to-paint attribution remains open.

<a id="qualified-ui-checkpoint-35a159c"></a>

### Earlier bounded evidence

The `35a159c` positive-input gate passed 20 distinct tests (native Filter 17,
Filter library 2, exact operation-lifetime 1); exact-first replay overlaps those
counts. Its `0bf32ff3` private runtime and shared-engine retry remain historical.
The initial shared executable-hashing failure in `app-smoke-results.json` remains
unattributed; later stable checks, diagnostics and new startup passes do not
identify its original changed field. Existing embedded-modal, selection and
font-cache behavior remains covered only by each recorded source/test scope.

The corrected Markdown gate is **72 passed / 0 failed across 12 targets**.
Its strict Clippy receipt,
`_build/verification/viewer-20261112/candidate-corrected-clippy-result.json`,
also records exit 0 at **19:41:22 CET**. The correctness summary is
`candidate-corrected-correctness/summary.json` in that viewer evidence directory.
These results qualify the corrected source manifest; matching packaged-worker
deployment/native scrolling qualification is not established by these receipts. Controlled debug request wall-time
and allocator measurements below supported Perf's source-integration acceptance;
Markdown is committed at `255fbdc`. Native scrolling/deployment and HTML viewer
qualification remain separate.
Earlier 69/2 artifacts retain only their pre-correction scope.

The controlled debug A/B summary is
`_build/verification/viewer-20261112/corrected-comparison-paired-20261112-1920/summary.json`.
All **32 profiling passes** completed with exact serialized-output equivalence.
Large-fixture backend request wall-time medians are about **23 ms → 11–15 ms**,
with **383,320 / 461,739 bytes** more per-request incremental allocator peak
(not total process memory or RSS). Small-fixture medians are approximately flat;
p95 rises **0.038–0.298 ms**, with mixed maxima. Timings use `Instant`, not process
CPU time. `_build/verification/perf-20261112/markdown-ab-acceptance-review.md`
accepts source integration with those costs. Native scrolling, packaged-worker
deployment and release/OS-cold qualification remain open.

HTML diagnostic-only instrumentation is committed at `e6e086c`.
`_build/verification/build-20261112/html-validation-result.json` records **20
tests passed**, strict Clippy/default build exit 0 and unchanged frozen source
hashes. `nativeCapturesRun=false`; no HTML performance improvement or native
qualification is claimed.

The later two-attempt Source/PNG bundle,
`_build/verification/viewer-20261113/native-baseline/build-owned-two-case-20261113T003328Z-e4cf9604/result.json`,
records failed foreground admission for both attempts and no qualified native
performance baseline. Owned jobs stopped and source/runtime/preferences/fixtures
were preserved. The partial PNG review (`_build/verification/perf-20261113/png-partial/REVIEW.md`)
places the dominant observed gap before the first counted image bytes; it does
not identify decoder cost or establish F3, completed native redraw or observer
overhead. The subsequent headless host control (`_build/verification/viewer-20261113/image-startup/headless-host-startup-20261113T005252Z/result.json`)
passed **1/0/0**, compile 0 and zero drift, against the unchanged `269f3419`
runtime. Its phase summary (`_build/verification/viewer-20261113/image-startup/headless-host-startup-20261113T005252Z/phase-summary.json`)
records **7,192.133 ms** from environment preparation to process creation returning,
before counted image bytes. This authored 2x1 BMP common-host control is not PNG
decoder, production F3, UI/GPU or smooth-scroll qualification. At that earlier
snapshot, image prewarm was frozen for review/execution and no measured
improvement had been established.

<a id="image-prewarming-source-and-evidence"></a>

The reviewed 14-file prewarm manifest (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-final.json`)
was SHA256 `d08cd7dfd20d875239b116ed5974e10ac259dcee096ef9e4c0c44f21a39babda`.
Root and Perf independently matched its source hashes without drift; the
source-delta review (`_build/verification/perf-20261113/prewarm-diff-review/final-deltas/reviewed-hashes.json`)
reported no source blocker before execution. The subsequent compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/build-result.json`)
and strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/clippy-result.json`)
each exit **101**, with zero drift. Two legacy test `Service` initializers lack
new fields; the owner-correction receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T015808Z/owner-correction-needed.json`)
records **0 tests / 0 native runs** and released source/target leases. Viewer owns
the two mechanical fixture corrections after that release; both are included in
the subsequent 16-file R2 freeze (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-r2.json`),
SHA256 `748cd46fc9d922f117ee5f264658d78705f2bb199106de963e6e209c5e6fb419`.
R2 execution resumed under Build. Its fresh compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/build-result.json`)
records exit **0** with zero drift. The first deterministic receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/deterministic-result.json`)
is **34 passed / 9 failed / 0 ignored**; all nine failures are host-fixture trust
rejections before test logic. The same-binary private-fixture rerun (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/host-final-private-result.json`)
passes the 13 host cases with zero failures/ignores/drift. Four host successes
overlap the initial 34: **34 + 13 − 4 = 43 unique deterministic cases**.
The final trusted fixture cleanup (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/trusted-fixture-cleanup.json`)
reports zero remaining entries; the trust receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/fixture-root-trust-result.json`)
confirms ancestor ACLs unchanged. Runtime/package hashes remain unchanged.

The five native headless controls (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/native-result.json`)
pass **5/0/0**, with zero drift, all **1,382 runtime files unchanged**, and owned
children released. The pinned worker package is the retained `269f3419` runtime;
these controls use the candidate host executable rather than the application UI.
The paired control trace (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/native-paired/stderr.log`)
records one same-candidate authored 2x1 BMP comparison:

| Host-control interval | Observed time |
| --- | --- |
| Cold Open to exact CPU pixels | 253.919 ms |
| Ready Open to exact CPU pixels, including claim | 139.410 ms |
| Claim identity validation within ready Open | 71.182 ms |
| Preparation before ready Open | 204.803 ms |

Preparation performs zero source validations/reads; ready Open records zero
process-creation attempts/successes. Preparation is separate from ready Open;
claim validation is already included in 139.410 ms. The
final native fixture cleanup (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/final-fixture-cleanup.json`)
reports zero remaining entries. The older 7.192-second control retains its own
build/environment scope. This host pair does not time engine manager/refill,
application F3, other images, scrolling/smoothness or deployment.

The R2 strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T020958Z/clippy-result.json`)
exits **101**, zero drift, on the unrelated copy-dialog test `type_complexity`
warning. The Root-approved alias-only correction (`_build/verification/ui-20261113/copy-dialog-clippy/applied-receipt.json`)
is applied, preserving every body/assertion; final fixture/lint qualification is
passed in the final R3 gate below. The later `fd94d85`/`c6c371bc` runtime is
startup-qualified in the [current checkpoint](#current-checkpoint); that startup
does not add application F3 or Source-gesture performance qualification.

The final 17-file R3 manifest (`_build/verification/viewer-20261113/image-startup/prewarm-candidate-r3.json`)
is SHA256 `a28d6c5c696b4183fb24285937b07c762eecc4d67ecf8ed51feda34783715054`.
Its compile receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/build-result.json`)
and strict workspace Clippy receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/clippy-result.json`)
both exit **0**, with zero source drift. The R3 host receipt (`_build/verification/viewer-20261113/image-startup/combined-prewarm-20261113T023426Z/host14-result.json`)
records **14 selected / 14 passed / 0 failed / 0 ignored**, the child exited,
fixture root absent, no remaining entries and unchanged ancestor ACLs. Those
14 host cases replace the earlier 13: **14 + 30 = 44 unique deterministic cases**.
The retained **five native controls and paired timings are R2 evidence**; R3 does
not relabel or repeat them. Production prewarm sources and existing native-control
bodies are byte-identical to R2; R3 changes only fixture cleanup/constructor-panic
coverage and UI test aliases.

Perf's measured acceptance (`_build/verification/perf-20261113/image-worker-startup/MEASURED-ACCEPTANCE.md`)
accepts this one same-candidate ready-hit host observation. Ready steady-idle
memory/CPU, complete-run peak resources, application F3, Source scrolling and
all-image performance remain unqualified. Root committed/pushed the 17 source/test
files and two prewarm docs as `fd94d85`.
The subsequent `c6c371bc` private startup is qualified separately; this host-control
result itself does not establish application performance or tree optimization.

Earlier size/model tests confirmed that each folder's active flag is separate
from warning severity: terminal partial totals keep normal metadata number
styling, `≈`, yellow warning/details and truthful partial/cancelled state, while
only actively unfinished totals are highlighted. `size_errors` 2/0,
`panel_warning_details` 4/0 and `tree_folder_sizes` 7/0, plus the selected library
groups, are preserved in
`_build/verification/docs-20261112/ui-followup-terminal-20261112T174659Z.json`.
The latest modal gate does not rerun that entire scope. See the
[tree size contract](developer/TREE_PANEL.md#directory-sizes). Full-control
clipping/native-layout qualification remains open.

The [12 November panel qualification](verification/2026-11-12-panel-interaction-polish.md)
records bounded tree publication/command admission, captured actions, diagnostics,
root-level native directory comparison, current deselection defaults and the coral
PDF palette correction. It records final Windows UI library **984 passed / five
ignored**, Linux UI library **979 passed / two timing benchmarks ignored**,
additional targeted gates and final debug backend/native/software startup smokes.
Those are recorded results, not tests rerun during this documentation pass, a full
workspace/remote CI pass or accepted UAC/polkit qualification.

### Retained remote CI boundary

At pushed `fe8a4177`, the CI engine gate (`_build/verification/ci-engine-warnings-20261113/gate-20261113T011857Z/result.json`)
records strict Clippy 0, compile 0 and **31/0/0**: recursive 21 + link 10, zero
source drift. These overlap earlier feature gates. The unchanged complete Clippy
step now precedes long workspace tests. Workflow validation (`_build/verification/ci-engine-warnings-20261113/workflow-static-validation.json`)
passes exact-item-move and step-sequence checks with all other YAML bytes preserved;
no full YAML parser was invoked and no full-suite pass follows.

The later recorded [run 43345016027](https://github.com/TomasJelinek/fm/actions/runs/43345016027)
has **18 successful / 2 failed host jobs**. Both hosts stop at `needless_range_loop`
in `crates/midnight-viewer-ui/src/image_presentation_tests.rs` lines 682/834.
The two-loop mechanical iterator correction is frozen at SHA256
`706f96f00ea7dcca842aa76ae44b596f2d2747463afe0501bdc91f5a5cf20369`;
the combined prewarm compile/Clippy attempt failed before tests at legacy `Service`
fixtures. Corrected R2 compiles with zero drift and has 43 unique deterministic
passes; workspace Clippy still exits 101 on the unrelated copy-dialog test
`type_complexity` warning. Its UI alias-only correction is applied; final
fixture/lint gate passes compile/strict Clippy 0 and host14/0/0, zero drift. This is a test-only change,
not a viewer behavior or performance correction. The earlier receipts below keep
their original source/failure scopes.

The initial local Linux UI compilation stopped on missing `pkg-config`; **no tests
executed in that attempt**. After documented CI prerequisites were resolved without
feature changes, the local Linux gate passed **13 selected / 13 passed / 0 failed /
0 ignored**, compile 0 and `source_drift=[]`: `tree_clipboard` 5, `tree_copy` 2 and
`tree_move` 6. Exact receipt:
`_build/verification/backend-tree-link-refresh-20261113/linux-green-prerequisites-retry-20261112T235508Z/result.json`.
This is WSL/tmpfs software UI/filesystem evidence, not Linux desktop or remote CI
qualification. The initial prerequisite failure is not a runtime result.


At exact `1714d99`, the [Linux CI job](https://github.com/TomasJelinek/fm/actions/runs/43324388443/job/131555743277)
records `tree_clipboard` **3 passed / 2 failed / 0 ignored**. The stale
[tree_filters helper](../crates/midnight-ui/tests/support/tree_filters.rs) expected
a separate native child instead of the Main embedded confirmation; failure occurs
before Return and native Move/byte assertions. The helper correction is committed in `1546030`; the local **13-test Linux runtime gate passes 13/0/0** with zero drift. Available evidence
is `_build/verification/build-20261113/ci-first-failure-diagnosis.json` and
`github-checkpoint-1714d99-job-131555743277-20261112T232014Z.json`.
This job did not reach the Linux recursive target;
its separate WSL/tmpfs 1/0/0 receipt remains independent.


The older retained terminal remote receipt is
`github-checkpoint-8b18892-terminal-20261112T221914Z.json`:
[run 43312251721](https://github.com/TomasJelinek/fm/actions/runs/43312251721) at
`8b18892` completed with **18 successful / 2 failed host jobs**. Both hosts
failed three `operations_native_ui` assertions at
`tests/support/operations_native.rs:193` (`get_dialog_input_blocked()`).
The later integrated Windows target passes 4/0; Linux confirmation/current-main
remote success remains separate. No retained terminal receipt qualifies
`1714d99` as a green remote run.

Earlier `955068c` Native Jobs notice failures, `35a159c` compact-feedback
failures and `2e0e554` operation/Filter failures remain dated history. Local
selected notice/operation/compact passes do not rewrite those CI outcomes or
qualify later targets that a failed run never reached. CMD-02's source timing
question was resolved by `cmd02-corrected-provenance.json`; its 37/0 gate remains
a targeted Windows result, not a full suite.

### Completed build-script integration

For merge `28cc79add548eed0d6c9e465a6fe26a5d141f9b7`, the Orchestrator reports:

| Gate | Final reported result |
| --- | --- |
| Upstream standalone helper suite | 20 passed |
| Combined helper suite | 46 cases: 45 passed, one platform skip |
| Windows hooks | Seven passed |
| Shell syntax | Six script checks passed |

These groups are not added together as unique coverage. They are helper/hook/syntax
results, not a live package-installation, Docker-image, application or native UI
qualification. Documentation did not rerun them. Build implemented the
[cleanup mount guard](../scripts/check-clean-mounts-linux.sh) fixes after the
Orchestrator's diagnosis and review: kernel escapes use
exactly three octal digits, and decoding preserves following filename digits and
trailing path newlines.

### QA artifact cleanup

The user explicitly authorized deleting the entire `_build/qa/` tree; Build owns
the operation and its space measurement. QA paths retained below are historical
provenance, not a promise that those files remain available or a prerequisite to
reading this durable summary. New final documentation checks and seven-file hashes
are stored under `_build/verification/docs-20261112/`, outside QA. Cleanup is
complete: the Orchestrator verified `_build/qa/` absent. Build's accessible stage
removed **681 roots**, with a reported space gain of **40,625,377,280 bytes**
(about 40.6 GB). The reviewed frozen UAC helper completed the protected remnants
at **17:15:21 CET (16:15:21Z)**, using per-directory ACL repair without traversing
reparse targets. Its final-stage free-space change was **155,648 bytes**; the final
free-space snapshot was **46,807,683,072 bytes**. These observations are not summed
because unrelated disk activity continued; the earlier four-BMP cleanup is separate.
The completion receipt is
`_build/verification/20261112-cleanup/qa-elevated-completion.json` (`completed`,
`directoryAbsent: true`). Protected debug SHA-256 before and after is unchanged:
`d731397f751647c5feb6ff2ba6ec822a598711bc48ee738e452f364363138984`.

Source checks used for this reconciliation include:

| Area | Current source / contract | Boundary to preserve |
| --- | --- | --- |
| Filename palette | [Defaults](../crates/midnight-core/src/filename_colors.rs), [palette contract](developer/FILENAME_COLOR_OPTIONS.md) | Twelve ordered categories; PDF base `#EF5A5A`, DOCX/Markdown base `#C4CFE0`. Rendered contrast may adjust bases. Filename coloring does not imply decoder support. Exact historical defaults migrate; custom rules survive. |
| Selection defaults | [Shortcut catalog](../crates/midnight-core/src/shortcuts/catalog.rs), [tree row policy](../crates/midnight-core/src/tree_view.rs) | Ctrl+D clears every mark, including folders/expanded descendants; Ctrl+Shift+D has no default. Ctrl+A/Ctrl+Shift+A replace top-level selection and clear nested marks; Ctrl+Shift+I inverts top-level entries while retaining descendant marks. Saved deliberate overrides remain authoritative. |
| Captured inline actions | [Tree panel contract](developer/TREE_PANEL.md), [current status](FEATURE_STATUS.md) | Typed native consumers exist for transfers, clipboard/drag, focused editor, mutations/metadata, Properties/context menus, checksums, file lists and comparisons. F3/file comparison use visible root files; inserted branches still contain folders. Archive capability limits remain explicit. |
| Transfer policy and order | [Verification enum](../crates/midnight-core/src/transfer_verification.rs), [copy worker](../crates/midnight-engine/src/copy_worker.rs), [native Move](../crates/midnight-engine/src/move_native.rs), [transfer contract](developer/TRANSFERS.md) | Serial bounded copy, immutable full/quick policy, same-volume rename and per-file source removal. Neither quick mode nor native rename weakens identity/consent/publication checks. |
| Filter/drive/status | [Panel controls](../crates/midnight-ui/ui/panel-view-controls.slint), [filter presenter](../crates/midnight-ui/src/presenter/filter.rs), [drive information](../crates/midnight-ui/src/presenter/drive_information.rs), [folder totals](../crates/midnight-ui/src/presenter/panel_details.rs) | Filter icon/applied state, backend volume information and omission of unmeasured folder totals already exist. Their current native usability and qualification are separate from implementation presence. |

The subsequent Viewer handoff records **62 independent Markdown plugin tests
passed, zero failed, one manual measurement target ignored**, strict Clippy and a
candidate DLL build. Bounded page selection now borrows retained semantic context
and clones only selected complete layout groups; cache/read/wire/Unicode bounds
are unchanged. The one-line forward allocation regression fell from **452,189 to
8,383 bytes**, with no additional source reads. Saved response streams from the
two fixture comparisons were byte-identical. Diagnostic timing was mixed, including
a slower small retained fixture; this is not an end-to-end speed or frame-pacing
improvement claim. The candidate was not installed by that handoff. Its debug DLL
SHA-256 was `1F3C04FCD6E4DF7FFC478DDDF32A1E68D7672D92349620819317A37B9F2715BD`.
The original `_build/qa/viewer-markdown-20261112/handoff.txt` and candidate may be
removed by the authorized full QA cleanup; the counts, allocation result and scope
are preserved here. The Orchestrator checked the source receipt and accepted
those five plugin files as `cc4857e`. Fresh host/native integration must rebuild
the candidate and supply its own evidence.

## Requirements and evidence backlog

Priorities retain the existing requirement IDs. P0 means an integrity prerequisite
before enabling a dependent action; P1 means functionality/reliability; P2 means
qualification or measured optimization. Owners below are the appropriate roles
for the next assignment, not permission to start unreserved edits or builds.

### Implementation, contract and performance work

| IDs / priority / kind | Verified basis and remaining work | Next acceptance / role |
| --- | --- | --- |
| VFS-01, PFS-01 / P1 / public API/SDK source integrated; native/host cutover open | Public [ABI 1.3](../filesystem-api/midnight-filesystem-plugin-api/src/abi.rs) and [operation family](../filesystem-api/midnight-filesystem-plugin-api/src/model.rs) now include optional prepared Rename alongside browsing/reads/storage and Activate/Refresh/ResolveText. Task 1 plus public docs are merged into D `e5cba5d`, with exact 33-file equality. [App composition](../crates/midnight-explorer/src/main.rs) still injects native MutationService/WatchService separately; Windows/Linux native WIP remains in C. | Architect + Backend: retain the source-integration boundary while qualifying native Task 2/3 and later host routing. No new provider Rename capability/deployment or host cutover is claimed. Reject stale/foreign/unsupported targets without native/TEMP substitution; plugins share only public APIs and own tests. |
| TREE-01, ARCH-02 / P1 / implementation | [Tree contract](developer/TREE_PANEL.md) includes implemented bounded recursive expansion (`ce9eff3`, Windows UI 69/0 and overlapping Backend refined 11/0) and captured consumers; physical Linux desktop qualification/larger-than-model paging remain open. Production container discovery/attachment, archive watches and nested mounts remain separate gaps: [attach API](../crates/midnight-engine/src/providers/tree.rs) has test callers, while [archive mount input](../crates/midnight-filesystem-host/src/session.rs) still takes a native captured file. | Backend + Architect: add provider-content mount leases, exact parent anchors, bounded aggregate nesting/decompression/cache, invalidation and cancel/retry. Prove native → archive → archive → viewer, outer-source changes and concurrent solid readers; qualify supported tree actions without removing unsupported-action guards. |
| TREE-01 / P1 / DirectoryLink maintenance implemented; Windows qualified | Parent-first loaded native-link refresh, exact acknowledged watch scope, physical-identity notification deduplication with separate logical aliases, and retarget-aware subscription/poll validation pass the Windows gate: 51 selected/passed, 0 failed/ignored, compile 0, zero drift. Disjoint groups: provider 10 + refresh 25 + watch 15 + real Windows junction 1. `build-green-result.json` and `build-green-runtime-result.json` under `_build/verification/backend-tree-link-refresh-20261113/` establish this scope. Prior RED 2/3/0 is historical. | Retain generation/exact Commit, failed-parent descendant suppression, cycle/retarget and logical-action safeguards; no write/ACL/elevation changes. Updated Linux-native fixture passes 1/0/0 on WSL/tmpfs with frozen overlays and zero drift. The separate local Linux UI gate passes 13/0/0; new debug compilation and registered private Windows startup pass. The earlier exact-`1714d99` Linux 1/0/0 fixture does not qualify this updated source. |
| TREE-01 / P1 / cap128 source integrated; synthetic gate passed | [Cap128](#recursive-tree-cap128-gate) at `2b28cc7`, integrated by `02b2f2f`, passes 25 targeted functional cases, strict Clippy 0 and one opt0 synthetic aggregate. Four timed fixtures complete with 8/80/80/79 publications; the preceding cap16 95/0/0 and failed aggregate remain historical. | Perf + Backend: retain cycle/retarget, exact Commit, cancellation and captured-action guards. No new baseline, release/native filesystem, UI paint or visible-smoothness qualification is established; the `c6c371bc` runtime predates both increments. |
| COPY-03 / P1 / implementation | [Metadata policy](../crates/midnight-core/src/transfer_metadata.rs) exposes ContentOnly and required exact modified time for up to 64 native regular-file roots. Full metadata/directory metadata and explicit link-object copying remain unavailable. | Backend: define capabilities for remaining times/attributes/ACLs/xattrs/streams and postorder directory metadata; unsupported kinds fail before writes where knowable. Verify required values before publication and retained sources on failure; link copying must preserve links without following referents. |
| TERM-01 / P1 / implementation | [Terminal service](../crates/midnight-core/src/desktop.rs) is launch-only and [Linux launch](../crates/midnight-platform-linux/src/desktop.rs) spawns a new process/TTY. The [reusable-session requirement](superpowers/specs/2026-11-04-terminal-sessions-design.md) remains open. | Backend: one unprivileged bounded broker with kernel-verified process/UID/lifetime identity; distinguish Idle/Busy/disconnected/unknown outcome and never replay uncertain commands. Qualify a concrete terminal; sudo owns authentication/expiry, with no stored password, policy change or keepalive. |
| FIND-01/02 / P1 / Open report unreproduced; durable regressions pass | The [validator](../crates/midnight-engine/src/find_worker.rs) compares raw path/kind/native identity and six revision words. The [current diagnostic and durable gate](#native-link-presence-and-find-gate) records 13/0/0 with the exact user fixture unchanged and no differing identity/revision, followed by 6/0/0 durable regressions with production Find unchanged. The original report's build and captured metadata are unknown; no false-stale cause or production fix is proven. | Backend + Build: if the report recurs, retain exact runtime/session identity and captured/current raw path, kind, identity and revision words before selecting a correction. Preserve replacement/stale-session rejection. UI Details is the query report, not a second file-Properties action. |
| NET-01 / P1 / implementation | Discovery and typed discovery Hot Paths exist; [Linux GIO](../plugins/midnight-filesystem-network/src/linux/gio.rs) explicitly aborts interactive password challenges. Live SMB/GVfs interoperability is a separate qualification gap. | Backend + Architect: host-owned ephemeral authentication/cancel flow with no credential persistence/logging. Cover stale/canceled auth, reconnect and provider removal; then qualify an owned live endpoint. |
| FILTER-01/02, UI-01 / P1 / targeted positive-input gate passed | The retained replacement-popup focus assertion passes. Native Filter 17/0, Filter library 2/0 and exact operation-lifetime 1/0 cover 20 distinct tests; exact-first replay is not added twice. Find geometry passed with unchanged source. Private engine/native UI startup subsequently passes. | Preserve verified input/focus/lifetime and source identities. The original shared hashing anomaly remains preserved/unattributed after a passing engine retry; full native/layout/platform acceptance is not closed by these focused tests or startup smokes. Retain real pointer/key, occurrence identity, history/clear and stale-token coverage. |
| SELECT-01 / P1 / fixture correction implemented; subset passed | [Flat](../crates/midnight-ui/tests/bulk_selection.rs) and [tree](../crates/midnight-ui/tests/tree_bulk_selection.rs) definitions match all-marks Ctrl+D and default-unassigned Ctrl+Shift+D in `2e0e554`; their recorded binary runs passed 1 and 8 tests. The earlier Linux CI failure remains historical and the combined current diagnostic failed elsewhere. | Build: finish the targeted both-pane/native/archive integrations and record exact source/artifact/commands/results. Preserve explicit files-only action/remap coverage and the wider native/layout/drag qualification requirements; this subset does not establish a full selection or remote CI pass. |
| CMD-02 / P1 / corrected Windows gate passed | Checkpoint `21c9c7a` records the six accepted UI files. Corrected gate: 37 passed / 0 failed / 0 ignored across five artifacts, zero source drift, completed 21:16:16 CET. Natural FocusOut/caret and explicit active-size-bit fixtures pass locally; initial 31/5 setup failures remain historical. Captured Custom Commands remain separate; the current private `c6c371bc` EXE passes startup, not all command workflows. | Preserve literal insertion, acknowledged occurrence identity, raw names, focus, stale rejection and shell-draft grammar. Build: retain the exact receipts and qualify remaining Linux/native command-workflow boundaries separately. Do not derive authority from displayed archive paths or execute inserted text; no remote CI fix is inferred. |
| STATUS-02, PFS-02 / P1 / terminal styling implemented; targeted UI tests pass | Completed partial totals retain `≈`, yellow warnings and details with normal metadata number styling; only actively unfinished totals are highlighted, even while another root runs. Earlier recorded size/model library tests, `size_errors` 2/0, `panel_warning_details` 4/0 and `tree_folder_sizes` 7/0 passed. | Preserve these guarantees through remaining UI fixes without extending those earlier receipts to changed source, including mixed running/finished, warning-completed and cancelled/incomplete states. Full native/layout/platform qualification remains in the UI acceptance row below; these software UI passes do not close it. |
| SCROLL-02, VIEW-PERF-01/02, MEDIA-03 / P1 / source integration accepted; native qualification open | Corrected Markdown is committed at `255fbdc`, with 72/0 and strict Clippy. Perf accepts source integration from 32 paired debug profiling passes with exact output equivalence: large-fixture request wall time falls about 23 ms → 11–15 ms, while per-request allocator peak rises 383,320/461,739 bytes and small-fixture p95 rises 0.038–0.298 ms. [Retained-seek report](verification/2026-11-12-retained-complete-page-seek.md) retains its dated XML/refill and native boundaries. | Viewer + Build: establish matching packaged-worker/native evidence. Native scrolling/deployment, release/frame pacing, process memory/RSS, DPI/viewport and scroll-back qualification remain open under coordinated leases. Matching deployment/interactive evidence is still required; backend wall-time measurements do not close these requirements. |
| SCROLL-02, VIEW-PERF-01/02 / P1 / native Source drag unresolved | The archived one-off Source probe completed 1,098 operations/732 exact replays without mismatches; it measures broker/codec work, not the user's 134-line C++ scrollbar input-to-paint path. Portable panel regression tests `3bee1dc` 4/0 and thumbnail correction `8e205c0` 14/0 do not qualify that separate viewer path. | Viewer + UI + Build/Perf: capture the actual native scrollbar gesture, dispatch, request, layout/render and presentation phases on an owned unchanged fixture; attribute the observed delay before choosing a fix. Preserve source identity, cancellation and bounded provider I/O; do not infer a rendering improvement from protocol replay. |
| LIFE-01 / P1 / source review + qualification | [Lifetime contract](developer/ENGINE_LIFETIME.md) and later service/mailbox/report/configuration reports supersede the first retirement plan's narrower status. Exhaustive closure remains unestablished. | Architect + Backend: reconcile each retained owner/task/channel/last-release path against current source; exercise first-poll/drop, blocked dispatch, shutdown and completed-unconsumed outputs with native-before-provider destruction. A blocked destructor retains real capacity; no weakening or unbounded cleanup fallback. |
| IO-01 / P2 / benchmark prerequisite + implementation | [Transfer contract](developer/TRANSFERS.md) and copy worker remain serial with bounded 256 KiB steps. [Same-SSD probe](verification/2026-11-10-ssd-copy-probe.md) is incomplete and no-go for enabling concurrency from that run. | Backend + Build: complete stable 1/2/4-stream full-verification measurements, then positive local-SSD endpoint classification and shared physical-device budgets. Unknown/HDD/network stay serial. Cover aliases, A↔B fairness, small/mixed/large files, UI p95/p99, failure/cancel and retained in-flight ownership before raising defaults. |
| PLAT-01 / P1 / implementation + native dependency | [Application platform selection](../crates/midnight-platform/src/lib.rs) rejects non-Windows/Linux targets; a separate [read-only Darwin provider](../plugins/midnight-filesystem-native-macos/IMPLEMENTATION_PLAN.md) already exists. | Architect + Backend + UI + Build: reuse that provider and implement separate macOS app services, secure loaders/containment, input/clipboard/windows/terminal, mutations, packaging and CI. Native hardware/runner evidence must establish supported OS/architectures; its absence does not block independent source work. |
| PLAT-03 / P2 / implementation | [Linux external drag](../crates/midnight-platform-linux/src/file_drag_drop.rs) explicitly requires X11/XWayland; native Wayland lacks the same-connection seat/data-device bridge. | Backend + UI: implement bounded raw-identity/admission/cancel/lifetime integration and qualify native compositors. Internal dragging and XWayland results do not establish native Wayland external transport. |

### Provider increments from the Architect review

The accepted Rename API/SDK Task 1 (`f3f8c59`) and public docs (`852ee0c`) are now
source-integrated into D `e5cba5d` with exact 33-file equality.
Their tests, strict Clippy and consumer checks retain their recorded source scopes.
Windows/Linux native candidates in C retain separate receipts and are excluded
from the retained private runtime. No native prepared-Rename capability deployment
or host cutover is established.
The public [native protocol](../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md)
explicitly grants no mutation authority to read handles/revisions. Its existing
Refresh invalidates discovery TTL caches; public watch subscriptions/general change
events remain missing, rather than all invalidation being absent. Archive staging
writes host-owned private storage and does not enable archive mutation.

<a id="native-rename-audit-gates"></a>

The later C native Rename audit-fix snapshot passes Linux **38 selected /
38 passed / 0 failed / 0 ignored**, compile 0 and zero source drift, under real
Linux WSL/tmpfs. The exact receipt (`_build/verification/backend-rename-audit-fixes-20261113/linux-green-20261113T014351Z/result.json`)
and raw logs cover recovery 23 + prepared Rename 9 + conflicts 6; no fixture
paths remain and compile/test children stopped with source/target leases released.

The preceding Windows gate (`_build/verification/backend-rename-audit-fixes-20261113/windows-green-20261113T012845Z/result.json`)
compiled successfully with **21 passed / 1 failed / 0 ignored** from 22 selected,
zero drift. Both spelling regressions passed; the remaining failure was exact
DACL/control restoration in the test fixture. Its fixture-only correction (`_build/verification/backend-rename-audit-fixes-20261113/windows-final-green-freeze.json`)
is applied without changing production ACL/spelling code. The same 22-case repeat (`_build/verification/backend-rename-audit-fixes-20261113/windows-fixture-green-20261113T014954Z/result.json`)
passes **22/0/0**, compile 0, zero source/binary drift and no remaining temporary
entries. Recovery/unit 12 + conflicts 6 + prepared Rename 4 retain the original
selection; exact file/directory ACL readbacks are iterations of one existing test,
not additional cases. Processes exited and source/target leases were released.
The preceding 21/1 remains historical. Task 4A's exact candidate is accepted
independently by Root and Architect and applied in C. Its reservation is exactly
`admission.rs`, `ffi.rs`, `discovery.rs`, `ffi_tests.rs` and
`rename_admission_tests.rs` under `crates/midnight-native-filesystem-host/src/`.
The applied host freeze (`_build/verification/backend-native-host-task4a-20261113/applied-freeze.json`)
records all five applied hashes matching candidate manifest
`c8d2463d902a41a6a79fc0ef5834b29ec8307e2504d783a9b1976a83d6c021ae`.
The Build handoff (`_build/verification/backend-native-host-task4a-20261113/applied-build-handoff.md`)
selected **44 unique direct host tests** (admission 1 + discovery 14 + FFI 29).
The fresh C host foundation receipt (`_build/verification/backend-native-host-task4a-20261113/build44-20261113T020849Z/result.json`)
records compile **0**, Clippy **0**, test exit **0**, **44 passed / 0 failed /
0 ignored** and **252 captured inputs / 0 drift**. The test executable's before/
after SHA256 is
`a965fa7cc8e593530e6079b4ad7d6c89704a4eca991a75b690be78b0ed05f196`;
the child exited and source/target leases were released. This qualifies the C-only
host foundation, not deployed application Rename. Root checkpointed these five
reviewed/qualified host files as `eb2eca8` on C branch `codex/main-deep-audit`;
the 44/44 and scoped Clippy receipts apply unchanged. Only those five files were
committed. Root later integrated this host foundation and Task 4B `cf64a6c` in
D merge `02b2f2f`. Unfinished native-plugin sources remain separate; source
integration does not establish application deployment.

After Architect agreement, Root accepted Task 4B's finite finish seam with an
owned prepare-failure carrier. Its **11 C core/host paths** (2 core + 9 host) are
committed as `cf64a6c`; the earlier
reservation baseline (`_build/verification/backend-native-host-task4b-20261113/baseline.json`)
records those paths. The [core/host contract](developer/NATIVE_FILESYSTEM_API.md#prepared-rename-corehost-lifecycle)
and [disposal boundary](developer/ENGINE_LIFETIME.md#prepared-rename-disposal-boundary)
describe the implementation; engine/UI integration remains unimplemented.

The preceding Windows/Linux core/host harness snapshots each passed **79/0/0**
with compile **0**, while strict Clippy exited **101** on deprecated `fetch_update`.
The atomic-helper recheck passed one overlapping case per platform but exposed
the test-helper `large_enum_variant` lint, also Clippy **101**. After boxing that
helper, the final narrow gate (`_build/verification/backend-native-host-task4b-20261113/test-helper-boxing/build-owned/combined-result.json`)
passes the same two exact cases on each platform: **2/0/0 Windows + 2/0/0 Linux**
(**four executions**), compile/strict scoped Clippy **0**, **28 stable pins**,
zero source/pin drift, all owned compiler/test children stopped and leases
released. The earlier 79/79 remains preceding runtime evidence, not a new
83-unique-case result or a broad 79-case rerun after boxing.

Root integrated Task 4A `eb2eca8` and Task 4B `cf64a6c` into D at `02b2f2f`.
These core/host harness gates do not qualify installed-provider/native runtime Rename. Engine/UI cutover,
physical Linux desktop and installed host workflows remain open; no native
Rename deployment is established.

| Gap | Finite scope and required future evidence |
| --- | --- |
| Prepared actions / first VFS-01 Rename slice | Accepted: optional, versioned single-item, same-provider, same-parent prepared Rename. Explicit preparation captures identity before confirmation; it does not guarantee identity from listing time or change the namespace. Opaque source/parent authority retains raw basename, prepared identity/revision, bounded admission/cancel and worker-owned cleanup through refusal, stale delivery and shutdown. Task 1 `f3f8c59` and public docs `852ee0c` are merged into D `e5cba5d` with exact equality of 33 incoming files. Five Architect findings are source-resolved; corrected prepared-Rename tests and strict API/SDK Clippy pass. Direct Windows/network consumer compilation checks pass; Windows Task 2 and Linux Task 3 source work is authorized in C. Native candidates remain WIP in C with [selected audit-gate receipts](#native-rename-audit-gates); Task 4A is accepted/applied within its five reserved C host files with the direct C host foundation gate passing 44/0/0, compile/Clippy 0 and zero drift, while Task 4B's finite finish/owned prepare-failure implementation is committed as `cf64a6c` in 11 C core/host paths with [preceding 79-case and final narrow gates](#native-rename-audit-gates); D source is integrated at `02b2f2f` and engine/UI cutover remains unimplemented; installed/native qualification remains incomplete, with no deployment or host cutover claimed. No-replace is the baseline; replacement must preserve current pinned/versioned/recoverable consent or remain Unsupported. No delete-first/copy/cross-device fallback. Public API/SDK and independent native plugins precede host cutover; no deployed native capability is established by these selected gates. Require compatibility/absent-capability/malformed-handle tests, native prepare/cancel/race/collision/raw-name/case-only/alias tests and installed flat/tree confirmation/queue/cleanup tests. Delete, metadata, transfer and watches stay separate slices. |
| Public watches/general invalidation / VFS-01 | Migrate existing [native tree watch scope](../crates/midnight-engine/src/tree_watch_scope.rs) through explicit provider subscription/revision/event/overflow contracts. Preserve bounded eligible loaded-branch observation, fair polling fallback where supported, dirty epochs through reconciliation and no implicit activation/authentication. Require expanded-empty growth, overflow, >32-branch fairness, late navigation/revocation events, worker cleanup and backing-archive revision invalidation checks. Existing explicit Network Refresh remains supported. |
| Preserved links / COPY-03 | Keep current rejection by default; propose explicit Preserve only for declared endpoint/link kinds. A distinct no-follow link proof captures raw payload/kind/flags, exclusively stages and independently reads back the link representation, revalidates source and publishes safely without opening the referent. Link Move needs separate retained finalization authority; it cannot reuse a regular-file content proof. Require relative/absolute/dangling/cyclic links, junction/kind/privilege limits, Unicode/raw names and untouched external sentinels, plus mismatch/replacement/conflict/cancel/cleanup failures. Hardlink topology and broader metadata remain separate capabilities. |

Proposed ownership: Backend owns public API/SDK/header, separate plugin/platform
implementations, host adapters and engine authority/proof integration; UI owns
capability presentation, drafts/confirmations and flat/tree UI regressions; Build
owns packaging/CI. Shared composition/`lib.rs` edits receive one explicitly named
owner. The Architect review itself was read-only; later source progress does not expand its recorded qualification.

### Qualification of implemented workflows

These are evidence tasks, not claims that the existing feature is absent or
broken. Commission only the missing pathway/environment after reconciling prior
receipts; injected failures, owned native probes, UI bridges and real application
gestures prove different boundaries.

| IDs / area | Evidence to retain | Remaining acceptance / role |
| --- | --- | --- |
| COPY-01/02/03, MOVE-01 / transfers | [Transfer contract](developer/TRANSFERS.md), [current full/quick/native Move evidence](verification/2026-11-10-transfer-verification-and-native-move.md), [panel batch](verification/2026-11-12-panel-interaction-polish.md) | Backend + Build: final integrated/release tests for same/cross-volume, merge/replacement/recovery, alias/source-change races, full/quick boundaries and corruption, exact required time, retry/cancel/cleanup. Real disk-full/disconnect coverage must be distinguished from injection and native Shell probes. Final Slint app → running Explorer drag remains separate from owned-window/Shell success. |
| TREE-01, FILTER-01/02 / captured action parity | [Per-action tree contracts/evidence](developer/TREE_PANEL.md), [bounded mailbox qualification](verification/2026-11-12-panel-interaction-polish.md) | Backend + UI + Build: installed native/archive workflows for each enabled consumer, both panes, root/descendant/mixed/hidden/stale targets and queue rejection. Native clipboard desktop interoperability, Linux desktop input and complete search/filter/sort/action parity remain separate. Do not reimplement consumers merely because old audits list them as guarded. |
| ARCH-01/04, HOTPATH-02, MEDIA-01/03 / archives and previews | [Archive API/limits](developer/FILESYSTEM_API.md), [streaming workers](verification/2026-11-09-archive-streaming.md), [cooperative thumbnail cancellation](verification/2026-11-10-archive-thumbnails.md), [native Hot Paths manager](verification/2026-11-11-native-tool-windows.md) | Backend + Viewer + Build: real-plugin full-app menu/shortcut/restart/restore/unavailable-source Hot Paths, large/ZIP64/solid/all-format paths, private-storage failure and release packaging. Qualify actual native panel scroll-back/cache reuse and stale provider revisions; protocol-3 installed worker checks do not cover every Linux/archive-panel desktop workflow. |
| FIND-01/02, HASH-01 / tools | [Find](developer/FIND.md), [captured Find actions](verification/2026-11-12-find-actions.md), [checksums](developer/CHECKSUMS.md) | Backend + UI + Build: current installed tool windows, roots/history, exact captured result actions, manifest restrictions, export/partial consent, stop/stale/shutdown and small-window behavior. Archive Find and provider-wide checksum support require separate implementation contracts. |
| CMD-01/02/03, OPTIONS-02, VIEW-03 / persisted settings and commands | [Tree commands](verification/2026-11-12-tree-custom-commands.md), [Cmd scope](verification/2026-11-12-cmd-custom-commands.md), [viewer preferences](developer/VIEWER_PREFERENCES.md) | UI + Backend + Build: actual Add/Edit/Preview/Cancel/Save/restart, exact argv/cwd and selected/focused identity, icon lifecycle and terminal behavior. Cmd `_ARG` direct argv evidence does not qualify arbitrary CALL/builtin reparsing. Viewer manager recreation is narrower than an app restart. |
| DIALOG-02, META-02, SELECT-01, PANEL-02, NAV-01/02, NOTICE-02, THEME-02, ICON-02, DRIVE-02, STATUS-02 / UI polish | Embedded modal/focus/input source and corrected selection fixtures are in [current checkpoint scope](#current-checkpoint); earlier [panel/selection/palette evidence](verification/2026-11-12-panel-interaction-polish.md) retains its dated scope | UI + Build: qualify normal/320×200, desktop/touch, fonts/DPI/themes, input/focus/IME, resizing, X/Escape and captured callbacks. Ordinary modals stay embedded; independent tools retain native hosts. Preserve warning Copy/retry/close and the tested finished-partial-size presentation. Render matrices do not substitute for native/compositor checks. |
| VIEW-01/02, SCROLL-02 / viewers and comparison | [Viewer presentation](developer/VIEWER_PRESENTATION.md), [image limits](developer/IMAGE_VIEWER_API.md), [HTML contract](developer/WEB_VIEWER_API.md), [foreground evidence](verification/2026-11-12-foreground-scroll-resume.md) | Viewer + UI + Build: native text/hex/source/Markdown/image/PDF/HTML opening/navigation/search/copy/zoom/font/persisted settings, exact user fixtures, native/archive content and text/image comparison. Qualify Linux frame pacing, physical IME, native Chromium Wayland and configurable browser key arbitration separately; preserve bounded page selection and inert content. |
| NET-01, PLAT-02 / native environments and permissions | [Network contract](developer/NETWORK_FILESYSTEM.md), [system targets](SYSTEM_REQUIREMENTS.md), [directory authorization](verification/2026-11-11-directory-authorization.md) | Backend + UI + Build: controlled live SMB/GVfs/FUSE, Windows 10, physical Linux desktops/distribution packages, GPU/driver/filesystem and multilingual/RTL/IME evidence. CSC warning/UI and Engine mock gates pass, but actual privileged CSC access, accepted UAC/polkit prompts and physical Linux desktop workflows remain unqualified. The original recursive symlink fixture is verified 1/0/0 under WSL/tmpfs at `1714d99`, with compile/test exit 0. Record exact environment, reduced-RAM measurements and artifact notices; WSL/compilation is narrower evidence. |
| CI / complete current validation | The [latest recorded remote boundary](#retained-remote-ci-boundary) is run 43345016027: 18 successful/2 failed host jobs, both on the two image-presentation `needless_range_loop` warnings. Engine CI corrections at pushed `fe8a4177` pass Clippy/compile and 31/0/0; the mechanical image-test iterator correction is frozen, but the combined prewarm compile/Clippy attempt failed before tests at legacy `Service` fixtures; corrected R2 compiles with zero drift and 43 unique deterministic cases pass after the same-binary host-fixture rerun. R2 strict workspace Clippy still exits 101 on one unrelated copy-dialog test type-complexity warning; the UI alias correction is applied and the final R3 gate passes compile/strict workspace Clippy 0 with zero drift, host14/0/0 and 44 unique deterministic cases. The five passing headless native controls remain the retained R2 result. Earlier `1714d99` clipboard 3/2, `8b18892` host failures and local Linux UI13 retain their dated scopes. | Build + Root: obtain fresh frozen-source targeted/workspace and remote receipts. Structural workflow validation, selected local gates, C native-plugin gates, WSL fixtures and qualified `c6c371bc` startup do not establish full-suite/desktop success. The earlier cap16 functional95/Clippy0 and failed aggregate remain historical; the later integrated cap128 passes only its recorded functional25 and single synthetic aggregate. Native provider/host Task 4, viewer performance and manual permissions remain separate. |
| Dependencies / release inventory | [Dependency policy](DEPENDENCY_POLICY.md), [third-party inventory](THIRD_PARTY.md) and distribution notices | Every role + Documentation + Build: review actual pinned direct/transitive/native/test/build dependencies and enabled features on changes, verify free intended use/distribution and required notices/source offers. No trial/paid-only path or silent project-license change. No dependency changed in this documentation pass. |

### Retained extension contracts

The broader design still records these capabilities. They require separately
scoped contracts and ownership before enabling controls; existing intentional
bounds are not regressions. Candidate optimizations are not silently promoted
into mandatory product features.

| Existing IDs | Next concrete contract / evidence |
| --- | --- |
| ARCH-03 | Archive write/auth/export lifetime, outer-container staging/publication, independently verified members and ephemeral credentials. More formats/methods/multivolume support need specific requirements and fee-free dependency review; no automatic RAR obligation. |
| REMOTE-01, REG-01 | Typed remote endpoint/auth/read/watch/seek/mutation capabilities and a distinct Windows Registry key/value model. Network discovery is not FTP/SFTP transport; Registry values are not ordinary file-copy sources. Depend on VFS-01 and bounded, independently verifiable destination contracts. |
| VIEW-01, MEDIA-02, HASH-02 | Any whole-file selection, animation, wider PDF/provider comparison/editing, additional codecs/algorithms/hash-plugin API or persistent history needs a bounded contract and actual license/feature review. Existing page-local copy, still-image/codec and five-hash limits remain disclosed. |
| PERF-01 | Single-file parallel blocks, GPU hashing/decoding, BLAKE3 policy substitution, parallel mutations and universal process isolation remain proposals/evaluations. Require acceptance, measured benefit and ownership/security contracts; do not replace current SHA-256 merely because a microbenchmark is faster. |

## Corrections to stale staged references

These notes qualify unowned/historical documents; this batch does not rewrite
their evidence or authorize edits to another role's files.

| Reference | Current correction / unresolved handoff |
| --- | --- |
| [5 November stages](superpowers/plans/2026-11-05-follow-up-stages.md), stage 3 and opening sequence | Whole-batch-before-delete and unconditional hash/copy wording is superseded by per-file Move and the accepted full/quick/native-rename policy. Historical stage order does not suspend the current parallel seven-role assignments. |
| [Implementation backlog](IMPLEMENTATION_BACKLOG.md), tree introduction/consumer text | Native Delete/Rename/metadata, clipboard, focused F4, checksums, Properties/context menus, Make File List and native comparisons have captured paths. In particular the later sentence calling directory comparison guarded is stale; root-level native comparison is implemented. Generic provider writes, archive watches/attachment/nesting and complete native qualification remain open. |
| [11 November gap audit](verification/2026-11-11-open-requirements-audit.md), tree/dialog/media/network rows | This is the named older baseline. Later reports/source add captured consumers, protocol-3 cooperative archive staging cancellation and discovery Hot Paths. Blanket native-modal migration is superseded; interactive Linux auth remains a gap. Preserve the audit as history. |
| [Feature status](FEATURE_STATUS.md), Dialogs row, and [dialog ergonomics](developer/DIALOG_ERGONOMICS.md), native-modal examples | These descriptions predate the embedded modal migration in `2e0e554`. Current operation/copy/filter/Hot Paths/file-list/notice forms use the embedded presentation and focus/input gates above; independent modeless tools keep native hosts. Those unowned documents need a separately assigned correction; the positive-input gate now passes the retained-focus assertion and selected boundaries; private startup and shared engine retry pass their recorded scopes; the initial shared hashing anomaly is preserved/unattributed and broader native qualification remains open. |
| [Initial retirement plan](superpowers/plans/2026-11-12-runtime-independent-retirement.md) and LIFE-01 backlog ledger | Later service/checksum/command/mailbox/report/configuration work exists. The backlog retains both 324 and an older 308 slot figure; [current lifetime contract](developer/ENGINE_LIFETIME.md) states 324. Architect/Backend must reconcile the current ownership ledger before closure, rather than declaring all retirement work missing or complete from checkboxes. |
| [Tree Move contract](developer/TREE_PANEL.md#move-and-transfer-reconciliation) | Corrected in this scope, including Copy/Paste wording: copied files use the frozen Full SHA-256 or explicit quick size plus first/middle/last 64 KiB policy; same-volume native rename reads/hashes no contents. Source identity, conflict consent, per-file removal and tree reconciliation guarantees remain intact. |
| [Transfer service](developer/TRANSFERS.md), ordinary Move/metadata descriptions | Corrected in this batch after source inspection: native same-volume rename precedes content proof; copied-file removal follows the immutable full/quick mode, with metadata/identity guards retained. Full SHA-256 remains the default, not an unconditional requirement for every Move. |
| [Worker SDK](developer/WORKER_SDK.md), startup/version descriptions | Corrected against package/source: worker SDK 0.2.0 accepts legacy MWS1/MWR1 and explicit MWS2/MWR2 resource policies. Thumbnail API remains 0.1.0 while its SDK is 0.2.0; released viewer API/SDK 0.1.0 is a separate package family. No dependency or released package bytes were changed. |
| [Panel qualification final handoff](verification/2026-11-12-panel-interaction-polish.md) | Its final “ready for main” and paused-backlog text describes that handoff. The current Orchestrator assignment establishes integration and resumed scoped work; it does not expand the report's executed test/native/CI scope. |
| [Bulk selection fixtures](../crates/midnight-ui/tests/bulk_selection.rs), [tree bulk selection fixtures](../crates/midnight-ui/tests/tree_bulk_selection.rs) | Former expectations caused the recorded Linux CI failure. Fixture source is corrected in `2e0e554`; the completed-artifact diagnostic passed 1 flat and 8 tree tests. The combined result failed elsewhere; no all-integration or remote CI pass is inferred. |
| [README Controls](../README.md#controls) | Corrected under the explicit scope extension: Ctrl+D clears every mark, including inline descendants; Ctrl+Shift+D is unassigned by default. Ctrl+A/Ctrl+Shift+A and Ctrl+I/Ctrl+Shift+I retain the verified files-only/top-level-all scopes. |

## Evidence required to close work

An owner handoff supplies exact files and source/build identity, behavior/capability
scope, the actual commands and test counts including ignores/failures, environment,
artifact paths under `_build/`, native versus substituted boundaries and remaining
requirements. Behavior changes receive targeted failure/cancel/race tests at direct
integration boundaries; full validation follows the planned integrated batch.

Documentation closes an implementation claim only when source/API behavior matches
the latest requirement. Native/performance/platform claims additionally need their
own evidence. A completed review or local test group does not close the full goal;
unavailable native hardware is a qualification dependency, not a reason to stop
independent authorized implementation.
