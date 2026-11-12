# Bounded requirements-gap audit, 11 November 2026

Baseline: `bc5c112` (clean checkout before this audit). This is a read-only
comparison of current source and tests with `FEATURE_STATUS.md`,
`IMPLEMENTATION_BACKLOG.md`, and the linked accepted designs/plans. No Cargo,
application execution, native operation, or benchmark was run. “Proven” below means
the cited source establishes the implementation boundary; it does not mean a new
runtime test passed. Viewer scrolling is owned by the separate investigation.

This is an initial, bounded audit of explicit open requirements, not an exhaustive
security review or a completion claim for the implement-all goal. No new data-loss
defect was established. Existing unsupported-action guards must remain until their
replacement preserves the captured identity and operation contracts.

## Confirmed implementation work

Priority follows integrity prerequisites, then functional usefulness and measured
performance. P1 does not imply that current unsupported operations are unsafe.

| Priority / requirement | Status and current evidence | Concrete remaining acceptance |
| --- | --- | --- |
| P1 — COPY-03: metadata and links | **Incomplete, proven.** `crates/midnight-core/src/transfer_metadata.rs:9` has only ContentOnly and RequireModifiedTime; lines 12 and 34 limit the latter to at most 64 native regular-file roots. `midnight-engine/src/copy_metadata_worker.rs:23` rejects directories, virtual sources and duplicate native identities. `copy_worker.rs:853` rejects links/special entries. Authority: backlog line 85 and main design lines 348–365. | Add separately declared preservation capabilities for the remaining accepted metadata and directory postorder metadata, and explicit link-object copying without following referents. Test unsupported capability preflight, exact metadata readback before publication, required-field failure with sources retained, and alias/link races. Preserve existing content-only and same-volume rename behavior; do not imply that copying already preserves full ACL/xattr/stream metadata. |
| P1 — TREE-01: ordinary actions after tree adoption | **Incomplete, proven.** `midnight-ui/src/presenter/tree.rs:160` rejects every call while `tree_view()` exists, including root rows. Callers include `operations.rs:47` (rename/delete/metadata, except create), `editor.rs:100`, `checksums.rs:127`, `comparison.rs:23`, and `file_actions.rs:106` (entry context menu). `presenter/tests/tree.rs:575` explicitly verifies refusal; line 953 permits root F3 while still refusing F4. Authority: backlog line 92. | Add typed, occurrence-preserving consumers for each remaining action rather than remove the guard. Cover selected descendants, root files, mixed selections, stale publications, changed paths, canceled queues, and native/archive capability differences. Copy/Move/clipboard/sizes/root-file viewers already have typed paths and are not missing. |
| P1 — DIALOG-02: all forms outside the main window | **Incomplete, proven.** The accepted `2026-11-10-native-dialog-windows.md` spec, lines 7–34 and inventory at 65, requires every listed form to be a movable/resizable native top-level. `midnight-ui/ui/main.slint:1149` and 1221 embed operation and Copy/Move drafts in DialogFrame; lines 1203, 1241, 1276, 1292, 1320, 1338 and 1357 host Jobs, conflicts, metadata, comparison, filter and Options forms as PopupWindow. `dialog-frame.slint:36` clamps size/position to the owner. | Complete the existing migration plan's remaining typed hosts, including Jobs, Hot Paths and color picker. Preserve exact draft/job IDs, native X/Escape semantics, owner modality/taskbar rules and late callback cancellation. Verify every inventory form moves outside its owner and across monitors, retains drafts through resize/theme changes, and remains usable at 320×200 in both densities. Help/About/Notices and independent tools already use native hosts. |
| P1 — MEDIA-01: cancel expensive archive thumbnail staging | **Incomplete, proven.** `midnight-thumbnail-host/src/archive_source.rs:96` deliberately drains an admitted read using a fresh Cancellation at line 110 because canceling the shared exchange kills the browsing mount. A small requested range can require complete ZIP-member/solid-block staging. `service_archive_tests.rs:113` proves two canceled reads retain both real worker slots. Dated evidence: archive-thumbnails verification lines 173–194. | Add cooperative per-operation cancellation or another bounded ownership design that keeps the browsing mount usable. Verify cancellation during large/solid staging stops new work promptly, frees temporary storage and real worker capacity only after native calls finish, rejects late pixels, and lets subsequent thumbnails progress. Do not fake responsiveness by dropping permits while I/O still runs. |
| P1 — NET-01: interactive Linux authentication and discovery Hot Paths | **Incomplete, proven.** `plugins/midnight-filesystem-network/src/linux/gio.rs:385` aborts password challenges; line 567 returns AuthenticationRequired with instructions to connect through the desktop. `linux/gio/tests.rs:161` tests that deliberate abort. `midnight-ui/src/presenter/bookmarks.rs:17` rejects discovery locations, though typed tabs and native-share bookmarks work. Authority: backlog line 93. | Implement host-owned ephemeral authentication/cancellation with no credentials in saved addresses/logs, and typed discovery bookmarks independent of captions. Test canceled/stale auth, reconnect, provider removal, persisted opaque addresses and shortcut restore. Qualify against an owned real SMB/GVfs endpoint, separately from fixture success. |
| P1 — TERM-01: reusable terminal/sudo session | **Incomplete, proven.** `midnight-core/src/desktop.rs:423` exposes launch-only TerminalService; `midnight-platform-linux/src/desktop.rs:32` creates a command and spawns it at line 52 for each request. The accepted terminal-session design lines 7–22 explicitly requests reuse of one unprivileged broker/TTY. | Implement the specified identity-checked, bounded broker/session lifecycle and qualify a concrete Linux terminal. Distinguish Idle/Busy/disconnected/unknown outcome; no automatic duplicate execution, password retention, sudo-policy changes or indefinite authentication refresh. |
| P1 — PLAT-01: macOS application support | **Incomplete, proven; native qualification unverified.** `midnight-platform/src/lib.rs:29` rejects all non-Windows/Linux targets. This is not absence of all Darwin work: `plugins/midnight-filesystem-native-macos/src/adapter.rs:17` implements the read-only provider, and its IMPLEMENTATION_PLAN lines 31–44 records portable/cross-check evidence with native execution pending. | Supply the separate application platform services, trusted loading/containment, UI/input/clipboard/terminal/window integration, mutations, packaging and CI. Reuse the existing provider instead of restarting it. Native macOS hardware/runner evidence is needed to claim supported versions/architectures; its absence does not prevent source implementation. |
| P1 — PLAT-03: native Wayland external file drag/drop | **Incomplete, proven.** `midnight-platform-linux/src/file_drag_drop.rs:58` accepts only NativeDragWindow::X11 and returns Unsupported for native Wayland at line 64. Its line 326 deliberately selects Copy. Authority: backlog line 122. | Add the accepted same-connection seat/data-device bridge with raw identities, bounded admission and correct cancel/lifetime handling. Test on native compositors. Preserve explicit unsupported status until qualified; internal dragging and XWayland evidence do not prove native Wayland transport. |
| P2 — IO-01: SSD-only parallel verified copy and shared scheduling | **Incomplete, proven.** `midnight-engine/src/copy_worker.rs:1` and line 71 retain one current EntryWork/VerifiedFileCopy. `midnight-platform-common/src/transfer.rs:516` acquires all shared I/O slots because transfer capabilities lack verified storage classification. The 10 November SSD probe lines 172–208 records an incomplete gate and no-go, not evidence that concurrency has no benefit. | First finish the agreed 1/2/4-stream benchmark under stable conditions, including the missing small-file/eight-file confirmation. Then implement positive local-SSD endpoint classification, shared physical-device budgets and bounded different-file concurrency. Keep unknown/HDD/network serial. Measure full verification throughput, UI p95/p99, cancel/error behavior, aliases and A→B/B→A fairness before changing defaults. |
| P2 — TREE-01 / ARCH-02: container attachment and archive watches/nesting | **Incomplete, proven within the inspected path.** `midnight-engine/src/providers/tree.rs:274` defines attach_container, but the current engine/UI production search finds no caller. `tree_watch_scope.rs:40` selects loaded native branches and line 110 requires ProviderTarget::Native. `midnight-filesystem-host/src/session.rs:73` takes a captured file with an absolute native container locator (line 87). Authority: backlog lines 92 and 180. | Connect explicit container discovery/attachment and provider-content leases with parent anchors, bounded aggregate nesting, cancellation and outer-source invalidation. Extend refresh/watch contracts to archives. Keep archive-in-archive as its separately identified later capability; do not claim ordinary archive browsing/extraction is absent. |

Paths abbreviated in the table after their first occurrence are relative to
`crates/`. Requirements retain their existing contracts; no new implementation
permission or general design restart is needed.

## Qualification work not established by this audit

These are **unverified**, not evidence that the implementation is broken:

- **Actual integrated archive Hot Paths:** baseline `FEATURE_STATUS.md:161` and
  backlog line 134 still require a real-plugin menu/shortcut, application restart,
  unavailable-provider and changed-source workflow. Source/presenter persistence
  tests are narrower than the full installed workflow.
- **Final Slint application → Explorer drag:** `FEATURE_STATUS.md:35` and
  `verification/file-drag-drop.md:165` explicitly distinguish successful owned
  native/Shell probes from a final application-to-running-Explorer gesture.
- **Live SMB/FUSE and native Linux desktop:** network verification's Limits section
  explicitly excludes live SMB interoperability; `SYSTEM_REQUIREMENTS.md:15–19`
  retains Mint/Fedora/openSUSE/macOS qualification, separate from WSL checks.
  Archive thumbnail verification lines 193 onward retains Linux execution,
  physical panel interaction and large/all-format cache qualification.
- **Adverse transfer environments:** `developer/OPERATIONS_REAL_QA.md:81–100`
  explains that disk-full and partial-write errors are injected, not a real filled
  volume or physical disconnect, and that that particular target does not prove
  actual cross-volume Move. Other Shell tests do cover cross-volume outcomes.
  Reconcile the existing evidence by pathway before commissioning only the missing
  controlled fixtures; never infer lack of all cross-volume coverage.
- **Complete current release/platform matrix:** current focused Windows gates and
  Linux compilation do not qualify every planned OS/desktop/GPU/distribution or
  every remaining feature. Old unchecked plan boxes are not proof of missing code.

## Baseline stale statements corrected or excluded from the work queue

- `FEATURE_STATUS.md:135` says Unix symbolic/octal permissions remain. Current
  `midnight-ui/src/detail_attributes/linux.rs:12` implements them, including
  special bits and tests at line 48; `presenter/panel_details.rs:152` uses them.
- Backlog line 152 says compact errors use plain text and need a close control.
  `midnight-ui/ui/file-pane.slint:688` uses ErrorNotice even when minimal, and
  `error-notice.slint:121` supplies its CloseButton. This is documentation drift,
  not a missing close implementation.
- The backlog introduction still calls clipboard consumers pending, although its
  later line 97 corrects that. Current `tests/tree_clipboard.rs:351–371` covers
  both panes, Copy/Cut and exact nested fallback; those paths must not be reopened.
- Backlog line 135 says native hosting is not hooked yet. Shared DialogHost and
  Help/About/Notice hosts are already connected; the remaining inventory, not the
  whole hosting foundation, is open.
- The earlier files-only inline expansion queue/generation bug is explicitly
  fixed in `verification/2026-11-11-tree-observations.md`, including the real
  installed Git/bin fixture. This audit found no current reproduction and does
  not reopen it.
- Guarded Find Open/Edit and archive search are absent, but the current recursive
  Find spec lines 131–158 expressly excludes execution from that milestone.
  Keep them as separate later scope, not an invented failure of the implemented
  validated Go-to-file workflow.

No production or test source was changed by this audit. This report was added, and the inspected stale statements in FEATURE_STATUS.md and IMPLEMENTATION_BACKLOG.md were corrected without claiming new runtime qualification.

## Subsequent implementation on 11 November

The table above records the original read-only baseline. Later work closes the
following bounded portions; it does not close the overall implementation goal:

- TREE-01: [F4 from an adopted tree](2026-11-11-tree-external-editor.md) captures
  the focused ordinary native file. Mutation, metadata, checksum, comparison,
  context-menu and container consumers retain their separate guards and backlog.
- NET-01: [discovery Hot Paths](2026-11-11-network-hot-paths.md) persist opaque
  addresses and configured numbered shortcuts. Interactive Linux authentication
  and live SMB/GVfs qualification remain separate.
- Transfer reliability: [stage Retry and cleanup retention](2026-11-11-transfer-stage-retry.md)
  fixes demonstrated orphaned-stage and lost-diagnostic paths without expanding
  metadata/link capabilities or changing verification/source-removal policy.

## Next scoped work from parallel source review

No implementation or runtime qualification is claimed for these two investigations:

- DIALOG-02: the next coherent slice in the accepted native-window plan is the
  shared Rename/Create/Delete confirmation body plus Copy/Move drafts. Reuse
  `DialogHost` ownership and a new native `OperationWindow`, with separate exact
  operation-ID and draft-token adapters. Validate stale close callbacks before
  cancellation; preserve child edits/caret/metadata state across refreshes and
  show queue-admission errors in the child (current Notices belong to MainWindow).
  Migrate conflicts with Jobs afterward because their Later/Cancel/reopen lifetime
  is a distinct contract. Qualify actual child Enter/Escape/X, small layouts,
  failed-show cleanup and outside-owner movement, not only the shared host.
- MEDIA-01: archive thumbnail `Member::read_at` intentionally uses an independent
  cancellation token. Ordinary archive exchange cancellation kills the shared
  worker, so merely forwarding viewport cancellation would also break browsing.
  Add a negotiated cooperative read path that aborts bounded staging exchanges,
  drains the matching cancellation acknowledgement, and discards only that
  request's spool while preserving the mount, scratch data and cached members.
  Preserve hard watchdog/invalid-message termination and legacy worker behavior.
  Avoid injecting ordinary `io::Interrupted` into archive source reads: the
  existing staging copy/discard loops retry that condition. Tests must cancel ZIP
  and solid-7z work and then prove another read succeeds on the same mount.
