# Current panel, viewer and dialog follow-ups

Accepted requests, 7 November 2026. This extension supersedes conflicting earlier
designs. It records requirements, not a claim that the current release implements
them. [Feature status](../../FEATURE_STATUS.md) and the
[implementation backlog](../../IMPLEMENTATION_BACKLOG.md) distinguish existing
behavior, working changes and outstanding native/release qualification.

## Archive Hot Paths

Add current path must work inside an archive, including its root and any browsed
subdirectory. The displayed form follows Salamander's requested separator, for
example `D:\packages\source.zip@src\viewer` on Windows and
`/home/user/source.zip@src/viewer` on Linux. Use the platform's normal separators,
without added spaces; the archive root ends at `@`.

Persistence retains a typed location: native archive container path, provider
identity and internal member path. The visible string is not an operation
identity and is never split blindly at `@`; a real filename may contain that
character. Reuse the existing lossless native-path codec and archive-location
representation. Internal archive names retain their original provider identity.
Adding the same location again retains one bookmark and its assigned shortcut.

Both Hot Paths menus and the drive dropdown expose archive bookmarks with the
same 1–9/0 shortcuts as native folders. The star beside View adds the current
location; its arrow opens the menu. Saved paths have no separate drive-toolbar
buttons. Manage remains open on selection and edits shortcut assignment without
redundant custom folder names.

Opening a saved location mounts the container and navigates to its saved member
through the backend. Loading, cancellation, missing container/member, removed or
disabled provider, and denied access must preserve the saved bookmark and report
the actual failure. A late reply cannot replace a newer panel/tab navigation.
Saving/restarting, raw Unicode/long paths, literal `@`, duplicate adds, shortcut
navigation and unavailable providers are required regression cases. Nested
archive mounts remain outside the currently implemented archive capability.

## Panel controls and status

- Ctrl+F12 has a panel toolbar filter icon as well as its configurable shortcut.
  An applied filter has a visible active state, including when the panel is
  narrow. The tooltip names the active filter/mode; clearing it removes the
  indicator. A rejected draft leaves the prior valid filter and indicator intact.
- Find offers the same name-matching choices as panel filters: Wildcards/Masks,
  Regex and Expression. Reuse the backend compiler, limits and error diagnostics;
  this does not turn the separate literal content search into regex execution.
  Results use one compact row per entry, like a file panel, with aligned metadata
  and the full native identity retained independently of display elision.
- Ctrl+F1 should show useful detailed information for the displayed filesystem:
  actual mount/volume, label, filesystem, capacity/free/used and available storage
  properties. Query native capabilities on backend workers. Omit unsupported
  fields and distinguish unavailable data; never guess SSD/network classification.
  If useful information cannot be supplied, remove the action rather than retain
  an uninformative placeholder. This action remains under implementation review.
- Prioritize the filename/selection summary in the bottom status line. Selected
  folder sizes appear when known; omit an uncomputed total and the distracting
  `unknown`/`N unknown` messages. Partial totals stay visibly approximate and do
  not become exact totals or zero. Do not start a scan merely to populate status.
  Keep selected file/folder counts, locale byte formatting and free space.
- Long upper paths retain the beginning and end with a literal three-dot `...`
  marker. Every shortened path label uses the same visible marker, including
  Find progress/results, comparison headings, checksum paths and saved-folder
  menus. Reserve space for all three dots when the label is narrow. Complete
  paths stay available to accessibility, navigation, right-click copy and
  editing; a display label never becomes an operation identity. The command
  line continues to show its full path through scrolling. Pasting appends the
  native trailing separator without lossy conversion or automatic navigation.
- Icons fill the unchanged file-row height with minimal padding. Compressed
  archive indicators remain small and unobtrusive. Discrete wheel input animates
  through pixel offsets and snaps only after the coast; gesture scrolling stays
  continuous. Returning to visited thumbnails reuses retained/cache content.

## Ordinary Move

The user explicitly removed **Move with recovery**. F6 and Cut/Paste must present
ordinary Move, inspired by Salamander, with no recovery opt-in or source-recovery
journal/workflow. For each file, copy, finish the required metadata/publication,
independently verify destination length and SHA-256, revalidate the exact source,
then remove that source immediately before progressing to the next file.

Do not wait for all files in a batch before removing an already completed source.
If a file fails verification or removal, retain that unremoved source and report
the failure; earlier completed moves remain completed. Cancellation is not a
whole-batch rollback. No deletion of `..`, changed identities or unrelated files
is authorized. A same-filesystem native move can be designed separately with
truthful guarantees. This correction does not silently change the separately
documented displaced-destination policy for an explicit Copy overwrite.

The old source-recovery implementation is being replaced; historical recovery
test reports describe that earlier implementation only. Current platform limits
and the replacement proof belong in [Transfers](../../developer/TRANSFERS.md).

## Viewers

- Text/source remain read-only, render the visible region with bounded prefetch
  and cache, and must not load/parse a whole file before scrolling. Measure native
  input-to-visible-frame latency; an internal microbenchmark is insufficient.
  Images reuse decoding and bounded fit/zoom caches. Ctrl+F must focus working
  search in the real viewer. Escape always closes the viewer, including search
  focus; initial bounds cover the main application's window.
- Source mode has a line-number toggle, **on by default**, and status for the
  current logical line and column. Position follows the reading/caret position
  and remains coherent across wrapping, search and paging; document Unicode
  column semantics. Line numbers must not require full-file loading up front.
- Markdown has **GitHub** (default), **Compact** and **Comfortable** rendering
  presets, plus readable font/size controls. Layout options change presentation,
  not document bytes or search/copy semantics. Preserve inert links and existing
  plugin security/resource limits. Do not imply a web renderer or network access.
- Plugins remain independent Cargo projects with their own tests and only public
  API dependencies. Host-only presentation changes do not grant plugins access
  to application internals.

## Dialogs and settings

[Dialog ergonomics](../../developer/DIALOG_ERGONOMICS.md) governs the current
redesign. Every dialog has an accessible top-right X equivalent to Cancel and a
real resizable window that can move outside the main window and between monitors.
Embedded popup movement alone is insufficient. Closing must preserve each
operation's actual cancel semantics, invalidate pending picker/draft callbacks
and never apply changes implicitly. Native OS chooser windows retain OS controls.

Create-folder and similar single-line inputs keep ordinary control height.
Attributes/Times/Case use appropriate icons and compact controls without the
obsolete preview pane; necessary backend validation/confirmation is retained.
Options Save validates and saves the current draft, shows failures and permits a
real retry. Color settings have a visual picker. File-valued settings have native
Browse actions. External editor arguments use rows, not JSON. Custom Commands
follow Salamander's User Menu list-and-form layout, retain drag reordering, and
use argument insertion helpers and native pickers. Copying a filename/path into
the command line adds neither quotes nor separator spaces.

## Completion evidence

Update affected feature/API/hotkey documents with implementation changes. Keep
historical verification reports dated; label superseded behavior instead of
rewriting old measurements as new evidence. A source change, focused test and
deployed release are different states. Native-window movement, viewer latency,
archive bookmark restoration and real Options Save require workflow tests as
well as unit tests before being marked complete.

## Additional appearance requests

Add several further themes informed by publicly popular theme projects, plus
separate Norton Commander and DOS Commander inspired retro palettes. Current
choices for implementation are Tokyo Night, One Dark and Rosé Pine. Record the
upstream references and any applicable asset attribution; repository popularity
is a selection signal, not proof of a universal favorite ranking. Maintain clear
normal/focused/selected states, folder/file distinction and readable error text.
Theme IDs already saved by users must remain stable. The 9 November revision
replaces DOS Commander with Amber Terminal using a storage migration, and adds
Verdant Light, Catppuccin Latte and Rosé Pine Dawn; see the
[updated theme design](2026-11-06-theme-expansion-design.md#retro-replacement-and-light-palettes-9-november-2026).

The user supplied a zipped-folder image as a visual reference and asked whether
a zipper is clearer than the current arrow archive overlay. Evaluate a small
original vector zipper overlay without an opaque badge, preserving system icons
and the unchanged row height. This is a presentation change only: it must not
change archive recognition, associations, provider availability or file actions.

## Follow-ups accepted 8 November

- Inline error notices, including panel archive/navigation failures, need a
  visible cross button. Dismissing a notice hides presentation only; it does not
  turn an unavailable provider into a native path, dismiss validation failures
  internally or authorize an operation. New error occurrences must be visible.
- Iterate on debug/test builds by default. Reserve release/fat LTO for final
  packaging/integration or a specific measured performance comparison, not each
  UI adjustment. The existing build helpers already default to debug.
- Options → Archives must let users add/remove custom filename extensions for
  each supported archive format/decoder, alongside built-in optional association
  checkboxes. Persist mappings, validate duplicates/conflicts and preserve disabled
  document-container defaults. Association choices do not enable unavailable
  decoders, bypass plugin trust, or prove that file contents form a valid archive.
- Viewer presentation preferences survive closing a viewer and restarting the
  application: text wrap, source line numbers, search case sensitivity, separate
  text/source/Markdown fonts and sizes, Markdown preset and image/PDF zoom mode.
  Store validated field changes through the existing backend settings service;
  concurrent viewers must not overwrite each other's unrelated preferences.
  Missing plugins do not erase their saved preferences. Search text, document
  contents and per-file reading positions are not part of these settings.
- Both panel lists and viewer surfaces should move smoothly in pixels and coast
  after scrolling. Reuse decoded/cached content during motion instead of waiting
  for a new full-surface backend render for every input event. Image panning is a
  priority performance defect: measure input-to-visible-motion separately from
  decode time, keep memory bounded and reject stale asynchronous output.
- Ctrl+F also opens Find from the file panel, retaining Alt+F7. In a viewer it
  opens search in the current document. Both bindings stay user configurable;
  an existing explicit assignment takes precedence over a new default.
- Find shows the currently scanned directory while running, updated at most four
  times per second independently of slow native directory/content reads. Results
  use clickable sorting headers with ascending/descending order and preserve the
  selected result by identity. Locale name/path/extension sorting and numeric
  size/time sorting run on backend workers; discovery IDs retain their original
  validation meaning. Both file panels retain the Extension sort menu option and
  Ctrl+F4 alias in every supported view mode. Each detailed panel also exposes a
  compact **Ext** sort header beside **Name**, with the current direction arrow
  and an explanatory tooltip. It activates its own panel and toggles ascending /
  descending through the existing backend sort command. Name and Ext share the
  filename header area; rows retain full filenames and aligned size/time fields.
- Dragging a viewer scrollbar must move its thumb immediately, retaining the
  original grab offset and stable geometry while held. Old renderer replies may
  not move it backwards during the gesture or after release. Consecutive absolute
  positions are coalesced before the bounded worker queue, preserving file/mode,
  search and other input ordering. Retain and retry the final position if the
  queue refuses it. Only the corresponding successfully rendered viewport may
  acknowledge that request. File/mode/viewport changes cancel old gestures and
  retries. Measure native pointer-to-render latency separately from final content
  readiness; a fast thumb alone does not establish fast document scrolling.
  See the [scrolling contract](../../developer/VIEWER_SCROLLING.md) for message
  ordering, cancellation, bounded pending input and measurement details.
