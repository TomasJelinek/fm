# Active work and requirements evidence — 13 November 2026

This is the current implementation/acceptance ledger alongside
[Feature status](FEATURE_STATUS.md) and the preserved
[requirement IDs](IMPLEMENTATION_BACKLOG.md). Historical runs are evidence only
for their named source/environment; they do not turn open acceptance into missing
code or a green subset into complete qualification.

<a id="current-checkpoint"></a>

## Current checkpoint

The separate Linux branch `codex/filter-linux-drive-list` has Debug-built desktop,
permissions, terminal, activation and status-bar fixes based on merge `2b62996`.
Its [scoped Linux verification report](verification/2026-11-13-linux-desktop-fixes.md)
records 106 passing targeted tests and native X11/startup checks. The first
Colors occurrence was restarted before capture. A later live hang revealed a
Slint popup-borrow panic followed by an idle image-owner runtime shutdown wait;
the report distinguishes those captured stacks, the reproduced tooltip panic
and their source fixes from the unavailable original UI callback.
The shortcut Replace/Cancel dialog, unavailable network-button hiding, tooltip
teardown and panic cleanup pass 68 final targeted Debug tests. Refreshing a stale
installed native filesystem plugin adds 8 passing mount tests; the resulting
Debug installation passes native startup/shutdown with the filtered drive bar.
Three separate presenter tests also passed. Exact follow-up receipts are in the
same report.
The later Options sidebar correction measures the full selected topic labels
and grows with the font; 51 targeted UI tests pass, with font-12/font-24 renders
visually reviewed. Its separate receipts and scope are recorded in the report's
[font-scaling follow-up](verification/2026-11-13-linux-desktop-fixes.md#options-navigation-font-scaling).
The subsequent default-on **Allow previews inside archives** setting includes
strict schema 31 persistence, reversible Options edits and archive-only scheduling/
presentation gating. Its [scoped follow-up](verification/2026-11-13-linux-desktop-fixes.md#archive-preview-option)
records 214 passing targeted tests and a visually reviewed default-checked control;
this does not broaden installed archive-format qualification.
The following [row-layout and notification follow-up](verification/2026-11-13-linux-desktop-fixes.md#panel-row-alignment-and-notification-cadence)
limits list labels to one line and reduces notification debounce to 150 ms quiet /
500 ms continuous activity. It records 55 passing targeted tests, three complete
native rendering probes and Debug startup/shutdown. A hidden mandatory line break
reproduced the clipped text; the original screenshot's filename and build age
could not be established, so its exact trigger remains unconfirmed.
These Linux results do not replace the Windows evidence below.
The later [Linux network investigation](verification/2026-11-13-linux-network-discovery.md)
installed the missing system `wsdd` helper and fixed WSDD namespace handling;
84 targeted tests pass, including installed-plugin discovery through GIO. Native
enumeration currently returns zero devices; remote SMB/FUSE remains unverified.

Root committed/pushed **`174c5fd6d426d23f873285eb5c70ffaa6ec28124`**:
38 exact source/documentation files including the COPY core/queue layer and
app-private HTML layout handshake. Root committed/pushed the later three-file
format-only correction as **`cdc9226c0ce04f2ee4b492bdf7763479f5500956`**;
exact-head remote CI is pending. The earlier debug attempt at `174c5fd` exited **101** after
**674.914 s**, zero source drift: cached core binary dep-info watched the older
COPY overlay. The origin-audited `retry-mtime-v1` also failed **E0460** after
**759.096 s** because cached `midnight_native_filesystem_host` referenced a
different core build. The retry had zero source drift and produced no EXE
artifact. These failed attempts retain their historical scope. The later coherent
normal debug build at `cdc9226` passed **254.609 s**, zero source drift, with all
**14** consumer origins rebuilt/coherent. The registered private Windows
startup-qualified EXE is now `f5b29eab`, source `cdc9226`; exact identity and scope
are [below](#current-windows-launch-qualified-runtime-f5b29eab).
Original failed receipt: `_build/verification/integrated-debug-20261113-v2/debug-build-result.json`.

| Area | Current source state | Acceptance still open |
| --- | --- | --- |
| COPY-03 preserved links | Internal core/queue layer integrated; default `Reject`, native hooks `Unsupported`, no UI enablement. | Native Preserve proof/hooks, Move/exact-time Preserve and remaining metadata capabilities; [exact scope/evidence](#provider-increments-from-the-architect-review). |
| HTML | App-private token/generation layout handshake integrated; public plugin API unchanged. | App/native opening/layout/performance/deployment evidence; old failed native measurements remain unqualified. |
| Native Rename | Public/native/core-host source integrated. | Engine/UI routing and finish ownership, installed flat/tree/native-provider workflows and deployment. |
| Tree/DirectoryLink | Captured actions, recursive expansion and loaded-link maintenance implemented; cap128 has a passing synthetic-only gate. | Native I/O/UI paint, physical Linux input, larger-than-model paging and archive/provider gaps. |
| Current runtime/CI | Coherent normal debug build and registered private startup pass at `cdc9226`, runtime `f5b29eab`; previous `516fc158` is retained. Exact-head `9176272` remote CI is **20/20 GREEN**. Historical `174c5fd` failed both format checks, 13 hunks/3 files; Root's format correction is committed/pushed as `cdc9226`. | Exact-head remote CI at `cdc9226`, complete tree/HTML/native performance and workflow qualification remain pending. |

[Archived run detail](verification/2026-11-13-checkpoint-evidence-history.md) retains unique failures, full hashes, deleted-QA
provenance and overlapping-count arithmetic. Existing
[recursive/tree](verification/2026-11-12-recursive-tree.md),
[panel](verification/2026-11-12-panel-interaction-polish.md) and
[CSC](verification/2026-11-11-directory-authorization.md) reports retain their
original qualification; later WSL/software fixtures do not relabel those runs.

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

| Role | Current focus |
| --- | --- |
| Orchestrator | Scoped task queue, acceptance criteria, exclusive files, independent review, source/build identity and source/Git integration. |
| Architect | Remaining provider/engine/UI contracts, Rename finish ownership, native Preserve authority and archive/nesting/capacity decisions. |
| Backend agent | Native/provider cutover, COPY native hooks, source/identity/consent guards, Find-report reproduction and remaining service lifetimes. |
| Viewer agent | Source/Markdown/image/PDF/HTML workers, matching packaged-runtime qualification and unresolved viewer opening/scrolling paths. |
| UI agent | Captured actions, modal/focus/input/layout integration and native/platform acceptance; no capability enablement from source-only gates. |
| GitHub build system and build tools developer | Retain coherent build/consumer-origin and registered startup receipts; coordinate pending remote CI, source/target leases, packaging and authorized publishing. |
| Perf agent | Source drag, HTML queue, native tree/scrolling and SSD copy measurements; distinguish protocol/software observations from input-to-paint. |
| Documentation guru | Maintain current contracts/status, requirement boundaries and verified source/environment evidence. |

Assignments require acceptance criteria and explicit file ownership; role chats do
not authorize unrelated backlog work. Agree shared API/UI edits with the Architect
and Orchestrator. One owner holds each Cargo target lease; compile compatible
groups once, bound independent headless runs and serialize desktop/input tests.
Root integrates source and owns Git; preserve exact source/environment, failures/ignores and
unresolved work in handoffs. Follow [Agent workflow](developer/AGENT_WORKFLOW.md),
[code style](../CODE_STYLE.md), [security](../SECURITY.md) and
[dependency policy](DEPENDENCY_POLICY.md).

## Integrated baseline and recorded evidence

### Current Windows launch-qualified runtime `f5b29eab`

The coherent normal debug receipt
`_build/verification/integrated-debug-20261113-v2/coherent-final/debug-build-result.json`
records **exit 0 / 254.609 s**, source
`cdc9226c0ce04f2ee4b492bdf7763479f5500956`, zero drift and all **14** consumer
origins rebuilt/coherent. The registered startup receipt is
`_build/verification/integrated-debug-20261113-v2/final-qualified-debug-result.json`.
The private EXE is
`C:/Users/tjelinek/MidnightExplorer-20261113-083454-f5b29eab/me.exe`,
**102,148,096 bytes**, SHA256
`f5b29eab9776990bb15c82b60b0fe5fa4b19421c05d089a5313a4478f9dc8a88`.

Engine **12.101 s** and native UI **6.386 s** startup smokes both exited **0**,
without timeout; registration followed the passing smokes. Fixtures/preferences
and the desktop shortcut are unchanged. All **1,382** new and **1,382** previous
runtime files were hash-verified. The build snapshot matches **3,636 app inputs**;
four dirty diagnostic examples are excluded from the 3,640 observed inputs.
Root independently checked all 14 consumer origins and the EXE hash, and visually
reviewed both panels, Unicode and footer. The previous qualified
[`516fc158` runtime](#current-windows-launch-qualified-runtime-516fc158) is retained.

This qualifies private Windows startup/snapshot identity only. Complete tree,
HTML, native performance and workflow qualification remain open, along with
physical Linux UI and installed native Rename deployment. Candidate native
plugins were not deployed. Exact-head remote CI at `cdc9226` remains pending.

<a id="current-windows-launch-qualified-runtime-516fc158"></a>

### Previous Windows launch-qualified runtime `516fc158`

The receipts under `_build/verification/integrated-debug-20261113/` record a
fresh debug build, **102,041,600 bytes**, source
`d6455a0133df5e279ffbdab62b86691007c2c5be` and zero drift. The private binary is
`C:/Users/tjelinek/MidnightExplorer-20261113-051356-516fc158/me.exe`,
SHA256 `516fc158567c2179087f4c166d23c8b51387c8c077236fc87bcabfa684226018`.

`qualified-debug-result.json` records engine **17.882 s** and native UI **7.901 s**,
both exit **0**, without timeout/fallback. Root/Build visually reviewed both
panels, Unicode text, toolbars, command bar and footer. The recorded build snapshot
matches **2,327 application inputs**; three unqualified diagnostic examples are
excluded from the 2,330 observed inputs. All **1,382** new and **1,382** retained
previous runtime hashes were verified. Fixtures/preferences/registrations and the
desktop shortcut remain unchanged; the previous `c6c371bc` runtime is preserved.
Owned jobs and source/target/native-input/performance leases were released.

This is private Windows startup/snapshot qualification. Native Rename candidates
were not deployed. It does not establish full workflows, native tree/viewer
performance, physical Linux UI or remote CI GREEN. PID 29940 remains unclassified;
these startup runs do not prove generic natural auxiliary-process lifetime.

Source/HTML performance qualification remains **failed/unqualified**. Source v4
(`viewer-20261113/inactive-source-gate-v4/result.json` under `_build/verification/`)
passes compile/scoped Clippy with zero drift, but native gesture submission fails
on hidden/minimized/invalid geometry; no completion report or qualified first
publication is available. HTML's `html-queue-build-owned/final-manifest.json` under
the same viewer directory retains diagnostic exit **1**, default skipped, zero
accepted first publications/draws and zero completed wheel rounds. No queue
attribution or tracing-overhead comparison is established.

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
auxiliary-process lifetime remains unqualified. At that source gate the private
EXE was `c6c371bc`; the later `516fc158` startup includes this correction without
broadening the shutdown gate's scope.

### Native link presence and Find gate

The native captured-root DirectoryLink admission correction passes
**18/0/0 = 12 presence + 6 durable Find regressions**, compile 0/zero drift.
Its prior presence RED is **6/2/0**. The earlier exact-user-fixture diagnostic is
**13/0/0**, with unchanged raw path/kind/identity/revision; the six final Find
regressions overlap it and did not rerun that user fixture. The original Open
report's build/capture is unknown: no production Find fix or false-stale cause is
proven. [Exact fixture, source pin and receipts](verification/2026-11-13-checkpoint-evidence-history.md#native-link-presence-and-find-gate)
remain retained.

<a id="tree-scaling-candidate-gate"></a>

### Tree scaling candidate gate — aggregate failed

Historical cap16 passes **95 unique functional cases/Clippy 0**, but its opt-level 0
synthetic scaling aggregate **failed**. The stale initial artifact was rejected;
unrepeated/censored baselines, exact times and callback/publication counts remain
in the [historical scaling record](verification/2026-11-13-checkpoint-evidence-history.md#tree-scaling-candidate-gate).
The separate cap128 result below does not rewrite that failure.

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
The retained `c6c371bc` runtime excludes this source. The later `516fc158` runtime
contains it, with startup-only qualification rather than native tree timing.

### Integrated source and targeted gates

| Gate | Exact retained scope and limit |
| --- | --- |
| HTML layout handshake | `174c5fd` source; compile/strict Clippy 0, **135 selected/executed/passed, 0 failed/ignored**, **35.656 s**, artifact/source drift 0, owned jobs released. Core tests opt-level 0; engine/UI opt-level 1 with debug assertions. Headless evidence only: no app/native measurement or deployment. [Contract](developer/WEB_VIEWER_API.md). |
| COPY core/queue | Integrated internal layer; [80-unique accounting, prior77/3 and queue17 retry](#provider-increments-from-the-architect-review) are retained in COPY-03 below. Native hooks/UI/Move/exact-time Preserve remain unavailable. |
| Original recursive/CSC/thumbnail | [Tracked original report](verification/2026-11-12-recursive-tree.md): Windows/software UI 69; broad Backend 158/1 and overlapping refined 11/0; thumbnail 12/2 → 14/0. CSC mocked/denied/cancelled helper paths do not prove privileged access. |
| Later Linux fixtures | Exact `1714d99` real symlink **1/0/0** and updated frozen DirectoryLink fixture **1/0/0** are separate WSL/tmpfs receipts. Helper UI **13/0/0 = clipboard 5 + copy 2 + move 6**, after an initial missing-`pkg-config` compile with no tests. They are not a 69-case native Linux rerun or physical desktop qualification. [Source/CRLF-wrapper provenance](verification/2026-11-13-checkpoint-evidence-history.md#selected-local-gates-and-linuxprobe-provenance). |
| Worker test-only CI correction | `38800aa`: strict `--lib --tests` default **18.3488 s**, diagnostics **1.6885 s**, both exit 0/libtest artifacts; **9 preserved inputs/0 drift**, rustfmt0 reported. No runtime tests/app rebuild; not remote GREEN. |
| Linux alias/octal compatibility | `9176272`: strict all-targets Clippy exits0, including library `cfg(test)`, **13.8297 s**, zero drift. Runtime tests were not rerun; no capability semantics change. [Receipt/source context](verification/2026-11-13-checkpoint-evidence-history.md#native-rename-gate-history). |

<a id="qualified-ui-checkpoint-35a159c"></a>
<a id="image-prewarming-source-and-evidence"></a>

### Earlier bounded evidence

The [recorded evidence note](verification/2026-11-13-checkpoint-evidence-history.md#earlier-bounded-evidence) retains the original
hashing anomaly, Markdown 69/2 → 72 and debug A/B costs, failed Source/PNG admission,
7.192-second control, prewarm compile/Clippy101, R2 34/9 → 43 unique and R3
44 unique scopes. The five native controls/BMP pair are R2-only. Source
integration/one same-candidate host observation does not qualify application F3,
all images, steady idle resources, native scrolling, Source gesture or RSS.

### Retained remote CI boundary

Build's exact-head watch records **`9176272`: 20/20 GREEN**. The historical
`174c5fd` result is **not green**: both format checks failed **13 hunks across
3 files**. After Build released the failed-build source lease, Root applied the
three-file format-only correction, committed/pushed as `cdc9226`; its exact-head
remote CI is pending. The separate registered `f5b29eab` startup receipt is
[above](#current-windows-launch-qualified-runtime-f5b29eab); it does not establish
remote CI or complete workflow/performance qualification. The
[older remote/local failure chain](verification/2026-11-13-checkpoint-evidence-history.md#retained-remote-ci-boundary)
retains `02b2f2f` run 43367992268/job 131699200987, 43345016027/43312251721,
exact `1714d99`, failed-unreached targets and fixture/lint correction provenance.

### Completed build-script integration

The [recorded build-port/merge evidence](verification/2026-11-13-checkpoint-evidence-history.md#completed-build-script-integration)
retains exact branch/input identities and non-additive test scopes. Historical
source integration is not a current workspace/remote test pass.

### QA artifact cleanup

The [completed QA cleanup record](verification/2026-11-13-checkpoint-evidence-history.md#qa-artifact-cleanup) retains authorization,
681 roots, non-additive free-space observations, protected executable hash and the
original Markdown62/allocation handoff. `_build/qa/` was removed; historical paths
are provenance, not required extant files or new command recommendations.

### Retained private runtime history

<a id="previous-windows-launch-qualified-runtime-c6c371bc"></a>
<a id="previous-windows-launch-qualified-runtime-26ff61f2"></a>
<a id="previous-windows-launch-qualified-runtime-269f3419"></a>
<a id="previous-launch-qualified-runtime-d2643b49"></a>

Exact earlier runtime identities/startup scopes are preserved in the
[private-runtime history](verification/2026-11-13-checkpoint-evidence-history.md#retained-private-runtimes) and original linked
verification notes. The unclassified PID 29940 and old hashing anomaly remain
unattributed; later clean owned-job samples do not identify them.

## Current source/contract boundaries

| Area | Current source / contract | Boundary to preserve |
| --- | --- | --- |
| Filename palette | [Defaults](../crates/midnight-core/src/filename_colors.rs), [palette contract](developer/FILENAME_COLOR_OPTIONS.md) | Twelve ordered categories; PDF base `#EF5A5A`, DOCX/Markdown base `#C4CFE0`. Rendered contrast may adjust bases. Filename coloring does not imply decoder support. Exact historical defaults migrate; custom rules survive. |
| Selection defaults | [Shortcut catalog](../crates/midnight-core/src/shortcuts/catalog.rs), [tree row policy](../crates/midnight-core/src/tree_view.rs) | Ctrl+D clears every mark, including folders/expanded descendants; Ctrl+Shift+D has no default. Ctrl+A/Ctrl+Shift+A replace top-level selection and clear nested marks; Ctrl+Shift+I inverts top-level entries while retaining descendant marks. Saved deliberate overrides remain authoritative. |
| Captured inline actions | [Tree panel contract](developer/TREE_PANEL.md), [current status](FEATURE_STATUS.md) | Typed native consumers exist for transfers, clipboard/drag, focused editor, mutations/metadata, Properties/context menus, checksums, file lists and comparisons. F3/file comparison use visible root files; inserted branches still contain folders. Archive capability limits remain explicit. |
| Transfer policy and order | [Verification enum](../crates/midnight-core/src/transfer_verification.rs), [copy worker](../crates/midnight-engine/src/copy_worker.rs), [native Move](../crates/midnight-engine/src/move_native.rs), [transfer contract](developer/TRANSFERS.md) | Serial bounded copy, immutable full/quick policy, same-volume rename and per-file source removal. Neither quick mode nor native rename weakens identity/consent/publication checks. |
| Filter/drive/status | [Panel controls](../crates/midnight-ui/ui/panel-view-controls.slint), [filter presenter](../crates/midnight-ui/src/presenter/filter.rs), [drive information](../crates/midnight-ui/src/presenter/drive_information.rs), [folder totals](../crates/midnight-ui/src/presenter/panel_details.rs) | Filter icon/applied state, backend volume information and omission of unmeasured folder totals already exist. Their current native usability and qualification are separate from implementation presence. |


## Requirements and evidence backlog

Priorities retain the existing requirement IDs. P0 means an integrity prerequisite
before enabling a dependent action; P1 means functionality/reliability; P2 means
qualification or measured optimization. Owners below are the appropriate roles
for the next assignment, not permission to start unreserved edits or builds.

### Implementation, contract and performance work

| IDs / priority / kind | Verified basis and remaining work | Next acceptance / role |
| --- | --- | --- |
| VFS-01, PFS-01 / P1 / public API/SDK source integrated; native/host cutover open | Public [ABI 1.3](../filesystem-api/midnight-filesystem-plugin-api/src/abi.rs) and [operation family](../filesystem-api/midnight-filesystem-plugin-api/src/model.rs) now include optional prepared Rename alongside browsing/reads/storage and Activate/Refresh/ResolveText. Task 1 plus public docs are merged into D `e5cba5d`, with exact 33-file equality. [App composition](../crates/midnight-explorer/src/main.rs) still injects native MutationService/WatchService separately; Windows/Linux native 0.2 source is integrated in D `5baee471`. | Architect + Backend: retain the source-integration boundary while qualifying native Task 2/3 and later host routing. No new provider Rename capability/deployment or host cutover is claimed. Reject stale/foreign/unsupported targets without native/TEMP substitution; plugins share only public APIs and own tests. |
| TREE-01, ARCH-02 / P1 / implementation | [Tree contract](developer/TREE_PANEL.md) includes implemented bounded recursive expansion (`ce9eff3`, Windows UI 69/0 and overlapping Backend refined 11/0) and captured consumers; physical Linux desktop qualification/larger-than-model paging remain open. Production container discovery/attachment, archive watches and nested mounts remain separate gaps: [attach API](../crates/midnight-engine/src/providers/tree.rs) has test callers, while [archive mount input](../crates/midnight-filesystem-host/src/session.rs) still takes a native captured file. | Backend + Architect: add provider-content mount leases, exact parent anchors, bounded aggregate nesting/decompression/cache, invalidation and cancel/retry. Prove native → archive → archive → viewer, outer-source changes and concurrent solid readers; qualify supported tree actions without removing unsupported-action guards. |
| TREE-01 / P1 / DirectoryLink maintenance implemented; Windows qualified | Parent-first loaded native-link refresh, exact acknowledged watch scope, physical-identity notification deduplication with separate logical aliases, and retarget-aware subscription/poll validation pass the Windows gate: 51 selected/passed, 0 failed/ignored, compile 0, zero drift. Disjoint groups: provider 10 + refresh 25 + watch 15 + real Windows junction 1. `build-green-result.json` and `build-green-runtime-result.json` under `_build/verification/backend-tree-link-refresh-20261113/` establish this scope. Prior RED 2/3/0 is historical. | Retain generation/exact Commit, failed-parent descendant suppression, cycle/retarget and logical-action safeguards; no write/ACL/elevation changes. Updated Linux-native fixture passes 1/0/0 on WSL/tmpfs with frozen overlays and zero drift. The separate local Linux UI gate passes 13/0/0; historical `269f3419` debug compilation and registered private Windows startup pass. The earlier exact-`1714d99` Linux 1/0/0 fixture does not qualify this updated source. |
| TREE-01 / P1 / cap128 source integrated; synthetic gate passed | [Cap128](#recursive-tree-cap128-gate) at `2b28cc7`, integrated by `02b2f2f`, passes 25 targeted functional cases, strict Clippy 0 and one opt-level 0 synthetic aggregate. Four timed fixtures complete with 8/80/80/79 publications; the preceding cap16 95/0/0 and failed aggregate remain historical. | Perf + Backend: retain cycle/retarget, exact Commit, cancellation and captured-action guards. No new baseline, release/native filesystem, UI paint or visible-smoothness qualification is established; the `c6c371bc` runtime predates both increments. |
| COPY-03 / P1 / implementation | [Metadata policy](../crates/midnight-core/src/transfer_metadata.rs) exposes ContentOnly and required exact modified time for up to 64 native regular-file roots. Full metadata/directory metadata and explicit link-object copying remain unavailable. | Backend: define capabilities for remaining times/attributes/ACLs/xattrs/streams and postorder directory metadata; unsupported kinds fail before writes where knowable. Verify required values before publication and retained sources on failure; link copying must preserve links without following referents. |
| TERM-01 / P1 / implementation | [Terminal service](../crates/midnight-core/src/desktop.rs) is launch-only and [Linux launch](../crates/midnight-platform-linux/src/desktop.rs) spawns a new process/TTY. The [reusable-session requirement](superpowers/specs/2026-11-04-terminal-sessions-design.md) remains open. | Backend: one unprivileged bounded broker with kernel-verified process/UID/lifetime identity; distinguish Idle/Busy/disconnected/unknown outcome and never replay uncertain commands. Qualify a concrete terminal; sudo owns authentication/expiry, with no stored password, policy change or keepalive. |
| FIND-01/02 / P1 / Open report unreproduced; durable regressions pass | The [validator](../crates/midnight-engine/src/find_worker.rs) compares raw path/kind/native identity and six revision words. The [current diagnostic and durable gate](#native-link-presence-and-find-gate) records 13/0/0 with the exact user fixture unchanged and no differing identity/revision, followed by 6/0/0 durable regressions with production Find unchanged. The original report's build and captured metadata are unknown; no false-stale cause or production fix is proven. | Backend + Build: if the report recurs, retain exact runtime/session identity and captured/current raw path, kind, identity and revision words before selecting a correction. Preserve replacement/stale-session rejection. UI Details is the query report, not a second file-Properties action. |
| NET-01 / P1 / implementation | Discovery and typed discovery Hot Paths exist; [Linux GIO](../plugins/midnight-filesystem-network/src/linux/gio.rs) explicitly aborts interactive password challenges. Live SMB/GVfs interoperability is a separate qualification gap. | Backend + Architect: host-owned ephemeral authentication/cancel flow with no credential persistence/logging. Cover stale/canceled auth, reconnect and provider removal; then qualify an owned live endpoint. |
| FILTER-01/02, UI-01 / P1 / targeted positive-input gate passed | The retained replacement-popup focus assertion passes. Native Filter 17/0, Filter library 2/0 and exact operation-lifetime 1/0 cover 20 distinct tests; exact-first replay is not added twice. Find geometry passed with unchanged source. Private engine/native UI startup subsequently passes. | Preserve verified input/focus/lifetime and source identities. The original shared hashing anomaly remains preserved/unattributed after a passing engine retry; full native/layout/platform acceptance is not closed by these focused tests or startup smokes. Retain real pointer/key, occurrence identity, history/clear and stale-token coverage. |
| SELECT-01 / P1 / fixture correction implemented; subset passed | [Flat](../crates/midnight-ui/tests/bulk_selection.rs) and [tree](../crates/midnight-ui/tests/tree_bulk_selection.rs) definitions match all-marks Ctrl+D and default-unassigned Ctrl+Shift+D in `2e0e554`; their recorded binary runs passed 1 and 8 tests. The earlier Linux CI failure remains historical and the combined current diagnostic failed elsewhere. | Build: finish the targeted both-pane/native/archive integrations and record exact source/artifact/commands/results. Preserve explicit files-only action/remap coverage and the wider native/layout/drag qualification requirements; this subset does not establish a full selection or remote CI pass. |
| CMD-02 / P1 / corrected Windows gate passed | Checkpoint `21c9c7a` records the six accepted UI files. Corrected gate: 37 passed / 0 failed / 0 ignored across five artifacts, zero source drift, completed 21:16:16 CET. Natural FocusOut/caret and explicit active-size-bit fixtures pass locally; initial 31/5 setup failures remain historical. Captured Custom Commands remain separate; the current private `f5b29eab` EXE has startup-only qualification, with command workflows separate. | Preserve literal insertion, acknowledged occurrence identity, raw names, focus, stale rejection and shell-draft grammar. Build: retain the exact receipts and qualify remaining Linux/native command-workflow boundaries separately. Do not derive authority from displayed archive paths or execute inserted text; no remote CI fix is inferred. |
| STATUS-02, PFS-02 / P1 / terminal styling implemented; targeted UI tests pass | Completed partial totals retain `≈`, yellow warnings and details with normal metadata number styling; only actively unfinished totals are highlighted, even while another root runs. Earlier recorded size/model library tests, `size_errors` 2/0, `panel_warning_details` 4/0 and `tree_folder_sizes` 7/0 passed. | Preserve these guarantees through remaining UI fixes without extending those earlier receipts to changed source, including mixed running/finished, warning-completed and cancelled/incomplete states. Full native/layout/platform qualification remains in the UI acceptance row below; these software UI passes do not close it. |
| SCROLL-02, VIEW-PERF-01/02, MEDIA-03 / P1 / source integration accepted; native qualification open | Corrected Markdown is committed at `255fbdc`, with 72/0 and strict Clippy. Perf accepts source integration from 32 paired debug profiling passes with exact output equivalence: large-fixture request wall time falls about 23 ms → 11–15 ms, while per-request allocator peak rises 383,320/461,739 bytes and small-fixture p95 rises 0.038–0.298 ms. [Retained-seek report](verification/2026-11-12-retained-complete-page-seek.md) retains its dated XML/refill and native boundaries. | Viewer + Build: establish matching packaged-worker/native evidence. Native scrolling/deployment, release/frame pacing, process memory/RSS, DPI/viewport and scroll-back qualification remain open under coordinated leases. Matching deployment/interactive evidence is still required; backend wall-time measurements do not close these requirements. |
| SCROLL-02, VIEW-PERF-01/02 / P1 / native Source drag unresolved | The archived one-off Source probe completed 1,098 operations/732 exact replays without mismatches; it measures broker/codec work, not the user's 134-line C++ scrollbar input-to-paint path. Portable panel regression tests `3bee1dc` 4/0 and thumbnail correction `8e205c0` 14/0 do not qualify that separate viewer path. | Viewer + UI + Build/Perf: capture the actual native scrollbar gesture, dispatch, request, layout/render and presentation phases on an owned unchanged fixture; attribute the observed delay before choosing a fix. Preserve source identity, cancellation and bounded provider I/O; do not infer a rendering improvement from protocol replay. |
| LIFE-01 / P1 / source review + qualification | [Lifetime contract](developer/ENGINE_LIFETIME.md) and later service/mailbox/report/configuration reports supersede the first retirement plan's narrower status. Exhaustive closure remains unestablished. | Architect + Backend: reconcile each retained owner/task/channel/last-release path against current source; exercise first-poll/drop, blocked dispatch, shutdown and completed-unconsumed outputs with native-before-provider destruction. A blocked destructor retains real capacity; no weakening or unbounded cleanup fallback. |
| IO-01 / P2 / benchmark prerequisite + implementation | [Transfer contract](developer/TRANSFERS.md) and copy worker remain serial with bounded 256 KiB steps. [Same-SSD probe](verification/2026-11-10-ssd-copy-probe.md) is incomplete and no-go for enabling concurrency from that run. | Backend + Build: complete stable 1/2/4-stream full-verification measurements, then positive local-SSD endpoint classification and shared physical-device budgets. Unknown/HDD/network stay serial. Cover aliases, A↔B fairness, small/mixed/large files, UI p95/p99, failure/cancel and retained in-flight ownership before raising defaults. |
| PLAT-01 / P1 / implementation + native dependency | [Application platform selection](../crates/midnight-platform/src/lib.rs) rejects non-Windows/Linux targets; a separate [read-only Darwin provider](../plugins/midnight-filesystem-native-macos/IMPLEMENTATION_PLAN.md) already exists. | Architect + Backend + UI + Build: reuse that provider and implement separate macOS app services, secure loaders/containment, input/clipboard/windows/terminal, mutations, packaging and CI. Native hardware/runner evidence must establish supported OS/architectures; its absence does not block independent source work. |
| PLAT-03 / P2 / implementation | [Linux external drag](../crates/midnight-platform-linux/src/file_drag_drop.rs) explicitly requires X11/XWayland; native Wayland lacks the same-connection seat/data-device bridge. | Backend + UI: implement bounded raw-identity/admission/cancel/lifetime integration and qualify native compositors. Internal dragging and XWayland results do not establish native Wayland external transport. |

### Provider increments from the Architect review

<a id="native-rename-audit-gates"></a>

Public prepared-Rename API/SDK source is integrated at `e5cba5d`; native
Windows/Linux 0.2 source at `5baee471` from C `42d45def` (25 paths matching frozen `4eea72bc`);
Task 4A/4B core-host source at `02b2f2f`. These are included in `174c5fd`.
Native source integration leaves engine/UI cutover, installed-provider/native
runtime qualification and deployment open. The retained `516fc158` runtime
predates the new native plugins. The earlier Linux 38-case, Windows 21/1 → 22/0,
Task 4A 44-case and Task 4B preceding 79 per platform → focused two per platform gates keep their original scopes;
[exact freezes, failures and overlap accounting](verification/2026-11-13-checkpoint-evidence-history.md#native-rename-gate-history)
are retained separately.

The public [native protocol](../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md)
explicitly grants no mutation authority to read handles/revisions. Its existing
Refresh invalidates discovery TTL caches; public watch subscriptions/general change
events remain missing, rather than all invalidation being absent. Archive staging
writes host-owned private storage and does not enable archive mutation.

| Gap | Finite scope and required future evidence |
| --- | --- |
| Prepared actions / first VFS-01 Rename slice | Accepted: optional, versioned single-item, same-provider, same-parent prepared Rename. Explicit preparation captures identity before confirmation; it does not guarantee identity from listing time or change the namespace. Opaque source/parent authority retains raw basename, prepared identity/revision, bounded admission/cancel and worker-owned cleanup through refusal, stale delivery and shutdown. Public/native/core-host source is integrated as summarized above; [dated native gates](#native-rename-audit-gates) do not establish installed-provider qualification, deployment or the unimplemented engine/UI cutover. No-replace is the baseline; replacement must preserve current pinned/versioned/recoverable consent or remain Unsupported. No delete-first/copy/cross-device fallback. Public API/SDK and independent native plugins precede host cutover; no deployed native capability is established by these selected gates. Require compatibility/absent-capability/malformed-handle tests, native prepare/cancel/race/collision/raw-name/case-only/alias tests and installed flat/tree confirmation/queue/cleanup tests. Delete, metadata, transfer and watches stay separate slices. |
| Public watches/general invalidation / VFS-01 | Migrate existing [native tree watch scope](../crates/midnight-engine/src/tree_watch_scope.rs) through explicit provider subscription/revision/event/overflow contracts. Preserve bounded eligible loaded-branch observation, fair polling fallback where supported, dirty epochs through reconciliation and no implicit activation/authentication. Require expanded-empty growth, overflow, >32-branch fairness, late navigation/revocation events, worker cleanup and backing-archive revision invalidation checks. Existing explicit Network Refresh remains supported. |
| Preserved links / COPY-03 | Task 1's internal core/queue source is integrated in `174c5fd`. Queue retry **17/0/0**, **11.562 s**; strict core+engine `--lib --tests` Clippy exits **0**, **44.6691 s**, with zero source-freeze drift. Current unique functional evidence is **80 = 63 retained nonqueue passes + 17 queue rerun**; the earlier 80-case **77/3** remains historical, not 97 unique cases. Default stays `Reject`; native Preserve hooks remain `Unsupported`, with no UI enablement. Move/exact-time Preserve remains unavailable. No dependency or public API/SDK change. Remaining native requirements: A distinct no-follow link proof captures raw payload/kind/flags, exclusively stages and independently reads back the link representation, revalidates source and publishes safely without opening the referent. Link Move needs separate retained finalization authority; it cannot reuse a regular-file content proof. Require relative/absolute/dangling/cyclic links, junction/kind/privilege limits, Unicode/raw names and untouched external sentinels, plus mismatch/replacement/conflict/cancel/cleanup failures. Hardlink topology and broader metadata remain separate capabilities. |

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
| CI / complete current validation | [Retained remote boundary](#retained-remote-ci-boundary): exact-head `9176272` is **20/20 GREEN**. The historical `174c5fd` result is **not green**: both format checks failed 13 hunks/3 files. Root's three-file format-only correction is committed/pushed as `cdc9226`; its exact-head remote CI is pending. [Earlier failures and correction provenance](verification/2026-11-13-checkpoint-evidence-history.md#retained-remote-ci-boundary) retain their dated scopes. HTML 135, COPY 80, C native-plugin and WSL/software gates remain selected local evidence. | Build + Root: obtain fresh frozen-source targeted/workspace and remote receipts. Structural workflow validation, selected local gates, C native-plugin gates, WSL fixtures and qualified `f5b29eab` startup do not establish full-suite/desktop success. The earlier cap16 functional95/Clippy 0 and failed aggregate remain historical; the later integrated cap128 passes only its recorded functional25 and single synthetic aggregate. Native provider/host Task 4, viewer performance and manual permissions remain separate. |
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
