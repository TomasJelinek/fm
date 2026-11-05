# Panel navigation, command history and compact details

Date: 2026-11-05. Status: implemented and verified on Windows and Linux; see the
[verification record](../../verification/panel-navigation-and-smooth-scrolling.md).
This extends the existing commander interface.

Behavioral references were checked in local Salamander sources:
`src/fileswn0.cpp:1057` (Space selects, optionally measures, then advances),
`:1119`/`:1376` (Insert and ending search), and `src/fileswn1.cpp:2191` (Hot Paths).
These guide behavior only; FM keeps its own Unicode and asynchronous implementation.
The requested numbered tab shortcuts take precedence over Salamander assignments.

## Requested behavior

- Keep managed build/test outputs together under `_build/`, with Cargo writing
  directly into that directory and no compatibility junctions. Shared Windows/WSL
  checkouts put Linux outputs in `_build/linux/`. The user requested a complete
  clean rebuild, superseding the initial cache-preservation approach. Retain
  installed toolchains/download caches; document those and OS temporary storage
  separately from project products.
- Print the command directory on one line followed by `>`. A compact dropdown
  recalls executed commands into the editor without executing them.
- Persist the latest **100 distinct command strings**, most recent first.
  Re-execution moves a command to the front. Match duplicates by exact text,
  even across shells; preserve the latest shell grammar, never elevated mode.
  Record only a successful terminal launch, through the common submission path.
  Provide clear history and persistence controls. User confirmed persistence.
- Insert and Space toggle the focused real entry and advance one row, clamped
  at the end. Space also schedules that directory's size on the backend. It
  never follows a directory link or cancels an existing scan. During active
  type-to-search, Space continues entering query text as before.
- Make selected rows more visible in every theme, retaining a distinct focused
  cursor, active pane and partially computed size indication.
- Add a persisted Panel option for readable sizes (default) versus exact bytes.
  Sorting continues using numeric values, never their formatted text.
- Add one compact details row below each file panel. Show the focused filename,
  exact byte size, date/time, native attributes and POSIX permissions when known.
  Unknown values remain unknown; partial directory totals remain visibly partial.
  Each field is hoverable and clicking copies its full displayed value through
  the backend clipboard service. Filename elision does not truncate copied text.
- Group Options into Panel, Appearance and Command topics. Retain theme/font,
  Desktop/Touch density, search filtering and existing save/error behavior.
- Put folder tabs below the drive toolbar. Ctrl+Shift+1…9 selects that numbered
  tab in the focused pane; missing tabs are a no-op. This supersedes numbered
  bookmark assignment on those keys. Assign slots through bookmark management;
  Ctrl+Shift+0 may retain slot 10. Existing bookmark jump shortcuts remain.
- Add dropdowns beside Back and Forward for the tab's corresponding history.
  Select a specific history entry by identity, including repeated locations.
  Choosing an item does not duplicate or reorder the browser history. Each arrow
  and its centered dropdown chevron form one compact, aligned split button.
- Add a Hot Paths icon/dropdown to each drive toolbar, and include every saved
  path in the full drive/location dropdown. Use saved bookmark identities.
- Vertically center icons. Drive and file/folder icons are 10% larger using reduced
  inner spacing; existing row and toolbar heights stay unchanged. Selecting a
  special location updates the dropdown to its containing volume/mount. Its
  visible caption and tooltip use the same canonical choice as its selection,
  so a folder under Home cannot rename the containing drive to Home.
- Keep the displayed top path synchronized with navigation and failed-navigation
  restoration. A focused edit is a separate draft, not the authoritative path.
  Hovering a breadcrumb highlights its cumulative prefix from the root; clicking
  navigates to that raw ancestor. Render components without artificial gaps,
  preserving actual separators and spaces inside folder names. Ctrl+G retains
  full-path editing.
- Identify native network paths asynchronously and show Network with the native
  network icon in the location dropdown. Device/share discovery belongs to the
  separate future [Network plugin](2026-11-05-network-filesystem-design.md).
- Animate discrete wheel input in short fractional-pixel steps in the file list;
  preserve native pixel/phased touchpad and touch input. When motion and input
  stop, gently settle on a complete row. Clicks, keys, popups and navigation stop
  residual wheel motion; other controls retain their usual scrolling.

## Architecture and invariants

Use the existing engine/presenter/Slint boundaries. Filesystem queries, targeted
size scans, preferences persistence, terminal launch and clipboard ownership stay
on bounded backend workers. UI renders bounded models and sends intent. No new
plugin implementation is included in this milestone.

Preferences move from schema 4 to 5 with strict migration/defaults. Command
history has a 100-item and 1MiB aggregate-text budget; it must not truncate command
strings or log them. Late preference loads cannot replace commands executed or
options edited since startup. Picking history restores text and compatible shell
grammar, never runs it and never restores sudo/elevation. Disabling persistence
removes saved history while current-session history can remain available.

Navigation, breadcrumb and details callbacks carry generation/identity tokens.
An open menu or held mouse gesture cannot navigate/copy a replacement entry
after a tab switch, refresh or cursor change. Paths retain raw native identity;
display text is never parsed back to recover a clicked breadcrumb/bookmark.

Single-folder scans have independent scan IDs and frozen raw entry identity,
share the global storage budget, and survive sorting/same-location refresh.
Changing location/tab invalidates them. A running all-folder scan already covers
Space; do not enqueue duplicate work. Updates remain at most four per second;
completion does not automatically re-sort the panel.
Starting a new all-folder scan explicitly supersedes older targeted scans and
their caches, so a late final result cannot overwrite the new scan's result.

Status metadata comes from the existing no-follow metadata query. Windows
attributes and Unix mode live in separate native implementations and common
optional typed fields. Formatting is presentation only. Clipboard work follows
native lifetime/affinity requirements; a failed write is reported rather than
pretending success. Large selections and file contents are not copied by a
status-field click.

Slint components are shared: topic options body, breadcrumb control and detail
strip, with the existing reusable compact picker. Do not duplicate per-pane
markup or construct separate Text widgets per grapheme/token inside a filename.
Each breadcrumb component retains its full Unicode name and cumulative prefix.
Keep source files below 100,000 bytes and rustfmt's 100-column target.

## Acceptance

Tests cover selection advancement at first/last/parent rows, command backpressure,
Space scan identity and concurrency, stale callbacks, history unique ordering and
schema migration, recalled shell grammar without launch/elevation, exact u64 byte
formatting, field-copy Unicode/raw identity, missing metadata, special-directory
volume selection, breadcrumb roots/Unicode and failed navigation, tab shortcut
precedence/AltGr/editor focus, options tabs, centered icons and all themes.

Run full Windows and Linux tests, strict Clippy, formatting and source-size checks
after integration, starting with the explicitly requested empty build directories.
Inspect native screenshots for compact layout, selected/cursor contrast, the
one-line prompt, details/breadcrumb controls and all Options topics. Build
consolidation must leave one common build root without junctions or a duplicate
native build tree. Update README, main design and hotkey docs.
