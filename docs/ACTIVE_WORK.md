# Active work and requirements evidence — 12 November 2026

The user has resumed the goal: implement all accepted app features, test each
deeply and polish the UI. This is the current coordination and requirements
ledger, alongside [Feature status](FEATURE_STATUS.md). It reconciles the older
[requirement IDs](IMPLEMENTATION_BACKLOG.md) without treating their stale status
text or stage ordering as current evidence.

Reviewed source checkpoint through **`e6e086c`**: Task 1/docs merged at
`e5cba5d`, accepted Markdown source at `255fbdc`, then HTML diagnostics at `e6e086c`.
Qualified runtime/UI source checkpoint: **`35a159cd00a5749f53d7f3522c64566e2c46a776`**.
Latest completed remote-CI checkpoint: **`2e0e554`**. The qualified source includes embedded modal presentation
with focus/input gating, corrected selection fixtures and shared cancellable font
catalog cold initialization. The positive-input UI gate passes **20 distinct
tests**. The new debug EXE compiles and its **private entrypoint** passes engine
smoke and native UI startup, both exit 0 with unchanged fixtures. The shared D
engine retry also passes. Its initial hashing failure remains preserved and
unattributed, with no source/validator fix; later stable-handle probes pass.
Corrected Markdown is 72/0 plus strict Clippy but is not deployed in the qualified
private runtime. Task 1 and its public docs are source-integrated into D; native
Task 2/3 and host cutover remain open. Controlled Markdown debug A/B measurements
are recorded below; Perf accepted source integration, with native qualification
and deployment still open. No new remote CI success is recorded.

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

## Current seven-role batch and exclusive ownership

The [agent workflow](developer/AGENT_WORKFLOW.md) and [project rules](../AGENTS.md)
govern assignments and handoffs. The documentation paths below are an explicit
file reservation. Other rows name the assigned domains; concrete source paths
must be recorded by the Orchestrator before shared edits, and a review alone
does not reserve or authorize edits throughout a directory.

| Owner | Current assignment | Ownership and coordination boundary |
| --- | --- | --- |
| Orchestrator | Coordinate the full goal; independently critique/review proposals, changes and evidence; own integration | Owns file reservations, API/UI integration decisions, Cargo target-directory/native-input leases and Git checkpoints. Commit/merge/push remain Orchestrator-owned. |
| Architect | Retain UI generated-code and accepted Rename contracts | The reviewed positive-input correction now passes the retained-focus regression and selected boundaries. Broader native qualification remains separate. Task 1's five findings are source-resolved; Windows Task 2 and Linux Task 3 source work is authorized in C; both remain unqualified, with host cutover separate. |
| Backend agent | Continue Windows Task 2 and Linux Task 3 native plugin WIP in C | Task 1 `f3f8c59` and public docs `852ee0c` are merged into D `e5cba5d`, with exact equality of 33 incoming files. Native platform WIP remains excluded/unqualified; no new Rename capability, runtime deployment or host cutover is claimed. |
| Viewer agent | Prepare the minimal native-worker handoff for accepted Markdown source `255fbdc` | Correctness is 72/0 plus strict Clippy. Perf accepted source integration using the 32-pass paired debug wall-time/allocator evidence, including heap and small-fixture tail costs. The worker is undeployed; native scrolling/deployment qualification remains open. |
| UI agent | Hold the tested positive-input source at pushed UI checkpoint `35a159c` | Selected boundaries pass (20 distinct tests), private engine/native UI startup passes and the shared D engine retry passes. The original shared hashing anomaly remains preserved/unattributed, with broader native interaction acceptance separate; CMD-02 source is frozen, with targeted runtime qualification pending. |
| GitHub build system and build tools developer | Preserve qualified launch receipts and the original shared failure; own Task 2/3 and Markdown performance validation | Private engine/native UI startup and shared engine retry pass. Later stable-handle checks do not identify the original changed field; no source/validator fix occurred. Corrected Markdown is not deployed. Preserve exact source/runtime identities, logs and coordinated leases. |
| Perf agent | Retain the accepted Markdown source boundary and review remaining native/deployment evidence | `markdown-ab-acceptance-review.md` accepts source integration, including measured allocation and small-fixture tail costs. Request timings use `Instant` wall time, not process CPU time. Native scrolling, release behavior, deployment and RSS remain unqualified. |
| Documentation guru | Maintain this compact current-state ledger only | Current edit ownership is only D `docs/ACTIVE_WORK.md`; other D/C docs remain Root-owned. Inspect source/log evidence without Git, Cargo or application runs. Freeze paths/hashes under `_build/verification/docs-20261112/` for Root's later commit. |

Native Windows Task 2 and Linux Task 3 remain WIP in the existing
`C:/Users/tjelinek/.codex/worktrees/main-deep-audit/FM` checkout and are excluded
from D integration. Build owns Cargo leases across C and D. Task 1 `f3f8c59`
and public docs `852ee0c` are now merged into D `main` at `e5cba5d`.
`_build/verification/build-20261112/api-sdk-integration-result.json` records
exact equality of all 33 incoming files and zero dirty overlap. Source gates
include API/SDK tests, corrected Rename tests, strict Clippy, direct Windows/network
consumers and the C host all-targets check, retaining each original receipt's scope.
This integrates public ABI/API/SDK source and docs; no native prepared-Rename
capability/deployment or host/UI cutover is established. The private EXE below
retains its pre-integration UI/source qualification.

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

### Qualified UI checkpoint `35a159c`

Root pushed the 33-file UI checkpoint `35a159c` and verified `origin/main` at the
same commit. Its sources exactly match the compiled/qualified private EXE. Plugin
WIP remains unstaged/excluded; the completed remote CI below still names `2e0e554`.

The positive-input gate passes **20 distinct tests**: native Filter 17/0, Filter
library 2/0 and exact background-operation retirement 1/0. Exact-first overlaps
the Filter group and is not added again. Find geometry passed earlier with
unchanged Find source. The debug EXE compiled successfully at **20:06:27 CET**.
Its private entrypoint then passed engine smoke and native UI startup at
**20:12:01 CET**, both exit 0 with unchanged fixtures. A shared D engine retry now
also passes, with later stable-handle probes. Its initial generic hashing failure
is preserved and unattributed; no source or validator fix occurred. These startup
results do not establish full native/HTML/performance or remote CI qualification.

| Area | Source scope / remaining evidence |
| --- | --- |
| Embedded modals | [Main presentation](../crates/midnight-ui/ui/main.slint), [operation/copy draft lifetime](../crates/midnight-ui/src/operation_windows.rs), [invoking-item focus restoration](../crates/midnight-ui/src/dialog_focus.rs) and [input gate registration](../crates/midnight-ui/src/lib.rs) implement the migrated forms. Modeless tools remain independent; native desktop/monitor qualification is separate. |
| Selection fixtures | [Flat](../crates/midnight-ui/tests/bulk_selection.rs) and [tree](../crates/midnight-ui/tests/tree_bulk_selection.rs) fixtures now expect all-marks Ctrl+D and default-unassigned Ctrl+Shift+D. The historical 22-binary diagnostic passed 1 and 8 tests respectively; this subset does not establish complete selection/native/layout or remote CI success. |
| Font catalog cold initialization | [Shared catalogs](../crates/midnight-viewer-render/src/fonts.rs) retain completed installed/monospace names process-wide and serialize cold misses per catalog. Waiting callers can cancel; cancelled discovery publishes no partial catalog. Work stays on backend workers and cancellation is cooperative around discovery. In the earlier renderer gate, the five [cache regressions](../crates/midnight-viewer-render/src/font_cache_tests.rs) ran within the 14-passing viewer-render library target; the separate monospace target passed 4 tests. |
| Current UI verification | Retained-focus/input/lifetime boundaries pass (20 distinct tests). Private engine/native UI startup passes, fixtures unchanged; shared D engine retry also passes. The original shared hashing failure is preserved/unattributed with no source/validator change. Earlier Find/size/model/warning evidence retains its own scope. |

The local gate is `_build/verification/build-20261112/ui-positive-qualification.json`,
with source identity in `ui-positive-source-before.json`. Root independently
verified all 33 UI hashes against the accepted manifest plus positive-input
overlay. Debug compilation is recorded in `app-debug-build-result.json` (exit 0).
The qualified private boundary is `private-qualified-debug-result.json`,
completed at 20:12:01 CET. Root independently inspected that result and the native
startup screenshot; the unchanged fixtures are part of its recorded scope.
Frozen identities are in
`_build/verification/docs-20261112/final-gate-handoff.json`.
Older failed UI gates remain historical. `qualified-debug-result.json` records the
shared engine retry pass and stable-handle probes. The initial failure remains in
`app-smoke-results.json`; its generic executable-hashing error does not identify
which field changed. No source/validator correction is claimed.

The corrected Markdown gate is **72 passed / 0 failed across 12 targets**.
Its strict Clippy receipt,
`_build/verification/viewer-20261112/candidate-corrected-clippy-result.json`,
also records exit 0 at **19:41:22 CET**. The correctness summary is
`candidate-corrected-correctness/summary.json` in that viewer evidence directory.
These results qualify the corrected source manifest; the candidate is **not
deployed** in the qualified private runtime. Controlled debug request wall-time
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

The new EXE is **101,542,400 bytes**, file timestamp **20:06:25 CET**, SHA-256
`0bf32ff3e13fefc19aae26cd35bc669129938f12f60b6a59f2cfa5cc420bd600`.
The qualified private entrypoint is
`C:/Users/tjelinek/MidnightExplorer-20261112-190803-0bf32ff3/me.exe`.
The same binary's shared path is `_build/debug/me.exe`; its engine retry passes.
The initial shared hashing failure remains preserved/unattributed despite later
stable checks, with no source/validator change. Desktop shortcut selection is
unchanged and the prior private runtime is retained. Corrected Markdown is not
deployed. Later API/SDK, Markdown and HTML source commits do not extend this
pre-merge EXE's qualification; no new runtime or prepared-Rename capability is claimed.

The [12 November panel qualification](verification/2026-11-12-panel-interaction-polish.md)
records bounded tree publication/command admission, captured actions, diagnostics,
root-level native directory comparison, current deselection defaults and the coral
PDF palette correction. It records final Windows UI library **984 passed / five
ignored**, Linux UI library **979 passed / two timing benchmarks ignored**,
additional targeted gates and final debug backend/native/software startup smokes.
Those are recorded results, not tests rerun during this documentation pass, a full
workspace/remote CI pass or accepted UAC/polkit qualification.

The Orchestrator verified [GitHub run 43260032611](https://github.com/TomasJelinek/fm/actions/runs/43260032611)
at `2e0e554` is complete: **18 successful jobs / 2 failed check jobs**. Both Linux
and Windows fail on the same two UI library tests:

- `operation_windows::lifetime_tests::embedded_operation_background_copy_retirement_keeps_owner_item_focus`
- `panel_filter_windows::tests::installed_filter_and_help_use_only_main_window_and_reject_retired_tokens`

Current frozen fixture corrections address both; the fresh local positive-input
gate passes the exact lifetime test and existing Filter module. No new remote
success is claimed. Earlier run `43239253816`, with its comparator timeout and
Ctrl+D fixture failures, predates this checkpoint and remains historical.

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
| TREE-01, ARCH-02 / P1 / implementation | [Tree contract](developer/TREE_PANEL.md) contains many implemented captured consumers. Production container discovery/attachment, archive watches and nested mounts remain separate gaps: [attach API](../crates/midnight-engine/src/providers/tree.rs) has test callers, while [archive mount input](../crates/midnight-filesystem-host/src/session.rs) still takes a native captured file. | Backend + Architect: add provider-content mount leases, exact parent anchors, bounded aggregate nesting/decompression/cache, invalidation and cancel/retry. Prove native → archive → archive → viewer, outer-source changes and concurrent solid readers; qualify supported tree actions without removing unsupported-action guards. |
| COPY-03 / P1 / implementation | [Metadata policy](../crates/midnight-core/src/transfer_metadata.rs) exposes ContentOnly and required exact modified time for up to 64 native regular-file roots. Full metadata/directory metadata and explicit link-object copying remain unavailable. | Backend: define capabilities for remaining times/attributes/ACLs/xattrs/streams and postorder directory metadata; unsupported kinds fail before writes where knowable. Verify required values before publication and retained sources on failure; link copying must preserve links without following referents. |
| TERM-01 / P1 / implementation | [Terminal service](../crates/midnight-core/src/desktop.rs) is launch-only and [Linux launch](../crates/midnight-platform-linux/src/desktop.rs) spawns a new process/TTY. The [reusable-session requirement](superpowers/specs/2026-11-04-terminal-sessions-design.md) remains open. | Backend: one unprivileged bounded broker with kernel-verified process/UID/lifetime identity; distinguish Idle/Busy/disconnected/unknown outcome and never replay uncertain commands. Qualify a concrete terminal; sudo owns authentication/expiry, with no stored password, policy change or keepalive. |
| NET-01 / P1 / implementation | Discovery and typed discovery Hot Paths exist; [Linux GIO](../plugins/midnight-filesystem-network/src/linux/gio.rs) explicitly aborts interactive password challenges. Live SMB/GVfs interoperability is a separate qualification gap. | Backend + Architect: host-owned ephemeral authentication/cancel flow with no credential persistence/logging. Cover stale/canceled auth, reconnect and provider removal; then qualify an owned live endpoint. |
| FILTER-01/02, UI-01 / P1 / targeted positive-input gate passed | The retained replacement-popup focus assertion passes. Native Filter 17/0, Filter library 2/0 and exact operation-lifetime 1/0 cover 20 distinct tests; exact-first replay is not added twice. Find geometry passed with unchanged source. Private engine/native UI startup subsequently passes. | Preserve verified input/focus/lifetime and source identities. The original shared hashing anomaly remains preserved/unattributed after a passing engine retry; full native/layout/platform acceptance is not closed by these focused tests or startup smokes. Retain real pointer/key, occurrence identity, history/clear and stale-token coverage. |
| SELECT-01 / P1 / fixture correction implemented; subset passed | [Flat](../crates/midnight-ui/tests/bulk_selection.rs) and [tree](../crates/midnight-ui/tests/tree_bulk_selection.rs) definitions match all-marks Ctrl+D and default-unassigned Ctrl+Shift+D in `2e0e554`; their recorded binary runs passed 1 and 8 tests. The earlier Linux CI failure remains historical and the combined current diagnostic failed elsewhere. | Build: finish the targeted both-pane/native/archive integrations and record exact source/artifact/commands/results. Preserve explicit files-only action/remap coverage and the wider native/layout/drag qualification requirements; this subset does not establish a full selection or remote CI pass. |
| CMD-02 / P1 / source frozen; runtime qualification pending | The focused-name insertion implementation and focused tests are frozen in source. Captured Custom Commands remain a separate consumer. The current source snapshot has not passed its targeted runtime gate and is not qualified by the older private EXE/startup receipts. | UI + Backend + Build: qualify literal basename insertion from acknowledged native occurrence identity in both panes/remaps, preserving raw names, focus, stale rejection and shell-draft grammar. Do not derive authority from displayed archive paths or execute inserted text. Record exact source/artifact results before closure. |
| STATUS-02, PFS-02 / P1 / terminal styling implemented; targeted UI tests pass | Completed partial totals retain `≈`, yellow warnings and details with normal metadata number styling; only actively unfinished totals are highlighted, even while another root runs. Earlier recorded size/model library tests, `size_errors` 2/0, `panel_warning_details` 4/0 and `tree_folder_sizes` 7/0 passed. | Preserve these guarantees through remaining UI fixes without extending those earlier receipts to changed source, including mixed running/finished, warning-completed and cancelled/incomplete states. Full native/layout/platform qualification remains in the UI acceptance row below; these software UI passes do not close it. |
| SCROLL-02, VIEW-PERF-01/02, MEDIA-03 / P1 / source integration accepted; native qualification open | Corrected Markdown is committed at `255fbdc`, with 72/0 and strict Clippy. Perf accepts source integration from 32 paired debug profiling passes with exact output equivalence: large-fixture request wall time falls about 23 ms → 11–15 ms, while per-request allocator peak rises 383,320/461,739 bytes and small-fixture p95 rises 0.038–0.298 ms. [Retained-seek report](verification/2026-11-12-retained-complete-page-seek.md) retains its dated XML/refill and native boundaries. | Viewer prepares the minimal native-worker handoff. Native scrolling/deployment, release/frame pacing, process memory/RSS, DPI/viewport and scroll-back qualification remain open under coordinated leases. The worker is undeployed; backend wall-time measurements do not close these requirements. |
| LIFE-01 / P1 / source review + qualification | [Lifetime contract](developer/ENGINE_LIFETIME.md) and later service/mailbox/report/configuration reports supersede the first retirement plan's narrower status. Exhaustive closure remains unestablished. | Architect + Backend: reconcile each retained owner/task/channel/last-release path against current source; exercise first-poll/drop, blocked dispatch, shutdown and completed-unconsumed outputs with native-before-provider destruction. A blocked destructor retains real capacity; no weakening or unbounded cleanup fallback. |
| IO-01 / P2 / benchmark prerequisite + implementation | [Transfer contract](developer/TRANSFERS.md) and copy worker remain serial with bounded 256 KiB steps. [Same-SSD probe](verification/2026-11-10-ssd-copy-probe.md) is incomplete and no-go for enabling concurrency from that run. | Backend + Build: complete stable 1/2/4-stream full-verification measurements, then positive local-SSD endpoint classification and shared physical-device budgets. Unknown/HDD/network stay serial. Cover aliases, A↔B fairness, small/mixed/large files, UI p95/p99, failure/cancel and retained in-flight ownership before raising defaults. |
| PLAT-01 / P1 / implementation + native dependency | [Application platform selection](../crates/midnight-platform/src/lib.rs) rejects non-Windows/Linux targets; a separate [read-only Darwin provider](../plugins/midnight-filesystem-native-macos/IMPLEMENTATION_PLAN.md) already exists. | Architect + Backend + UI + Build: reuse that provider and implement separate macOS app services, secure loaders/containment, input/clipboard/windows/terminal, mutations, packaging and CI. Native hardware/runner evidence must establish supported OS/architectures; its absence does not block independent source work. |
| PLAT-03 / P2 / implementation | [Linux external drag](../crates/midnight-platform-linux/src/file_drag_drop.rs) explicitly requires X11/XWayland; native Wayland lacks the same-connection seat/data-device bridge. | Backend + UI: implement bounded raw-identity/admission/cancel/lifetime integration and qualify native compositors. Internal dragging and XWayland results do not establish native Wayland external transport. |

### Provider increments from the Architect review

The accepted Rename API/SDK Task 1 (`f3f8c59`) and public docs (`852ee0c`) are now
source-integrated into D `e5cba5d` with exact 33-file equality.
Their tests, strict Clippy and consumer checks retain their recorded source scopes.
Windows Task 2 and Linux Task 3 remain native WIP in C; no new Rename capability,
runtime deployment or host cutover is claimed.
The public [native protocol](../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md)
explicitly grants no mutation authority to read handles/revisions. Its existing
Refresh invalidates discovery TTL caches; public watch subscriptions/general change
events remain missing, rather than all invalidation being absent. Archive staging
writes host-owned private storage and does not enable archive mutation.

| Gap | Finite scope and required future evidence |
| --- | --- |
| Prepared actions / first VFS-01 Rename slice | Accepted: optional, versioned single-item, same-provider, same-parent prepared Rename. Explicit preparation captures identity before confirmation; it does not guarantee identity from listing time or change the namespace. Opaque source/parent authority retains raw basename, prepared identity/revision, bounded admission/cancel and worker-owned cleanup through refusal, stale delivery and shutdown. Task 1 `f3f8c59` and public docs `852ee0c` are merged into D `e5cba5d` with exact equality of 33 incoming files. Five Architect findings are source-resolved; corrected prepared-Rename tests and strict API/SDK Clippy pass. Direct Windows/network consumer compilation checks pass; Windows Task 2 and Linux Task 3 source work is authorized in C. Both native platform implementations remain WIP/unqualified in C; no new capability/deployment or host cutover is claimed. No-replace is the baseline; replacement must preserve current pinned/versioned/recoverable consent or remain Unsupported. No delete-first/copy/cross-device fallback. Public API/SDK and independent native plugins precede host cutover; no capability is advertised by this in-progress slice. Require compatibility/absent-capability/malformed-handle tests, native prepare/cancel/race/collision/raw-name/case-only/alias tests and installed flat/tree confirmation/queue/cleanup tests. Delete, metadata, transfer and watches stay separate slices. |
| Public watches/general invalidation / VFS-01 | Migrate existing [native tree watch scope](../crates/midnight-engine/src/tree_watch_scope.rs) through explicit provider subscription/revision/event/overflow contracts. Preserve bounded eligible loaded-branch observation, fair polling fallback where supported, dirty epochs through reconciliation and no implicit activation/authentication. Require expanded-empty growth, overflow, >32-branch fairness, late navigation/revocation events, worker cleanup and backing-archive revision invalidation checks. Existing explicit Network Refresh remains supported. |
| Preserved links / COPY-03 | Keep current rejection by default; propose explicit Preserve only for declared endpoint/link kinds. A distinct no-follow link proof captures raw payload/kind/flags, exclusively stages and independently reads back the link representation, revalidates source and publishes safely without opening the referent. Link Move needs separate retained finalization authority; it cannot reuse a regular-file content proof. Require relative/absolute/dangling/cyclic links, junction/kind/privilege limits, Unicode/raw names and untouched external sentinels, plus mismatch/replacement/conflict/cancel/cleanup failures. Hardlink topology and broader metadata remain separate capabilities. |

Proposed ownership: Backend owns public API/SDK/header, separate plugin/platform
implementations, host adapters and engine authority/proof integration; UI owns
capability presentation, drafts/confirmations and flat/tree UI regressions; Build
owns packaging/CI. Shared composition/`lib.rs` edits receive one explicitly named
owner. No source implementation began in the Architect's read-only review.

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
| DIALOG-02, META-02, SELECT-01, PANEL-02, NAV-01/02, NOTICE-02, THEME-02, ICON-02, DRIVE-02, STATUS-02 / UI polish | Embedded modal/focus/input source and corrected selection fixtures are in [current checkpoint scope](#qualified-ui-checkpoint-35a159c); earlier [panel/selection/palette evidence](verification/2026-11-12-panel-interaction-polish.md) retains its dated scope | UI + Build: qualify normal/320×200, desktop/touch, fonts/DPI/themes, input/focus/IME, resizing, X/Escape and captured callbacks. Ordinary modals stay embedded; independent tools retain native hosts. Preserve warning Copy/retry/close and the tested finished-partial-size presentation. Render matrices do not substitute for native/compositor checks. |
| VIEW-01/02, SCROLL-02 / viewers and comparison | [Viewer presentation](developer/VIEWER_PRESENTATION.md), [image limits](developer/IMAGE_VIEWER_API.md), [HTML contract](developer/WEB_VIEWER_API.md), [foreground evidence](verification/2026-11-12-foreground-scroll-resume.md) | Viewer + UI + Build: native text/hex/source/Markdown/image/PDF/HTML opening/navigation/search/copy/zoom/font/persisted settings, exact user fixtures, native/archive content and text/image comparison. Qualify Linux frame pacing, physical IME, native Chromium Wayland and configurable browser key arbitration separately; preserve bounded page selection and inert content. |
| NET-01, PLAT-02 / native environments and permissions | [Network contract](developer/NETWORK_FILESYSTEM.md), [system targets](SYSTEM_REQUIREMENTS.md), [directory authorization](verification/2026-11-11-directory-authorization.md) | Backend + UI + Build: controlled live SMB/GVfs/FUSE, Windows 10, physical Linux desktops/distribution packages, GPU/driver/filesystem and multilingual/RTL/IME evidence. Accepted UAC/polkit prompts remain unqualified by the panel report. Record exact environment, reduced-RAM measurements and artifact notices; WSL/compilation is narrower evidence. |
| CI / complete current validation | Completed remote [run 43260032611](https://github.com/TomasJelinek/fm/actions/runs/43260032611) at `2e0e554` records 18 successful / 2 failed check jobs. Pushed UI checkpoint `35a159c` has passing local regression gates, debug compilation, private engine/native UI startup and shared engine retry | No new completed remote success is claimed. Preserve the initial unattributed hashing failure and later stable checks without claiming a source/validator fix. Local D now includes Task 1/docs (`e5cba5d`), accepted Markdown (`255fbdc`) and diagnostic-only HTML (`e6e086c`). Native WIP and worker deployment remain excluded; broader native/HTML/performance qualification is separate. |
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
