# Feature status for 0.2.0

This is an early preview. A design document is not evidence that a feature is available.
The [pixel-scrolling contract](developer/SCROLLING.md) distinguishes fractional
movement, native input routing and actual frame cadence. Native observations and
known performance limits are recorded in the [scrolling investigation](NATIVE_SCROLL_VERIFICATION.md);
intermediate offset tests alone do not establish 60 Hz scrolling.

The 11 November scrolling update applies fractional wheel coasting to panel and
standard-list scrollbar paths, shares input handling across tool windows, and
reuses visible-row and GPU rendering resources. Final Windows panel probes reach
roughly 60 Hz average redraw cadence, with occasional longer gaps. Text/source
viewers at high DPI and viewer idle restart still have measured delays; see the
[viewer qualification and remaining work](verification/2026-11-11-viewer-pixel-scroll.md).
The fresh debug application passed backend, software-UI and native-UI startup
checks with its protected plugin runtime. Linux compilation passed; Linux native
frame pacing has not been qualified in this update.

The current transfer update is available in the qualified Windows debug build:
whole-operation speed charts, native same-volume Move without
content copying, and Options → General full SHA-256 verification (default on).
Its optional quick mode checks size and first/middle/last 64 KiB, or all bytes up
to 192 KiB. The immutable mode also covers clipboard/drop/tree/archive requests.
See the [transfer contract](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
and [speed-chart evidence](verification/2026-11-10-operation-speed-charts.md).
Compact Windows staging names and cancellation cleanup also have native Windows/
Linux, queue and UI coverage; see the [final debug qualification](verification/2026-11-10-staging-cleanup.md).
The [folder-drop update](superpowers/specs/2026-11-10-folder-drag-and-native-handoff.md)
adds exact native folder targets, tree captures and volume-based Copy/Move defaults.
Windows native handoff now passes a real owned-window success/cancel/success test.
Actual Shell targets also pass same-volume Move/Ctrl-Copy and cross-volume
Copy/Shift-Move for files and nested directories; see the [dated qualification](verification/file-drag-drop.md#2026-11-10-folder-targets-and-windows-shell-move).
Linux outgoing transport remains Copy-only on X11/XWayland; native Wayland
external dragging is unavailable. Integrated component checks and debug deployment
are complete; an end-to-end drag from the final Slint app into a running Explorer
window still needs separate native qualification.

The 9 November follow-up adds [Make File List](MAKE_FILE_LIST.md) on Ctrl+M and
[user viewer associations](VIEWER_ASSOCIATIONS.md) in Options > Plugins. Markdown
scrolling now reuses semantic pages, fixes incorrect line/block limits and backward
table overlap, and has a proper GitHub block layout. See the dated
[file-list](verification/2026-11-09-make-file-list.md),
[associations](verification/2026-11-09-viewer-associations.md) and
[Markdown native verification](verification/2026-11-09-markdown-scroll-and-style.md).

A further Markdown correction preserves complete bounded paragraph/table context
across overlapping scroll requests and reuses parsed context for nearby positions.
GitHub selection resets its body typography to platform system-ui at 16 px;
tables use content-aware columns, intact numeric tokens, correct body striping
and stable physical-pixel borders. See the [continuity evidence](verification/2026-11-09-markdown-continuity.md)
and [font/table checks](verification/2026-11-09-markdown-font-tables.md), including
the actual report-file regressions missed by the earlier exact-anchor test.

Inline tree background checks now batch without false queue-exhaustion errors;
Network follows the drives before local special folders on both navigation surfaces.
Ctrl+A selects only top-level panel entries, clearing nested marks. See
[tree scheduling](verification/2026-11-09-tree-budget.md) and
[tree selection](verification/2026-11-09-tree-select-all.md).

Adopted inline trees now retain literal prefix search and optional quick-search
filtering over their loaded rows, composed with persistent filters and sibling
sorting. Archive extension sorting uses original UTF-8 member names and locale
keys. See the [current scope and qualification](verification/2026-11-10-tree-prefix-search.md);
remaining typed tree operation consumers stay open. Native manual refresh and
batched watch notifications now reconcile the adopted tree in-session; see the
[watch contract](developer/TREE_PANEL.md#native-watches-and-reconciliation) for
scope and bounds. Archive watches and broader tree workflows remain separate.

Native clipboard routing from adopted trees is implemented in source: Ctrl+C/Ctrl+X
capture exact selected or focused native occurrences, and Ctrl+V retains the panel
root destination and ordinary verified Copy/Move behavior. Windows presenter and
real-Engine UI bridge tests pass, as do Windows/Linux backend libraries. The UI
bridges substitute the clipboard boundary; Linux UI and native desktop clipboard
interoperability remain unqualified. Archive/mixed clipboard export stays unsupported.
See the [clipboard contract](developer/TREE_PANEL.md#native-clipboard-from-expanded-panels)
and [measured scope](verification/2026-11-10-tree-clipboard.md).

The settings polish merges Panel/Command into General, reuses ordered argument
editors and compact priority actions, and separates viewer/filesystem catalogs.
Panel metadata now aligns date and time independently; Windows Hidden/System
attributes dim file and folder artwork. Both Ctrl+A and Ctrl+Shift+A select
top-level files and folders; Ctrl+Shift+I inverts them. See the
[qualification and deployment status](verification/2026-11-10-settings-polish.md).

Native, archive and expanded-tree size scans now retain clickable yellow warning
reports. Red listing errors expose full details separately from Retry/Collapse.
Tooltips show the actual reason; clicks and Copy preserve the captured occurrence
and complete diagnostic. See the [contract](developer/PANEL_DIAGNOSTICS.md) and
[qualification](verification/2026-11-10-panel-warning-details.md).

Default filename coloring now covers eleven categories with contrast adjustment
for every theme and selection/focus state. Exact historical defaults migrate;
custom rules remain unchanged. Readable sizes use KiB/MiB/GiB/TiB for the existing
1,024-based conversion, and Reverse order is separated in the sort menu. See the
[palette and units evidence](verification/2026-11-10-filename-palette-and-units.md).

Copy and Move now deselect successfully completed roots progressively, including
verified archive extraction. Skipped, failed and unfinished roots stay selected;
folders clear only when their complete subtree succeeds. See
[transfer-selection verification](verification/2026-11-08-transfer-selection.md).

Copy/Move also has versioned Retry/Skip/Cancel I/O error decisions with safe
retained-object checkpoints. Automatic progress closes on success; manually
opened Jobs and its recent history remain. Unsupported checkpoints cannot be
retried optimistically. See [error recovery verification](verification/2026-11-08-transfer-errors.md).

Reconciled on 10 November against the dated archive, Find and inline-tree evidence below.
Native/archive inline trees also consume typed targets for directory sizes:
selected folders or all displayed folders (including expanded descendants) without
selection, exact-folder Space,
coalesced progress, known selected totals and explicit size sorting. Other tree
operation consumers remain unfinished. See the [size verification record](verification/2026-11-10-tree-folder-sizes.md)
and [expanded-folder correction](verification/2026-11-10-expanded-folder-sizes.md).
F3/Alt+F3 also retain root-file viewing while branches are expanded. Viewer capture
uses the exact root aliases and visible order, preserving native/archive source
authority. Other descendant operation consumers remain fenced; see the
[viewer integration contract](developer/TREE_PANEL.md#viewers-from-expanded-panels)
and [Windows/Linux qualification](verification/2026-11-10-tree-viewers.md).
Network Neighborhood and viewer requests retain their separately recorded scope. The
[current follow-up contract](superpowers/specs/2026-11-07-panel-viewer-followups.md)
and [backlog](IMPLEMENTATION_BACKLOG.md#latest-accepted-follow-ups) track work in
progress separately from released behavior, including archive Hot Paths qualification.

Markdown now renders pipe tables with aligned, wrapping cells and borders,
strikethrough, and read-only task markers. Its font chooser lists installed
families and retains the Markdown family/size independently of text/source
preferences. See [verification and limits](verification/2026-11-08-markdown-tables-and-fonts.md).

| Feature | Current implementation | Remaining work / design |
| --- | --- | --- |
| HTML viewer | Optional independent Chromium/CEF process plugin, captured native/archive assets, in-window rendering, find/zoom and explicit offline/online controls; default HTML/HTM/XHTML associations are configurable | Complete matching CEF package required. [Windows installed verification](verification/2026-11-10-html-viewer.md) and [Ubuntu 24.04 WSLg XWayland host/Slint qualification](verification/2026-11-10-html-linux.md) pass. Linux requires Chromium sandbox plus existing delegated aggregate resource limits; native Chromium Wayland and physical IME remain unqualified. [API and security contract](developer/WEB_VIEWER_API.md) |
| Native panels | Windows and Linux browsing, tabs/bookmarks, multiselection, locale sorting, watches, selection-aware directory sizes, three view modes, themes and configurable shortcuts | macOS adapter remains pending |
| Shared toolbar | One action row above both panels follows the active panel; includes navigation/history, Checksums, Folder Sizes, Properties, file/directory comparison, filtering and Hot Paths; View and Sort belong to each panel drive bar | Scrollable overflow preserves the 320×200 minimum; context changes and canceled input cannot retarget actions. Drives, paths and tabs remain per-panel. [Verification](verification/2026-11-08-shared-toolbar.md) |
| Inline folder branches | Production native/archive panels lazily enable occurrence-based branches, selection, disclosure and typed directory navigation; prefix/quick search, persistent filters and sibling sorting share acknowledged backend reprojection | Search examines loaded visible rows without enumeration; filtering retains unselectable ancestor context and rebases selection atomically. Native manual/watch refresh retains the adopted tree; typed operation consumers, archive watches and container attachment/nesting remain pending. [Panel contract](developer/TREE_PANEL.md), [10 November prefix scope](verification/2026-11-10-tree-prefix-search.md), [controller verification](verification/2026-11-09-tree-controller-integration.md). These integrations do not establish full TREE-01 completion. |
| Panel status | Filename or selection summary takes space before secondary metadata; Windows attributes use compact characters such as A/C/R with a tooltip legend; clicks copy full captured values | Unix symbolic/octal permissions remain. Narrow panels elide only after assigning the available width to the name. [Verification](verification/2026-11-07-status-and-viewer-readability.md) |
| Compact main window | Exact 320×200 logical minimum at every density; active pane and other-pane rail, command editor and horizontally scrollable function keys; Options/Jobs stay visible | Regular two-pane controls return when space permits. See [small-window verification](verification/small-window.md) |
| F10 main menu | 58 actions grouped by Files, Selection, Panels/navigation, Tools and Application; category picker, icons and configured key hints | Reuses existing action/confirmation paths and availability rules; [verification](verification/2026-11-08-main-menu.md) |
| Window placement | Saved physical normal bounds, maximized/fullscreen state and monitor; unavailable monitors fall back to the primary work area; viewer windows inherit parent bounds and always accept Escape | Wayland owns absolute placement. See [window geometry](verification/window-geometry.md) |
| Dialogs | Responsive, content-sized forms with shared resize/close controls; Help/About/Notices use native hosts, alongside independent Find/Checksums/File List tools | Remaining operation/settings/conflict forms still use embedded hosts. Windows ownership/taskbar behavior has a native probe; Linux desktop runtime remains unqualified. [Current ergonomics](developer/DIALOG_ERGONOMICS.md), [UI polish evidence](verification/2026-11-10-ui-dialog-polish.md) |
| Shared Windows build startup | Matching protected deployment can supply viewers, thumbnails and archive providers to the original build EXE; normal Windows build helper deploys/registers it | Raw Cargo builds need explicit registration; restart after deployment. [Runtime discovery](developer/RUNTIME_DISCOVERY.md) preserves package/ancestor trust checks |
| Keyboard settings | Search, capture, conflicts and reassignment; fixed first-character crash caused by retiring hovered tooltip rows during Slint popup handling | Causal native release reproduction and corrected four-profile matrix passed 657 assertions. [Verification](verification/2026-11-07-keyboard-search-crash.md) |
| Drag selection | Rectangle selection from empty panel space in Details/Brief/Thumbnails; Ctrl/Shift, archive identities, edge scrolling and revision-checked backend ranges | Windows software input regressions pass; final Linux/native release qualification remains. Dragging actual rows retains native file drag/drop. [Selection contract](developer/SELECTION.md) |
| Plugin catalog | Options → Plugins lists Viewers, Filesystems and Thumbnailers; asynchronous Refresh includes installed image/video helper metadata and discovery errors | Thumbnailer overview is read-only; global controls stay on Previews. [Catalog contract](developer/VIEWER_PREFERENCES.md#plugin-catalogs-in-options) |
| Media thumbnails | Independent image/video workers, private persistent cache, bounded background decoding and progressive selected-video animation; typed archive member captures now use the same services. Windows installed ZIP image/video decoding and cache reuse across reopening are verified | Requires trusted helpers; archive packages must advertise stable member IDs. Images: PNG/JPEG/GIF/WebP/BMP/ICO/TIFF. Initial video codecs are limited; no MP4/H.264 promise. Actual container storage controls network opt-in. Linux archive/native-panel qualification and generic remote content leases remain pending. A first compressed-member read may finish staging after cancellation. [Verification](verification/2026-11-10-archive-thumbnails.md), [worker SDK](developer/WORKER_SDK.md) |
| Panel search/filter | Unicode filename prefix search and optional quick-search filtering in flat and adopted tree panels; per-panel Masks/Regex/typed metadata Expression filters on Ctrl+F12 compose with the prefix and sibling sorting. Backend validation preserves the last valid view | Trees search loaded raw names; opaque archive captions cannot invent a match. One latest desired query and one in-flight options request preserve typing under queue pressure. Insert waits for cleared search/settled selection; hidden/context rows and stale carets cannot authorize keyboard actions. Options commit only on matching publications; Cancel discards and clear retains the persistent expression. [Syntax](developer/METADATA_FILTERS.md), [prefix scope and qualification](verification/2026-11-10-tree-prefix-search.md), [earlier filter evidence](developer/TREE_PANEL.md#filter-and-sort-evidence). Full installed tree workflows remain separate. |
| Filename colors | Ordered masks/regex rules, source/executable/image/video/document presets, one base foreground and optional background; automatically derived focus/selection colors, four-state preview, nine themes including Carbon Green | Matching runs on backend snapshots; invalid drafts do not replace active rules; explicitly removing all presets survives restart |
| Internal viewers and F4 | Independent basic text/binary, source highlighting, Markdown, image and PDF packages; text wrap/search/copy, default-on source line numbers and logical position, font controls and Markdown GitHub/Compact/Comfortable presets; image/PDF Fit/1:1/zoom/pan, captured-order Space navigation; configurable external editor | Requires installed, enabled viewer packages in a trusted installation. Text selection/copy is bounded to the loaded page; distant source positions index asynchronously. Images decode at most 128 MiB of pixels and present at most 4 megapixels; animated formats show the first frame. See [presentation and native controls](verification/2026-11-08-viewer-controls.md), [viewer limits](verification/2026-11-07-viewer-controls.md) and [image API](developer/IMAGE_VIEWER_API.md) |
| Copy | F5, clipboard and drag/drop share a bounded queue, full SHA-256 readback by default, explicit size plus three 64 KiB samples as a weaker option, folder merge and recoverable replacement; opt-in exact modified time for up to 64 native regular files | Provider capabilities govern replacement; strong identity-conditional overwrite is unavailable. Links/complete metadata preservation remain pending. [Transfer contract](developer/TRANSFERS.md) |
| Move | Qualified same-volume native rename moves files/whole folders without content reads or hashes. Cross-volume Move copies and verifies each file before source removal; explicit directory merge processes children individually | Both OS backend gates pass; final integrated release qualification is pending. Linux identity/removal is live-validated, not atomic compare-and-unlink. Links, virtual providers and full metadata remain unsupported. [Current contract](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md), [evidence](verification/2026-11-10-transfer-verification-and-native-move.md) |
| Find files | Alt+F7 or the magnifier opens a modeless window with multiple roots, Masks/Regex/Expression, recursion, literal UTF-8/BOM-marked UTF-16 content, whole words, modified-within minutes/hours/days, exact-content duplicate groups, five sortable headers, phased progress, Stop and validated Go to file | Native Windows/Linux folders with explicit link/mount/encoding/budget exclusions. New relative-age/duplicate/content behavior has 10 November backend regression coverage; duplicate groups stay adjacent and publish atomically. Installed application/native-desktop and new Linux runtime qualification remain separate; archives and guarded Open/Edit remain pending. [Find contract](developer/FIND.md), [earlier mode/sort evidence](verification/2026-11-08-find-refactor.md) |
| Native mutations | Rename, empty-file creation, recursive F7, confirmed Trash/permanent deletion, attributes/timestamps/case changes | Capability limits and per-item failures remain explicit |
| Directory comparison | Ctrl+F10 and the shared top toolbar compare displayed names, sizes and modification times; optional unmatched folders, case matching and time tolerance | Both panels must be native; metadata only, no recursive/content/attribute comparison |
| Checksums | Ctrl+F8 opens a modeless window with captured-selection calculation and explicit checksum-file verification; SHA-256 default and optional SHA-512/MD5/SHA-1/CRC32, streamed progress, Stop, per-hash/full-row copy and verified export to the five corresponding formats, including SFV | Partial export requires explicit consent; output choices match the captured run. Manifest paths are bounded and validated without following links; matching checksums do not authenticate sources. Windows/Linux focused checks are recorded; packaged release qualification remains separate. [Checksum contract](developer/CHECKSUMS.md), [file verification evidence](verification/2026-11-08-checksum-files.md) |
| Archive providers | Independent workers browse ZIP/7z/GZIP/TAR and TAR.GZ/TAR.XZ/TAR.BZ2/TAR.ZST plus raw compressed streams; F5 uses verified copying. Protocol 2 streams captured source ranges and verified members through private disk storage without artificial source/member/decoded-disk/ratio byte ceilings | Protocol 1 retains legacy limits. Protocol 2 retains two mounts, 512 MiB worker memory, 20-second progress watchdog, 256 KiB I/O, 100,000 entries, 8 MiB names and decoder/header bounds; available private TEMP storage can limit content. Installed Windows/Linux worker checks passed 9 November; complete application/UI qualification remains separate. No writing, passwords, nested mounts, RAR or external member launch; unavailable saved tabs remain explicit. [API and limits](developer/FILESYSTEM_API.md), [streaming verification](verification/2026-11-09-archive-streaming.md) |
| PDF | Independent isolated PDF plugin with page controls, zoom and pan; new PDFs open Fit window, including enlargement of small pages; Ctrl+PageUp/Down preserves manual zoom, while Fit adapts to each page size | Hayro rendering; unsupported/encrypted documents report an error. No scripts/network/link activation; bounded source/pages/rendering. See [opening/zoom verification](verification/2026-11-08-viewer-opening-toolbar.md) |
| File comparison | Ctrl+Shift+C or the shared top toolbar opens text differences or image comparison with Side-by-Side, Swipe and Onion Skin; two active-panel selections take priority, otherwise an opposite selection or exact same-name counterpart is used. An unresolved pair opens File A / File B with native Browse and editable paths. | Native regular files only; automatic matching uses visible native names with exact case, never an unrelated opposite cursor; bounded text diff and raster sizes. [Automatic pairing](verification/2026-11-08-comparison-same-name.md), [manual choice contract](developer/FILE_COMPARISON.md) |
| Custom Commands | Windows/Linux backend supports captured native arguments/cwd, per-item or grouped execution, previews and bounded process ownership; settings includes drag ordering, a conditional toolbar and configurable shortcuts | Windows Program/PowerShell and Linux Program/POSIX profiles; Cmd custom scripts and native macOS support remain unavailable. [Custom Commands design](superpowers/specs/2026-11-07-user-commands-design.md), [settings verification](verification/2026-11-07-custom-command-settings.md) |
| Native providers | Independent Windows/Linux native libraries provide panel Browse/Stat/Storage and native F3 content; required packages are included by build/release helpers, with no hidden fallback | Remaining services still use explicit OS adapters. Generic provider action migration, writes/watches and nested-container attachment remain pending; inline directory browsing is integrated. [Native API](developer/NATIVE_FILESYSTEM_API.md) |
| Network Neighborhood | Independent Windows WNet/MPR and Linux GIO/GVfs plugin; Network entry after drives in the dropdown and each panel drive bar, incremental discovery, cache, manual targets, explicit native-share handoff, parent/back context and persisted typed tabs | Requires OS discovery/runtime services. Controlled live SMB/FUSE interoperability, interactive Linux credentials and discovery-node Hot Paths remain unqualified/unimplemented; native share Hot Paths work. macOS is a later adapter. [Contract](developer/NETWORK_FILESYSTEM.md), [qualification](verification/2026-11-09-network-neighborhood.md) |
| Options | OK validates all pages, saves asynchronously and closes on success; Cancel/Escape/X discard the session and restore appearance previews; scrollable color-picker popup with fixed actions, native file choosers and editor argument rows | Automatic session saves exclude unaccepted previews; [transaction design](superpowers/specs/2026-11-08-options-transaction.md). Release qualification is separate. |
| Archive Hot Paths | Typed provider/container/member/decoder addresses persist losslessly, display `archive.zip@member` and deduplicate; re-adding refreshes the source revision while preserving the shortcut slot. Source/settings/presenter checks passed 8 November | Qualify the complete real-plugin menu/shortcut, restart/restore and unavailable-provider workflow before deployment. Display text, including literal `@`, never becomes a parsed locator. [Dated evidence and qualification gap](verification/2026-11-08-panel-viewer-followups.md) |
| Latest panel/viewer follow-ups | Larger same-height icons, middle path elision and longer pixel wheel coast have focused checks; Markdown/source presentation controls have actual Windows input checks; Find modes and compact results have separate debug evidence above | Filter toolbar active state, useful drive information, removal of unknown-folder status, thumbnail retention and further viewer latency work remain tracked separately in the backlog |

The [follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md) preserve
the uncompleted requests. Native copy currently runs serially with a 256 KiB
buffer. Future concurrent copying is permitted only when both source and
destination are positively classified as local SSD storage. HDD, network and
unknown storage retain serial copying. Details are in the repository's security
and architecture documents.

The [10 November same-SSD benchmark](verification/2026-11-10-ssd-copy-probe.md)
measured substantial gains in the native primitive for large/mixed files, but
repeated abnormal audit delays prevented full qualification. Its decision is
no-go for implementation from this run; it does not mark parallel copying as
available or establish that parallel copying has no benefit.

The release archives include this status file; relative design links are intended
for the repository version at https://github.com/TomasJelinek/fm.
