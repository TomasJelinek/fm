# Implementation backlog and design audit

Audit date: 2026-11-07. Requirement/evidence matrix for accepted design and
follow-up work, compared with current source and verification records. Historical
checkboxes are not implementation evidence. Close stable IDs with code and native
validation evidence. This is maintained as implementation proceeds; cited test
results belong to their dated verification documents, not to the documentation
audit itself. The latest accepted follow-ups below supersede conflicting earlier
requirements or historical evidence descriptions. The 10 November documentation
reconciliation updates HOTPATH-02, FIND-02, ARCH-01 and TREE-01 from their dated
source/debug/worker evidence, including loaded-row tree prefix integration and
raw-name archive extension sorting; it does not qualify their remaining native
workflows. Root-file F3/Alt+F3 from adopted trees is now connected through immutable
viewer snapshots and backend normalization. F4 now captures the focused native
regular file through acknowledged tree identity, independently of selection;
[editor evidence](verification/2026-11-11-tree-external-editor.md) covers stale
receipts, cancellation, worker lifetime and both panel inputs. Expanded branches
still contain folders only. Native Delete now uses typed capture and the common
confirmation/progress path; remaining typed actions and provider writes are pending.
See the [tree viewer contract](developer/TREE_PANEL.md#viewers-from-expanded-panels).
F5 now captures selected native/archive tree occurrences for the normal verified
copy queue; Windows/Linux tests and Windows debug deployment are recorded in
[tree Copy evidence](verification/2026-11-10-tree-copy.md). F6 now shares the typed
capture with immutable Move intent, and terminal Copy/Move reconciles loaded
native branches in-session. Windows/Linux bridge tests, fixed review findings
and the registered Windows debug build are recorded in the
[tree Move evidence](verification/2026-11-10-tree-move.md). Native clipboard is also implemented;
remaining operation consumers are pending. Native manual/watch refresh now uses the
same in-session maintenance, retaining expanded branches and surviving selection;
the [watch contract](developer/TREE_PANEL.md#native-watches-and-reconciliation)
describes bounded native subscriptions, fair polling and generation fences.

The [9 November provider/performance/usability follow-ups](superpowers/plans/2026-11-09-provider-performance-usability.md)
add concrete reproductions and acceptance checks. They explicitly resume generic
archive streaming/disk spooling with no artificial file-size ceiling; the
JPEG-specific decoder experiment remains reverted. Their newer requirements
supersede the older archive-size and deferred-status notes below.

States: **missing** workflow, **partial** implementation, **investigate** unresolved
defect, **validation** outstanding environment/deployment check, **later** separate
extension. P0 is an integrity prerequisite before enabling its dependent action;
P1 functional/reliability work; P2 qualification/optimization. Priority determines
sequence, not whether accepted work can be omitted.

## Lifecycle follow-up found on 12 November

**LIFE-01 — P1, partial:** the original unpolled-engine and actor-abort destruction
bugs were reproduced. Startup now reserves a plain cleanup worker and bounded slots
before taking provider ownership. The synchronous Engine run wrapper owns suspended
execution and the complete command inbox; Copy/Move/Delete handoffs, shared snapshots
and drag state retain their cleanup reservations through actual destruction.
The [engine contract](developer/ENGINE_LIFETIME.md) now describes 308 payload slots,
seven bounded intake waiters, eight shared terminal/editor/Custom Command process
slots and sixteen independent opener slots. The
[initial integrated evidence](verification/2026-11-12-engine-lifetime-integration.md)
remains historical; the [child-task verification](verification/2026-11-12-task-services.md)
records the additional shared service guard for Find, preferences and fonts,
[checksum capture retirement](verification/2026-11-12-checksum-retirement.md), and
[Custom Command ownership](verification/2026-11-12-user-command-lifetime.md).
Targeted tests cover first-poll/drop, blocked dispatch, runtime shutdown,
native-before-provider order and completed/unconsumed preparation. No data-integrity
failure was reproduced. A single blocked native destructor or held send permit can
stall its bounded cleanup domain; UI/executor Drop does not wait. Custom Command
process quota remains occupied through observed exit and actual native destruction.

The [native-service follow-up](verification/2026-11-12-native-service-retirement.md)
adds guarded desktop/editor process outputs, both watch pane controls and clipboard
inbox/rejection cleanup. Controller Drop now stops a clipboard pump even with a full
result queue; native sessions/subscriptions remain on their original threads.

This is not full LIFE-01 closure. The
[remaining actor inventory](verification/2026-11-12-remaining-actor-ownership-inventory.md)
records the original File List and dynamic network findings now implemented by the
[remaining native-results batch](superpowers/plans/2026-11-12-remaining-native-results.md).
That batch also covers the remaining original service task pairs and PathPicker Drop.
[Qualification](verification/2026-11-12-remaining-native-results.md) records the final
Windows/Linux affected gates, debug deployment and native/software startup smokes.
Archive navigation and pending size roots now have guarded ownership; see the
[mailbox qualification](verification/2026-11-12-native-mailboxes.md). Completed tree
branches proved metadata-only; no redundant native guard was added. Newly generated
warning reports now retain a shared engine-owned arena: actual storage and retained text
retire on a plain worker, including runtime shutdown and last-owner release. Both original
native-file regressions and producer tests pass; see [arena qualification](verification/2026-11-12-report-arena-retirement.md)
and the [producer contract](verification/2026-11-12-report-arena-plumbing.md).
Repeated Engine builder replacement is prevented by one-time admission: all 17 optional
services obtain a vacant slot before transferring ownership, so duplicate configuration
accepts no replacement owner. Existing composition and test fixtures use the new API.
[Configuration evidence](verification/2026-11-12-engine-service-configuration.md) records
the reproduced caller-side cleanup and the focused Linux correction; combined Windows
qualification passed and is recorded in the
[combined checkpoint](verification/2026-11-12-config-rename-cmd-checkpoint.md).
No service-configuration retirement capacity was added.
Native tree Rename adds four reserved capture slots to the domain, bringing the combined
ledger to 308; accepted captures retain admission until actual worker-side destruction.
This is not an exhaustive completion audit. The
[runtime-independent retirement plan](superpowers/plans/2026-11-12-runtime-independent-retirement.md)
tracks implementation and outstanding cross-actor qualification.

## Viewer position follow-up found on 12 November

**SCROLL-01 — implemented:** bounded page replacement now uses the existing rendered
visual/source anchors instead of replacing an average byte-density estimate. Position
and extent share one monotone map, with covered absolute inversion, real endpoints,
wrapped/table spans, and immediate reversal. No whole-file scan or history index was
added. Actual BAT Text software and D3D traces show no opposite thumb steps through
refills; the original failed traces remain preserved. See the
[coordinate verification](verification/2026-11-12-scroll-thumb-continuity.md).

**SCROLL-02 — P1, investigate D3D pacing:** the final valid 12-case matrix has 10 original
wheel/drag guard passes and two D3D sparse-frame failures. A preserved pre-change probe
reproduces 230–652 ms D3D gaps on the same Markdown/runtime/geometry, so the observed
large delay is not unique to the coordinate change. Fresh locked-desktop attribution
places all 55 interior samples from nine slow moving redraws in the DX12 swapchain
frame-latency wait, before Slint item rendering. Windows LockApp owned the foreground;
valid foreground and actual-application qualification remain pending after unlock.
See the [D3D attribution](verification/2026-11-12-d3d-scroll-pacing.md). No renderer
policy or performance gate was changed.
Qualify native input-to-presentation pacing without source scans, weakening the existing
guards, or claiming CPU-source crop correspondence as GPU readback. Software at BAT0.1
preserves approximately +135,+135,-135 pixel motion and returns to the same thumb
position on reversal; that does not establish uniform D3D or universal 60 Hz scrolling.
The [original glyph evidence](verification/2026-11-12-glyph-compositing.md) and
[corrected native gate](verification/2026-11-12-scroll-thumb-continuity.md) retain the
invalid first-batch seek cases and final performance limitations separately.

## CI execution follow-up

The completed Windows Test step took 50m32s across 281 serial Cargo gates; it did
not hang. [Recorded timings and the optimization proposal](verification/2026-11-12-ci-windows-test-runtime.md)
motivated bounded Cargo-native batches. The default is now eight selected integration
executables per command; same-name workspace groups remain indivisible. Exact target
identity validation and successful-batch retirement preserve coverage and failure
artifacts. The [measured experiment](verification/2026-11-12-ci-test-batching.md) ran
the same 19 tests in eight targets: batch eight was 13.8% faster than serial in paired
local runs. This representative result is not a measured full-CI speedup. Library,
binary, example and doctest gates and active feature selection are unchanged.

## Scope and completed baseline

Latest integrated feature work: [embedded Chromium HTML viewer](verification/2026-11-10-html-viewer.md),
[Copy from inline directory trees](verification/2026-11-10-tree-copy.md),
[archive thumbnail capture/cache](verification/2026-11-10-archive-thumbnails.md),
[tree prefix search and archive extension sorting](verification/2026-11-10-tree-prefix-search.md),
[inline tree directory sizes](verification/2026-11-10-tree-folder-sizes.md),
[Make File List](verification/2026-11-09-make-file-list.md),
[viewer associations](verification/2026-11-09-viewer-associations.md), and
[Markdown scrolling/GitHub layout](verification/2026-11-09-markdown-scroll-and-style.md).
The older broad goal entries below retain their separately documented limitations;
this update does not mark unrelated unfinished platform/provider work complete.
The optional Chromium viewer now has [Linux XWayland qualification](verification/2026-11-10-html-linux.md)
through the real installed host and Slint component, in addition to the recorded
Windows installation. Existing cgroup delegation and Chromium sandbox remain
required; native Chromium Wayland and physical IME remain qualification work.

The governing sources are the [main design](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md),
[accepted follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md),
[feature status](FEATURE_STATUS.md), [security contract](../SECURITY.md),
[code style](../CODE_STYLE.md) and [dependency policy](DEPENDENCY_POLICY.md).
Later, more specific extensions supersede the old read-only milestones and old
statements that F3, copy, archives or native mutations do not exist.

The following are implemented baselines, not new backlog items:

| Area | Concrete implementation/evidence | Current scope to preserve |
| --- | --- | --- |
| Panels and operations | `crates/midnight-engine/src/{selection,folder_sizes,operations,operation_metadata}.rs`; [feature status](FEATURE_STATUS.md) | Windows/Linux browsing, tabs/bookmarks, locale sort, watches, prefix search, multiselection/sizes, three modes/themes/shortcuts; F2, Shift+F4, recursive F7, Trash/permanent deletion and metadata/time/case editing. |
| Verified copy | `crates/midnight-platform-common/src/transfer.rs::VerifiedFileCopy`, `crates/midnight-engine/src/{copy_worker,archive_transfer}.rs` | Native/archive F5, Ctrl+C/V, incoming/internal drops; bounded queue, skip/rename, progress/cancel, staging, SHA-256 independent destination readback and no-replace publication. Sources remain. |
| Viewers and comparison | `plugins/midnight-viewer-{basic,source,markdown,image,pdf,html}/`; [viewer controls](verification/2026-11-07-viewer-controls.md), [PDF](verification/pdf-viewer.md) | Text/hex/source/Markdown/image/PDF plus optional [embedded Chromium HTML](verification/2026-11-10-html-viewer.md); wrap/search/page-local copy, raster zoom/Fit/1:1/pan, PDF pages, Escape/Space, external F4, Ctrl+Shift+C text/image comparison. |
| Archive/media plugins | `plugins/midnight-fs-archive/`, `plugins/midnight-thumbnail-{image,video}/` | Read-only archives/compressed streams, saved unavailable tabs, F3/F5 extraction, restricted workers, private thumbnail cache and selected-video sampling. |
| Runtime, windows/dialogs | [Runtime discovery](developer/RUNTIME_DISCOVERY.md), [small window](verification/small-window.md), [geometry](verification/window-geometry.md), [UI follow-ups](verification/2026-11-07-ui-followups.md) | Protected SHA-256-bound Windows runtime registration; exact 320x200 minimum, compact layout, saved bounds, parent-sized viewers/comparator, movable bounded dialogs, navigation-only `..`. |
| Commander tools | `crates/midnight-engine/src/{comparison,filters,filename_colors}.rs`; [feature status](FEATURE_STATUS.md) | Directory comparison, masks/regex, filename colors/presets, keyboard editor, locations/OneDrive, Jobs, command editor/history and terminal launch. |

## Assigned functional requirements

All rows target Windows x64 and Linux x86-64 unless a narrower platform is stated.
macOS is a separate platform dependency (PLAT-01); a shared implementation or
compilation check alone does not establish native support.

| ID / priority / state | Requirement and authority | Current evidence and precise gap | Acceptance criteria / dependencies |
| --- | --- | --- | --- |
| COPY-01 / P0 / partial | Explicit overwrite policy and safe publication. Main design sections 8, 25. | Recoverable replacement is implemented in native Windows/Linux, common verification, queue and UI: Replace/Replace all keep displaced originals, with exact conflict metadata and versioned consent. [Contract](developer/TRANSFERS.md), [evidence](verification/2026-11-07-recoverable-copy.md). | Strong identity-conditional overwrite remains Unsupported. Review found and fixed late Replace-all consent, selected-source alias and snapshot/revision gaps. One intermittent Windows pre-copy revision refusal remains uncaused; no checks were relaxed. Qualify final release and retain truthful recovery/durability reporting. |
| COPY-02 / P1 / implemented; final release pending | Explicit directory merge and type collision handling. | Merge/Merge all use pinned destination directories, source-ancestry checks and independent child conflicts. Existing children remain intact. Native, queue and UI tests pass on Windows/Linux. | Keep metadata/link limits under COPY-03 explicit. Final integration/release validation remains with COPY-01. Never translate a directory collision into implicit recursive deletion. |
| COPY-03 / P1 / partial | Declared metadata preservation and explicit link-copy capability. Main design sections 8 and 11, follow-up stage 3. | Explicit required modified time is implemented for up to64 distinct native regular files on Windows/Linux: pre-read source pin, complete SHA-256/readback, owned-stage set/sync/exact time readback before publication, plus Move proof checks. Existing APIs remain ContentOnly. The unchecked per-dialog option passed Windows/Linux software-window tests; the current Windows release passed native startup. [Contract](developer/TRANSFERS.md), [option](developer/TRANSFER_OPTIONS.md). | Retain current UI/backend qualification and add separately declared remaining times/attributes, permissions/ACLs/xattrs/streams and new-directory postorder metadata. Links/special files remain refused; preserve-link semantics must never follow referents or weaken no-follow safety. Hardlink topology and general provider metadata remain unsupported. Kind/capability preflight precedes writes; value-specific precision failure can occur after data verification on an unpublished stage, with earlier completed files retained and no removal of the failed source. |
| MOVE-01 / P0 then P1 / partial | F6 and Cut/paste Move with per-file verified source removal. | Qualified Windows NTFS and Linux ext regular files and ContentOnly ordinary directories now use ordinary Move, with independent SHA-256/readback, immediate per-file source removal and postorder empty-directory cleanup. Skips retain source ancestors; overlap and unsupported kinds fail before mutation. [Contract](developer/TRANSFERS.md), [recursive evidence](verification/2026-11-08-recursive-move.md). | Both OS backend/strict gates pass. Complete combined application qualification and retain explicit live-namespace limits, earlier completed files on later error/cancel and truthful durability counts. Required modified time remains file-only. Links, provider Move and remaining metadata are pending. No source-recovery catalogue, measured-freed-space claim or whole-batch rollback. |
| FIND-01 / P1 / implemented; qualification in progress | Separate modeless recursive Find. Follow-up stage 5. | Windows/Linux backend: four sessions, two scans, masks/multiple roots/literal streamed UTF-8, cancellation and stable result IDs. Alt+F7/magnifier opens independent native windows with lazy results and validated Go to file. [Contract](developer/FIND.md). | Complete final tooltip/small-window/native application qualification. UI rejects old generation/result tokens and prevents shutdown resurrection. Preserve raw roots, explicit link/mount/encoding/budget exclusions, backend-only I/O and deferred result reclamation. Archives and guarded Open/Edit remain unsupported. |
| SELECT-01 / P1 / implemented; qualification in progress | Rectangle selection in Details, Brief and Thumbnails, including archives. | Empty-space drag emits bounded disjoint ranges against captured listing/selection revisions; Ctrl/Shift, provider IDs, auto-scroll and stale cancellation preserve row-based native file dragging. Windows software UI9, failure2 and existing drag1 regressions pass. [Contract](developer/SELECTION.md). | Complete Linux and native release checks, density/font/320-window coverage and keyboard/settings integration. Do not count old Details-only test harness runs as all-layout evidence. |
| CMD-01 / P1 / implemented; follow-ups | Custom Commands toolbar, settings and execution. [Design](superpowers/specs/2026-11-07-user-commands-design.md). | Stable IDs, schema 17 persistence, native Program argv/cwd, literal PowerShell/POSIX scripts, selection captures, previews and process permits are implemented. Windows/Linux backend and UI tests pass, including settings drag ordering, scrolling to Apply, raw-path preservation, dynamic shortcuts, toolbar routing and stale callbacks. [Settings evidence](verification/2026-11-07-custom-command-settings.md), [release checkpoint](verification/2026-11-07-goal-checkpoint.md). | Automatic Windows/Linux executable artwork is implemented for visible Program commands without an explicit image; both paths share the bounded backend service and icon budget. [Artwork qualification](verification/2026-11-12-command-executable-icons.md) covers native lookup, stale generations and actual app-timer delivery. Unavailable host artwork retains the generic Run icon. Cmd Custom Commands now have native environment-reference qualification, exact working-directory checks and explicit refusal of multiline/native-limit cases; `_ARG` companions are qualified for direct external argv, not arbitrary CALL/builtin reparsing. [Cmd evidence](verification/2026-11-12-cmd-custom-commands.md). Native macOS support depends on PLAT-01. Retain cancellation/queue limits, the 256-total icon target budget and explicit launch consent; never interpolate filenames into shell source or elevate implicitly. |
| HASH-01 / P1 / implemented; qualification in progress | Ctrl+F8 checksum calculation, file verification and export. [Contract](developer/CHECKSUMS.md). | Native Windows/Linux actor streams one read through SHA-256/SHA-512/MD5/SHA-1/CRC32, validates revisions and bounds resources. Modeless UI captures selection, streams rows, supports per-hash/full-row copy and staged export with exact names and partial acknowledgement. Explicit manifest verification supports the five corresponding formats, including SFV, with bounded UTF-8/BOM-marked UTF-16 parsing and pinned native paths that reject links and cross-device descendants. Focused Windows/Linux backend and UI checks are recorded in [checksum-file evidence](verification/2026-11-08-checksum-files.md). | Finish combined application and packaged native release qualification; focused reports do not establish a green current workspace or CI result. Ctrl+F7/custom bindings survive migration. Actual hash dependency licenses/notices are recorded. Native scheduling remains serial while IO-01 is pending; matching checksums do not authenticate sources or provide snapshots. |
| VFS-01 / P1 / partial | Dynamic native filesystem plugins and generic provider migration. | Independent Windows/Linux native libraries, public ABI1.1/SDK and trusted host now supply production panel Browse/Stat/Storage and native F3 capture. Rich volume queries preserve raw labels and independent capacity; storage classification requires positive SSD proof. Build/release helpers and independent plugin CI package the required native provider. Missing packages do not fall back. [Native API](developer/NATIVE_FILESYSTEM_API.md), [cutover plan](superpowers/plans/2026-11-08-native-filesystem-host.md). | Windows native/software startup and Linux headless navigation pass; both systems pass installed app-service tests for all five viewer formats and reject missing providers. [Cutover evidence](verification/2026-11-08-native-filesystem-cutover.md), [Linux final qualification](verification/2026-11-08-linux-viewer-cutover.md). Complete release qualification and remaining provider migration, including mutations, transfer, watch, Find and generic provider identities. Native comparison/thumbnail captures now use the same installed provider, with live cancellation and no fallback; [media evidence](verification/2026-11-08-native-media-capture.md). New/restored archive captures now share that provider and accept only exact legacy revision equivalents; checked-read dispatch avoids redundant validation while preserving the SDK contract. Windows/Linux integration and Windows deployment pass; [archive evidence](verification/2026-11-09-archive-native-capture.md). Preserve lossless settings, parent context, icons, sorting, selection and explicit unavailable states. These checks do not establish full production generic migration or Linux GPU/gesture qualification. Generic writes must retain verified-copy publication guarantees. |
| TREE-01 / P1 / partial; browsing, prefix/filter and sorting integrated | Inline expandable directory/archive/provider branches in the existing listing. Follow-up stage 7. | Production workers, typed projection/selection, Commit validation, disclosure controls and Enter/double-click directory navigation drive native/archive panels. Prefix/quick search examines loaded visible raw names without enumeration; persistent Masks/Regex/Expression filters compose with the prefix and retain unselectable ancestor context. Sibling sorting, rows and rebased selection await correlated backend receipts. One latest desired query plus one in-flight request preserve input under queue pressure; Insert waits for cleared search and settled selection. Shared extension sorting uses actual native or UTF-8 archive basenames with prepared locale keys. [Prefix scope and qualification](verification/2026-11-10-tree-prefix-search.md), [earlier filter/sort evidence](developer/TREE_PANEL.md#filter-and-sort-evidence), [bounded scheduling](verification/2026-11-09-tree-budget.md) and [top-level selection](verification/2026-11-09-tree-select-all.md). | Complete installed native search/filter/sort and full tree workflow qualification; finish remaining typed operation consumers (native Delete and focused F2 Rename now use captured targets and ordinary confirmation; Rename retains the shared versioned overwrite flow), container discovery/attachment, nested archives and archive watches. Native manual/watch maintenance retains expanded branches; its separate qualification does not establish complete tree parity. Keep unsupported actions guarded; software-window/model tests and debug startup do not establish complete production action support. |
| NET-01 / P1 / implemented; live SMB qualification pending | Network Neighborhood discovery provider and manual targets. [Network design](superpowers/specs/2026-11-05-network-filesystem-design.md); [developer contract](developer/NETWORK_FILESYSTEM.md). | Independent midnight-filesystem-network plugin has Windows MPR and Linux GIO/GVfs adapters, paged catalog/cache/cancellation, ABI 1.2 explicit activation, typed host bridge, Network panel navigation and saved tabs. Windows/Linux plugin builds, installed metadata probes, packaging and input/host tests are recorded in [qualification](verification/2026-11-09-network-neighborhood.md). Windows debug runtime is installed and startup-tested. | Qualify controlled real SMB/FUSE endpoints; fixture tests do not prove interoperability. Typed discovery Hot Paths now preserve provider/opaque identity through settings schema 29 and reopen with Browse, including unavailable providers; [identity, persistence and shortcut evidence](verification/2026-11-11-network-hot-paths.md). Interactive Linux authentication remains unimplemented; native share Hot Paths also work. macOS requires its own later adapter. No subnet/port scan or claim of complete network visibility. |
| FILTER-01 / P1 / partial; tree filter/prefix routing integrated | Typed metadata expressions alongside masks and regex. Follow-up stage 8; main design 2026-11-07 color extension. | Flat panels implement typed validation, Ctrl+F12 mode/help/diagnostics and schema18; [syntax/limits](developer/METADATA_FILTERS.md), [Windows/Linux evidence](verification/2026-11-07-metadata-expression-filters.md). Adopted trees compose persistent filters, transient prefix/quick search and sibling sorting through backend reprojection, retain unselectable ancestor context and commit only a matching receipt. Query/preference preview and Cancel preserve that composition. [Prefix scope and qualification](verification/2026-11-10-tree-prefix-search.md), [earlier Windows debug evidence](developer/TREE_PANEL.md#filter-and-sort-evidence). Captured metadata keeps three-valued missing semantics; opaque archive captions do not invent name encoding or prefix matches. | Qualify installed native tree search/filter/sort workflows and complete TREE-01 operation consumers. Preserve last-valid filters, selection/identity guards and separate refresh semantics. Native manual/watch refresh retains the adopted tree; archive refresh remains separate. No script, network or arbitrary filesystem access during evaluation. |
| IO-01 / P2 / partial | Measured SSD-only parallel verified copy and shared storage arbitration. [Copy concurrency design](superpowers/specs/2026-11-04-copy-concurrency-design.md). | `midnight-engine/src/copy_worker.rs` owns one current `VerifiedFileCopy`; `midnight-platform-common/src/transfer.rs` uses a reusable 256 KiB buffer and process I/O permits. Native copying is serial even SSD-to-SSD. Folder-size SSD hints do not establish physical copy scheduling. | Enable bounded different-file concurrency only when both endpoints are positively local SSD; HDD/network/unknown/mixed/error remain serial. Charge reads, writes, sync and full readback to shared physical-device/endpoint budgets across scans/copy/thumbnails/checksums; aliases cannot multiply permits. Avoid A-to-B/B-to-A deadlocks, bound handles/buffers/CPU and keep inflight ownership until OS calls return. Measure verified end-to-end throughput and UI p95/p99 for 1/2/4 streams, topology aliases, many-small/mixed/large files and cancel/error cases before raising defaults. Keep SHA-256; BLAKE3 in historical recommendations is not a required algorithm migration. |

TREE-01's native clipboard consumer is now implemented in source. Ctrl+C/Ctrl+X
capture exact acknowledged native occurrences; Ctrl+V keeps the native panel root
destination and ordinary verified Copy/Move behavior. Windows/Linux backend
libraries, Windows clipboard/F5/F6 UI bridges and the Windows debug deployment
passed their recorded gates. Linux UI and native OS clipboard interoperability
remain unqualified; bridge tests substitute the clipboard boundary. Archive/mixed
exports and other guarded tree consumers are not enabled. See the
[clipboard evidence](verification/2026-11-10-tree-clipboard.md).
Yellow size-warning reports are connected through native/archive/tree scanners,
backend snapshots and all panel views. Red diagnostics and full-error Copy retain
exact occurrence authority. Windows/Linux backend and Windows UI evidence,
deployment status and remaining desktop limits are recorded in the
[diagnostics qualification](verification/2026-11-10-panel-warning-details.md).

## Retained platform, integration and reliability work

| ID / priority / state | Evidence and scope | Acceptance / dependencies |
| --- | --- | --- |
| NAV-01 / P1 / implemented in source; release pending | Native path-editor paste capture and backend clipboard decoding now append the native separator; Windows/Linux focused tests pass. See [path paste](developer/PATH_PASTE.md) and NAV-02 below. | Cover keyboard and context-menu paste, replacement and insertion, Unicode, already-terminated paths, Windows roots/UNC and Unix paths. Preserve literal path identity; do not infer a paste from normal typing, trim valid characters or navigate until Enter. |
| VIEW-PERF-01 / P1 / partial | Cached glyph runs skip scaler setup; measured short-line rendering improved approximately 24%. NUL-separated UTF-8 text detection and stronger syntax colors are implemented and tested. [Evidence](verification/2026-11-07-status-and-viewer-readability.md). | Retained text/source/Markdown/hex pixels and exact-file JSON/XML scrollbar mapping are implemented; all eight exact-file GPU/software wheel/drag runs pass, with typical 16–17 ms wheel gaps and occasional gaps up to 66.519 ms. Default GPU startup failures recover once to CPU. [11 November evidence](verification/2026-11-11-viewer-pixel-scroll.md). Continue page-boundary, long-line, PDF and Linux desktop qualification; uniform 60 Hz is not established. The older 256 KiB single-line fixture still costs approximately 21.8 ms/frame; paging and long-line performance need further work. Keep worker-only rendering, bounded memory, exact Unicode shaping and cancellation. Renderer timings alone do not establish instant scrolling. |
| BUG-01 / P1 / fixed, native release verified | [Causal report](verification/2026-11-07-keyboard-search-crash.md): first-character filtering destroys a hovered command row while Slint1.18.1 materializes its tooltip, causing `active_popups` RefCell reentrancy at `window.rs:2252:52` and abort `0xc0000409`. The historical dump source Location agrees with the fresh native reproduction. Keyboard search retires tooltip popups before borrowing Presenter or replacing rows. | Pre-fix release executable/panic/screenshot retained under `_build/goal/keyboard-current/causal-red/`. New timed-tooltip regression passes; corrected native SendInput standard/dynamic/320×200/large-font matrix passes657 assertions with unchanged actual preferences/application hashes. No exception swallowing, dependency patch or trust relaxation. Broader current-source gates and deployment remain separately recorded checkpoint work. |
| DEPLOY-01 / complete, Windows native verified | Fixed CLOUD_6 Desktop rejection through separate `scripts/windows-shortcut.ps1`/`windows-shortcut-native.cs` policy, Unicode COM staging/readback and handle-relative same-volume publication. Existing plugin/deployment/registration trust functions are unchanged. [Verification](verification/2026-11-07-desktop-shortcut.md). | Focused Unicode/cloud/junction/hardlink/retag/no-replace/cross-volume tests and existing deployment/registration suites pass on PowerShell 7/5.1. Actual existing OneDrive Desktop shortcut targets verified `MidnightExplorer-20261107-120804-5c2b92b9/me.exe`; icon/cwd/empty arguments/hash read back. No app launch or preferences writes. Evidence: `_build/goal/desktop/report.md`; Windows 10 remains platform qualification, not an unresolved shortcut implementation. |
| TERM-01 / P1 / missing accepted follow-up | Automatic reuse of a safe external terminal/sudo session. Main design section 24 and [terminal session design](superpowers/specs/2026-11-04-terminal-sessions-design.md). `midnight-platform-linux/src/desktop.rs::launch_terminal` spawns a fresh process/TTY for each call; `TerminalService` has no reusable session API. Existing shell picker, literal path insertion and explicit launch are complete. | Qualify at least one concrete Linux terminal bootstrap; use one unprivileged broker with kernel-verified process/UID/lifetime identity and separate bounded IPC. Only explicit commands enter an idle session; Busy, disconnected and unknown outcome are distinct; no retry that could execute a command twice. System sudo owns authentication/cache expiry; no password storage, `sudo -S`, sudoers change or indefinite keepalive. Close/timeout/app shutdown own cleanup and revoke session admission. Windows may retain new-process behavior. macOS waits for PLAT-01 and its own bootstrap qualification. |
| PLAT-01 / P1 / missing platform | macOS adapter, packaging and native conformance. Main design sections 7, 15, 23, 39; [system requirements](SYSTEM_REQUIREMENTS.md). `midnight-platform/src/lib.rs` has a compile error outside Windows/Linux; loader, worker containment, settings/cache/window/input/clipboard/terminal implementations also require platform selection. The independent read-only [Darwin native provider](../plugins/midnight-filesystem-native-macos/IMPLEMENTATION_PLAN.md) has source and portable/cross-check coverage; native execution remains pending. No supported macOS application version/architecture is established. | Reuse that provider and add separate platform files/crates with all required services, Unicode/raw identities, secure mutations/transfers, volumes/watch/icons/fonts/locale, settings, Trash, editor/terminal, window geometry and drag/drop. Supply trusted plugin loading, media/PDF containment, packaging/notices and CI. Choose and record OS/architecture/toolchain/RAM targets based on native evidence. A macOS machine/runner is a validation dependency; its absence does not prevent implementation, static review or cross-platform contract tests and does not by itself block the overall goal. Do not claim native success from cross-compilation. |
| PLAT-02 / P2 / validation | Qualification of compatibility targets and distribution. [System requirements](SYSTEM_REQUIREMENTS.md) records Windows 11 development checks and Ubuntu 24.04 WSL2 evidence, with dedicated Windows 10, native Mint, Fedora/openSUSE, Ubuntu 26.04 and physical desktop/GPU coverage pending. Fedora RPM/openSUSE packaging is planned there. | Record exact OS/kernel/desktop/GPU/driver/filesystem and rolling snapshot. Exercise native WGPU multilingual rendering/IME/RTL/emoji, filesystem operations, shell/clipboard/drag/drop, plugin trust/containment and packaging. Test SELinux where relevant and Btrfs/ext4. Measure runtime memory/large listings/parallel jobs and reduced-RAM build configurations before turning planning allowances into measured minima. Include dependencies and notices in each artifact; source build instructions alone are not native distribution qualification. |
| PLAT-03 / P2 / partial integration | Native Wayland external file drag/drop. `midnight-platform-linux/src/file_drag_drop.rs` and [security contract](../SECURITY.md) intentionally support XDND through a worker-owned X11 proxy; native Wayland external transport lacks same-connection seat/data-device integration. Internal panel dragging works. | Implement a narrowly scoped Wayland bridge with correct connection/seat ownership, raw identity transfer, bounded admission and the existing copy-only safety contract, then qualify on native compositors. Keep explicit unavailable status until then; do not advertise X11/XWayland tests as native Wayland transport success. Depends on an appropriate toolkit/native integration and native compositor validation. |

## Latest accepted follow-ups

These are active assigned work, not optional later proposals. The governing
[current extension](superpowers/specs/2026-11-07-panel-viewer-followups.md) includes
the latest user corrections. Working-tree tests do not establish that a feature
has been packaged or deployed. Refresh each row when the corresponding workflow
is verified; preserve the existing feature/security contracts.

| ID / state | Accepted requirement | Current evidence / next acceptance |
| --- | --- | --- |
| HOTPATH-02 / source implemented, integration qualification pending | Add, persist and open Hot Paths inside archives using `container.zip@member` display syntax and native separators | Typed provider/container/member/decoder persistence, deduplication, queue rejection and stale navigation tests pass. The 8 November settings/presenter checks and 494-test combined Windows UI gate are recorded in [follow-up evidence](verification/2026-11-08-panel-viewer-followups.md) and `_build/goal/archive-hotpaths/{identity-green,combined-ui-green,combined-ui-integrations}.log`. Native/raw paths and literal `@` remain identities rather than parsed display text. Re-adding refreshes a changed source revision while preserving its shortcut slot. The 11 November native manager retains the typed location captured at opening; focused checks cover archive capture after navigation, deduplication, numbered-slot swaps and stale callbacks within the 22-pass Windows Filter/Hot Paths gate, including visible creation-failure recovery without bookmark mutation. [Native manager evidence and limits](verification/2026-11-11-native-tool-windows.md). Qualify the complete menu/shortcut, restart/restore and unavailable-provider workflow with real archive plugins before deployment. |
| DIALOG-02 / partial | Resizable dialogs with top-right X equivalent to Cancel; independent movement for modeless working tools. Ordinary confirmations/settings are modal without separate taskbar entries; Ctrl+M is owner-modal and Attributes / Times / Case stays embedded | The earlier native-host batch recorded 88 targeted Windows and 63 Linux checks, actual Windows ownership/foreground checks, debug build, deployment and startup qualification. These remain dated evidence. The [current follow-up](superpowers/plans/2026-11-11-dialog-polish-followup.md) supersedes further blanket modal migration; review existing native modal hosts against that policy and finish broader desktop/monitor qualification. [Earlier host evidence](verification/2026-11-11-native-tool-windows.md), [operation evidence](verification/2026-11-11-native-operation-windows.md). |
| OPTIONS-02 / partial | Working Save, visual color picker, native Browse, editor argv rows instead of JSON, Salamander User Menu form and drag ordering | Save state/retry tests, actual picker widget and native chooser cancellation have focused evidence. Combined forms/Save/stale-picker checks remain part of final qualification. Native end-to-end explicit command argv/cwd, release deployment and wider layout checks remain. [Ergonomics](developer/DIALOG_ERGONOMICS.md), [evidence](verification/2026-11-07-options-ergonomics.md). |
| META-02 / partial | Attributes/Times/Case icons and readable forms without the obsolete preview pane | Compact shared form and icon changes have focused tests; immutable backend review/confirmation remains. The dialog remains embedded under the current policy; qualify real metadata editing and cancellation without a native-host migration. |
| MOVE-02 / ordinary backend implemented | Remove the source-recovery product and use per-file ordinary Move | Qualified local NTFS/ext regular files and ContentOnly ordinary directories are implemented. Each source file is removed immediately after its destination proof; directories are removed postorder only while empty and identity-valid. Cancellation/errors/Skip retain unfinished sources. Windows engine67/native28/proof24 and Linux59/30/24 plus strict checks pass. [Evidence and limits](verification/2026-11-08-recursive-move.md). Required modified time is file-only; links/provider Move and wider metadata remain pending. |
| FIND-02 / source implemented; debug verified | Find Masks, Regex, Expression; one row per result; sorting headers and current scanned directory | Backend locale/numeric ascending/descending sorting keeps discovery IDs authoritative and selected IDs stable. Directory status is sampled at most 4 Hz while native open/enumeration/content I/O blocks. The 8 November Windows/Linux software-window mode chooser, all five pointer headers, row/header geometry and minimum-size checks pass; 19 backend cases pass, including sorting after Stop. Ctrl+F/Alt+F7 routing and viewer document search remain separate; content matching stays literal UTF-8. [Contract](developer/FIND.md), [dated evidence and logs](verification/2026-11-08-find-refactor.md). Integrated deployment/native-desktop qualification, archive search and guarded Open/Edit remain pending. |
| FILTER-02 / source implemented, qualification pending | Panel toolbar filter icon and visible applied-filter indicator | Original funnel icon and marked active state are implemented alongside Ctrl+F12 and the textual active filter. The native owner-modal filter now retains its captured pane/navigation and form generation; focused Windows/Linux checks cover draft preservation, Cancel, stale Apply/Show all callbacks, compact first-open Escape and failed-create cleanup with a visible owned Notice that preserves the applied filter. [Native filter evidence](verification/2026-11-11-native-tool-windows.md). Qualify icon visibility in normal/narrow panels and the installed apply/reject/clear/navigation workflow; last valid filter survives invalid edits. |
| DRIVE-02 / source implemented, Windows/Linux backend verified | Ctrl+F1 detailed useful volume information, or remove the action | Ctrl+F1/Ctrl+L now query native filesystem, root, label, capacity, storage class and reported properties asynchronously. Used/free/caller-available remain distinct; unknown fields remain unavailable. Windows/Linux native raw/long-path and strict backend checks pass; the actual Windows dialog verifies details, Copy, resize and no late revival after dismissal. [Contract](DRIVE_INFORMATION.md), [evidence](verification/2026-11-08-drive-information.md). Deployment remains pending. |
| STATUS-02 / source implemented, Windows verified | Known selected-folder size totals without `unknown` messages | Presentation omits unmeasured size fields and preserves approximate partial values and exact byte/locale formatting. Regression and the combined 494-test Windows UI unit gate pass. Deployment remains pending. |
| VIEW-02 / implemented; Windows native controls verified | Source line-number toggle default on, current logical line/column; Markdown GitHub default, Compact/Comfortable and font/size controls | Bounded source indexing uses an optional independent-plugin API and never delays viewport acknowledgement. Columns count Unicode scalar values; unavailable/indexing states never invent numbers. Actual Windows controls and a 20,000-line source fixture pass; final latency and packaged-app evidence are tracked in [verification](verification/2026-11-08-viewer-controls.md). [Presentation contract](developer/VIEWER_PRESENTATION.md). |
| VIEW-03 / implemented; native restart qualification pending | Viewer preferences survive closing/reopening and application restart | Strict schema21 stores wrap, line numbers, search case, text/source/Markdown fonts and sizes, Markdown preset and image/PDF zoom modes. Field patches merge concurrent windows through backend atomic storage, including immediate Escape. Codec and manager-recreation tests pass; these are not an actual app restart. PDF page changes preserve exact manual zoom and recompute Fit for each page. No query text, file contents or reading positions are stored. |
| VIEW-PERF-02 / implemented; measured debug improvement | Viewport-driven text/source, faster image viewer, working Ctrl+F and responsive scrollbar dragging | Bounded latest-position queues and intermediate content fix continuous-drag starvation. Causal Windows profiling then found the requested Direct3D policy silently selected software; explicit WGPU30/DX12 now proves actual GPU rendering. Debug text/source/image input-to-redraw p95 improved from 184/198/147 ms to 16/31/23 ms. Existing software pixel loops also use opt-level2. These are redraw-submission observations, not guaranteed compositor 60 Hz. All 128 viewer tests and strict checks pass; [measured evidence and remaining limits](verification/2026-11-08-viewer-latency.md). The combined Windows debug app is deployed with native/software startup and all-format viewer acceptance; optimized PDF panning remains tracked. |
| MEDIA-03 / partial | Loaded thumbnails remain when scrolling back and reuse persistent cache | Bounded visited-image retention and revision/cache handling have focused tests. Qualify actual scrolling/refresh/provider changes and preserve selected-video scheduling/resource limits. |
| PANEL-02 / partial | Largest same-height icons, subtle archive marker, middle path ellipsis, pixel wheel coast then nearest-row snap | Archive marker committed with four-theme rendering. Middle-ellipsis identity/copy/edit test and eight wheel-motion tests pass; native synthetic WGPU probe shows 39 intermediate changing offsets. Actual deployed combined behavior still needs qualification. |
| NAV-02 / partial | On navigating up, focus the folder just left; pasted path gets native separator | Native/archive parent identity and actual MainWindow focus test pass. Path-paste Windows/Linux widgets/backend checks are documented. Combined release qualification remains; supersedes the earlier NAV-01 missing assessment. |
| CMD-02 / implemented in source, release pending | Ctrl+Enter filename and path shortcuts insert without added quotes or spaces | Raw insertion and command binding tests pass on Windows/Linux; filenames retain their own literal characters. No automatic execution. |
| THEME-02 / implemented, Windows deployed | Additional popular palettes, retro replacement and three light themes | Tokyo Night, One Dark, Rosé Pine and Norton Commander retain IDs9–12. Amber Terminal replaces DOS Commander at13 with a saved-name migration; Verdant Light, Catppuccin Latte and Rosé Pine Dawn append IDs14–16. Windows rendered UI/presenter and Windows/Linux persistence checks passed; provenance and distribution notices are recorded. See [verification](verification/2026-11-09-theme-refresh.md). |
| ICON-02 / source implemented, Windows verified | Replace the arrow archive overlay with a clearer zipper motif inspired by the attached reference | Original small vector zipper preserves row height and system artwork; the archive fallback does not get a duplicate badge. The rebuilt actual UI archive-badge tests pass across all 14 themes and panel modes. No association/provider behavior changes; deployment pending. |
| NOTICE-02 / source implemented; native qualification pending | Cross button on every inline error notice, including archive/navigation failures | Shared notice closes/collapses presentation without clearing host errors; changed text/occurrence restores visibility. Actual component close/repeat tests pass. Compact panels also use ErrorNotice with its close control; source implementation is complete for this control. Qualify repeated identical host failures and native deployment separately. |
| ARCH-04 / source implemented, Windows workflow verified | Add/remove custom extensions per archive format in Options | All eleven format groups retain built-in checkboxes and custom Add/Remove with preserved invalid drafts and actionable built-in conflicts. Unicode case folding, global duplicate prevention, longest compound suffixes, typed decoder persistence and trusted capability checks have backend coverage. Rebuilt Windows Options and presenter workflows pass; optional Office associations remain disabled by default. [Evidence and platform limits](verification/2026-11-08-archive-custom-extensions.md). Deployment remains pending. |

Development iteration now defaults to debug/test. Release/fat LTO is reserved for
final delivery or an explicit performance comparison; do not repeat release
builds for routine UI changes.

Independent Windows/Linux native filesystem API/plugin projects now have a host
consumer with strict dynamic discovery/loading, bounded resource ownership and a
16-reader cache for large viewer orders. The main panels use its Browse/Stat/Storage
services, including richer volume details, and native F3 uses its content capture.
Normal build/release helpers package the required provider; missing packages do
not fall back. [Cutover evidence](verification/2026-11-08-native-filesystem-cutover.md)
and [Linux all-format evidence](verification/2026-11-08-linux-viewer-cutover.md)
record installed app-service checks for all five viewer formats, missing-provider
rejection, Windows native/software startup and Linux headless navigation. Full
release qualification and migration of writes, watches and other host services
remain open. These checks do not establish a green current workspace or remote CI,
Linux GPU/gesture qualification, network discovery or inline-tree completion.

## Deliberate later capabilities and current limits

These preserve product direction and disclosed limits. Later capabilities need
explicit contracts before their controls are enabled.

| ID / state | Current evidence / authority | Later acceptance and dependency |
| --- | --- | --- |
| ARCH-01 / streaming implemented; application qualification pending | Protocol 2 streams captured source ranges and verified members through host-owned private disk storage; production avoids complete source/member Vec buffers. `session.rs::Mount::open_captured` retains the 64 MiB source cap only for protocol 1; protocol-2 parser defaults impose no artificial source/member/aggregate-disk/ratio byte ceiling. The [9 November installed Windows/Linux worker evidence](verification/2026-11-09-archive-streaming.md) covers an 86,013,866-byte ZIP and 180,858,888-byte member, all eleven formats, corrupt tails/solid siblings, cancellation and storage failure. [API and limits](developer/FILESYSTEM_API.md). | Qualify the integrated application/UI with the matching protocol-2 package and retain format-specific ZIP64/large-content acceptance coverage; the virtual five-GiB TAR index test is not universal >4 GiB qualification. Preserve two mounts, 512 MiB worker memory, 20-second progress watchdog, 256 KiB I/O, 100,000 entries, 8 MiB names, bounded decoder windows/64 MiB raw 7z header and live source validation. Private TEMP availability/native offsets govern content length; compressed 7z header allocation relies on process containment. Legacy protocol 1 retains its original limits. Nested sources remain ARCH-02; no native path length is restricted by archive budgets. |
| ARCH-02 / missing later mount capability | General VFS design explicitly includes archive-in-archive, but current mounts capture a native `CapturedFile`; [current archive extension](superpowers/specs/2026-11-07-archive-copy-pdf-comparison-design.md) excludes nested mounts. | Add provider-content leases as mount sources with bounded aggregate nesting/decompression/cache, parent anchors, independent readers/cancel and invalidation when the outer source changes. Test native-to-archive-to-archive-to-viewer and shared solid-block readers without deadlock. Depends on VFS-01 and ARCH-01 content-source work. |
| ARCH-03 / later write/auth/export | Current archive API is read-only; encrypted/unsupported members fail explicitly. F4/external activation and writes/moves inside archives remain unavailable. RAR is absent; existing ZIP/7z/GZIP/TAR/XZ/Bzip2/Zstd workflows are real. [VFS design](superpowers/specs/2026-11-04-viewer-vfs-design.md), [archive extension](superpowers/specs/2026-11-07-archive-copy-pdf-comparison-design.md). | Archive writes need staged outer-container reconstruction, independently reread member verification and safe outer publication/conflict handling. Editing/external launch needs explicit export lifetime/writeback rules; passwords require separate host-owned ephemeral auth without URI/log persistence. Additional formats/methods/multivolume support need actual fee-free license/feature review and hostile fixtures. No implicit obligation to add RAR or unsafe external extraction. |
| MEDIA-01 / native/archive sources implemented; cooperative cancellation verified; UI qualification partial | Typed native/archive targets share the bounded thumbnail service and private revision-aware cache. Protocol 3 now cancels one member-staging request at existing checkpoints, drains its exact acknowledgement and retires only its spool while preserving the browsing mount, scratch and cached leases. Installed ZIP and solid 7z tests prove partial-read cancellation followed by listing/cache reuse and successful retry on the same mount; all 11 format controls pass. Windows/Linux scoped host, thumbnail and API checks pass; the deployed Windows application starts successfully with the qualified protocol-3 workers. [Current evidence and limits](verification/2026-11-10-archive-thumbnails.md). | Qualify actual native panel scrolling/selection/refresh and frame pacing, all-format persistent reopening and release qualification beyond the verified Windows debug deployment. Generic remote providers still require authenticated readable leases and network opt-in. Legacy protocol 1/2 reads still drain; cooperative cancellation waits for the next checkpoint and never releases real worker/I/O capacity before native calls and cleanup finish. Never convert display archive paths to native member filenames or use public extraction folders. |
| MEDIA-02 / intentional codec limit | Video previews exist, but the documented minimal FFmpeg build enables VP8/VP9/Theora/FFV1/raw with Matroska/WebM/AVI/Ogg, not general MP4/H.264 coverage. Image formats and first-frame behavior are likewise bounded. See [worker SDK](developer/WORKER_SDK.md) and thumbnail design. | Add a codec/container only after actual pinned native/wrapper sources, build flags, licenses, required notices/source distribution and any mandatory-fee implications satisfy policy. No requirement to add audio/full playback, GPU decode, patent-fee dependencies or every proprietary format. Unsupported media keeps its ordinary icon/error behavior. |
| VIEW-01 / intentional bounds | Loaded-page text copy, bounded raster/PDF decoding, first-frame animated images and native-file-only bounded comparison are documented in [viewer controls](verification/2026-11-07-viewer-controls.md) and [PDF verification](verification/pdf-viewer.md). | Whole-file selection, animation, wider PDF compatibility and provider comparison/editing need separate bounded contracts. Existing viewers are implemented; preserve inert links and containment. |
| REMOTE-01 / later provider families | FTP/FTPS/SFTP and other remote filesystems are extension points in main design sections 14/23 and the VFS design. Known native network paths already work; NET-01 discovery alone is not a transport provider. | Separate read/auth/watch/seek/mutation capabilities, typed endpoint identities, no credentials in settings/logs, explicit missing/offline/auth errors. Every destination copy still needs independent complete readback or a separately justified trusted equivalent; server checksum absence never disables verification. Depends on VFS-01 and fee-free dependencies. |
| REG-01 / later Windows namespace | Main design section 23 and VFS design section 7 retain Windows Registry as a later read-only plugin. No Registry provider exists. | Distinguish keys/values/native views, retain raw names/type/data, bounded typed F3 values; no automatic expansion/elevation/execution. Export and future writes need their own contracts; Registry values are not ordinary file byte copies. Depends on VFS-01; no fake Linux/macOS Registry provider is required. |
| HASH-02 / later | Additional algorithms beyond SHA-256/SHA-512/MD5/SHA-1/CRC32, a hash plugin API and disk-backed history remain outside HASH-01. Manifest verification and SFV/CRC32 are implemented under HASH-01. | Define separate bounded contracts and dependency review for these extensions; preserve explicit failure, changed-source and cancellation results. Matching checksums are not source authentication or snapshots. |
| PERF-01 / deferred optimization | Single-file parallel blocks, GPU hashing/decoding, BLAKE3 substitution, parallel mutations and universal process isolation are separate proposals. | Require measured benefit and ownership/security contracts. CPU SHA-256 and WGPU presentation remain valid; proposed numbers are not measured performance. |

## Text viewer performance checkpoint, 10 November

Visible-range loading is qualified for the independent basic, source/JSON and
Markdown plugins on Windows and Linux. Real native 10 GiB files also pass the
Windows production Service probe: no whole-file allocation, scan or line index,
including distant automatic coordinate lookup. First-page and seek I/O remain
bounded independently of file length. See [measurements and limits](verification/2026-11-10-large-text-viewers.md).

Fresh-session first paint remains **341–372 ms** in the final Windows debug probe,
mostly installed-plugin discovery; seek is **8–18 ms**. Success-only caching of
the fixed Windows TrustedInstaller SID reduced cold initialization without
caching permissions. The user's one-millisecond aspiration is not established.
Further catalog initialization optimization needs to preserve user association
priority, live trust checks before loading, bounded workers and cancellation;
do not substitute plugin-only timings for complete first paint. Archive member
preparation has separate streaming-to-disk latency.

## Completion and platform evidence rules

Sequence COPY-01/02 contracts before overwrite/merge moves; develop independent
FIND-01/SELECT-01/CMD-01/HASH-01 in parallel where ownership permits. VFS-01 feeds
TREE-01/NET-01/provider content; FILTER-01 and IO-01 integrate with these services.
TERM-01 remains an independent accepted workflow.

Close rows with source/API, native workflow, failure/cancel/race evidence and
tested platform. Preserve separate OS implementations, backend I/O/hash/traversal,
raw Unicode identities, no MAX_PATH, bounded queues and worker cleanup. Dependency
changes require pinned license/feature/advisory checks, [inventory](THIRD_PARTY.md)
and notices with no mandatory fees. Outputs stay under `_build/`, caches under
`.tools/`. A design, enabled button or happy-path check alone does not close work;
an absent native runner does not block independent implementation.
