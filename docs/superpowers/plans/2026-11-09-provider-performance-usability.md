# Provider correctness, performance and usability follow-ups

Accepted user requests, 9 November 2026. This ledger extends the active product
goal; unfinished items remain required work. JPEG-specific decoding stays reverted.

## Approach and ownership

First enforce provider identity/capabilities in UI admission and backend execution,
then reproduce the reported workflows. Native paths, virtual members and viewer
temporary files must never be interchangeable operation targets. Measure actual
performance before/after. Archive, theme and tree slices use separate ownership.

The user explicitly resumed generic source streaming and disk-backed archive
viewing. Remove artificial archive/member byte-size ceilings; stream structure
and decoded content with available-space/write-error handling. Preserve bounded
working memory, cancellation, source identity, checksums and isolation. Never
replace a size limit by a whole-input allocation or restore the JPEG adapter.

## Requirements and acceptance

| ID | Work | Evidence required |
| --- | --- | --- |
| PFS-01 | Audit every filesystem action across native, archive and Network providers. Disable unsupported F8/Delete and other mutations. | Hotkeys, toolbars, menus, delayed callbacks, clipboard, drag/drop and stale dialogs never mutate a native/TEMP substitute for a virtual member. Backend rejects wrong provider identity. |
| PFS-02 | Ctrl+Shift+S sums archive folders/descendants, using selection when present. Show archive date/time. | ZIP/7z/TAR metadata, absent dates, nested/empty/duplicate folders, cancellation and sorting; no extraction for stored size sums. |
| PFS-03 | Browse large archives from structure; stage viewer content to private disk without fixed file-size ceilings. | Real Godot ZIP below, large/sparse/solid fixtures, integrity, low disk, cancellation and cleanup on Windows/Linux. |
| UI-01 | Per-panel filter icon immediately left of View on drive bar; applied indicator and easy clear. | Inactive-panel click activates and filters that panel only. |
| UI-02 | All path elision visibly uses three dots, including command editor path. | Beginning/end remain clear at narrow widths and varying fonts; operation/copied paths remain full. |
| THEME-01 | Evaluate themes; add higher-contrast light/dark choices and two extreme accessibility variants; theme top toolbar background. | Semantic text/selection/focus/border contrast tests, persistence and rendered panels/dialogs. |
| PERF-01 | Optimize image viewer zoom. | Measure Fit/1:1/zoom/pan; avoid repeated decode/resampling and retain correct pixels/alpha. |
| PERF-02 | Optimize panel scrollbar dragging. | Native `C:\Windows\System32`, Ctrl+F11, drag scrollbar; record latency before/after, retain pixel scrolling and row snap. |
| MEDIA-03 | Fix thumbnail flicker/reload; deeply review cache and presentation. | Repeated scrolling in Screenshots path below; stable completed images, cache reuse and no stale clearing of newer images. |
| CMP-03 | Compact convenient comparison; prominent equality/difference count. | Text/image modes, equal files, many changes, keyboard navigation and resizing. |
| PERF-03 | Reduce startup wait for restored folder contents. | Actual native startup timing; correct saved folders/theme/geometry without a default-view flash. |
| DEL-03 | Name a single file/folder in deletion dialog; fix permanent-delete error5 on exact QQQ below. | Owned ordinary/read-only/cloud/reparse fixtures first, then user-authorized application deletion of QQQ only; do not weaken identity/ACL checks or follow arbitrary junctions. |
| LOC-03 | Alphabetically sort drives/special paths by section; remove extra right button padding; put Network on drive bar. | Locale-aware ordering, compact centered icons and per-panel activation. |
| MD-02 | Restore rendered Markdown instead of plain text. | Actual F3 selects installed renderer and shows headings/tables/styles, including archive members. |
| ERR-04 | Copy displayed panel errors to clipboard. | Full details/path copied explicitly, close available in compact/full panels. |
| JOBS-02 | Retain all session jobs without persistence; add search and text export. | Completed jobs remain after many operations; active-job admission limits do not prune history; backend export using save picker. |
| FIND-02 | Freely editable search roots, filesystem picker and saved history. | Native Unicode/multiple roots, editing/choosing without starting scan, persisted history. |
| CMD-03 | Preview must not hide Options behind Jobs; simplify Custom Commands using Salamander Configuration/User Menu. | Real-window Add/Edit/Preview/Cancel/Save; Options stays reachable, preview never executes, simple program/args/cwd/icon controls and pickers. |

## Reproduction inputs and initial evidence

- `D:\experiments\godot\Godot_v4.7.2-stable_win64.exe.zip` exists and is
  86,013,866 bytes, larger than the current 64 MiB archive mount ceiling.
- `C:\Users\tjelinek\OneDrive - NVIDIA Corporation\Pictures\Screenshots`
  is the requested native thumbnail performance/flicker reproduction directory.
- `C:\Users\tjelinek\OneDrive - NVIDIA Corporation\Pictures\Backgrounds\QQQ`
  exists with Directory, Archive, ReparsePoint and Compressed flags. The actual
  tag, permissions and failing native call still require investigation.
- Ordinary archive mutation guards already exist in presenter/backend. Their
  presence does not explain or fix the reported F8/TEMP workflow.

## Completion ledger

- DEL-03 backend: Read-only permanent-delete error5 reproduced and fixed, 39
  mutation/permission/transfer tests passed; exact user-requested QQQ successfully
  deleted with the product backend. [Evidence](../../verification/2026-11-09-readonly-delete.md).
  Single-folder confirmation now includes name/date and passed its real UI
  regression. Committed and pushed as `eb86033`; packaged application checks
  remain pending.
- THEME-01: six palettes and themed toolbar implemented; core/persistence and
  all-theme UI/render gates passed. [Evidence and accessibility limits](../../verification/2026-11-09-contrast-themes.md).
- UI-01/LOC-03: per-panel filter beside View and Network on each drive bar passed
  real mouse/callback tests, including inactive and swapped panels. Backend locale
  ordering, shared Network dropdown/bar placement and compact drive geometry now
  pass Windows/Linux backend and focused UI gates, including duplicate-provider
  records and missing-plugin/stale-token cases.
  [Evidence](../../verification/2026-11-09-navigation-ordering.md).
- UI-02/ERR-04: explicit path omission markers and panel error copy controls passed
  16 targeted UI tests, including a RED→GREEN painted-dot regression and the
  compact layout. [Evidence](../../verification/2026-11-09-panel-controls-and-paths.md).
  Installed GPU acceptance remains pending.
- PFS-02 timestamps: backend calendar-only date formatting passed five locale
  tests on Windows and five on native Linux. Archive folder-size counting and
  installed timestamp workflow checks are still outstanding.
- PFS-02 folder sizes: flat archive listing UI integration passed 595 library,
  5 menu and 11 toolbar tests, including 14 provider-size regressions.
  [UI evidence and remaining scope](../../verification/2026-11-09-archive-folder-sizes-ui.md).
  Installed hotkeys, measured-folder Size sorting and typed tree selections
  remain outstanding; backend qualification is recorded separately.
- PFS-01: a new 24-case mutation test retained an exact old TEMP snapshot while
  archive/network state was active. It reproduced a missing retained-Network
  guard; after correction all 479 engine unit tests passed (one optional benchmark
  ignored). This is additional coverage, not a reproduced cause of the reported
  archive F8 behavior or a completed audit of every action.
- All other items remain open until their explicit workflow evidence is recorded.
- CMD-03: Preview stays inline in Options and survives real pointer/keyboard
  interaction, editing, Cancel and compact resizing. Draft/attempt identity scopes
  late replies and selection failures; waiting-status transitions are tested in
  the actual settings presentation. The combined UI gate passes 631 tests.
  [Evidence and remaining qualification](../../verification/2026-11-09-custom-command-preview.md).
- MD-02: actual F3 rendered headings, tables and styles for native and ZIP member
  fixtures; Escape closed both viewers and preferences stayed unchanged.
  [Evidence](../../verification/2026-11-09-markdown-runtime.md). The older installed
  binary's plain-text report is not reproduced; final deployment remains pending.
