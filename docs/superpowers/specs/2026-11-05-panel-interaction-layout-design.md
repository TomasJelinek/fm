# Panel interaction, layout and responsiveness

Date: 2026-11-05. Implemented continuation of FM 0.1; the viewer extension below
remains a design for the plugin milestone.

## Behavior

- Ctrl+Tab focuses the command editor from a file panel or path editor. There it returns
  to the active file panel. Numbered tab shortcuts remain available; Ctrl+Shift+Tab
  retains previous-tab navigation. Ctrl+U swaps the complete visual panels,
  including their tabs, histories, selection and active work. Stable backend pane
  identities stay attached to their content. Side-specific shortcuts address the
  new visual left/right sides; the active content keeps focus.
- Right-click a breadcrumb to copy its cumulative highlighted path through the
  backend clipboard service. Copy does not navigate. Held gestures carry identity
  and generation guards, just like left-click navigation.
- Each panel starts with a left-aligned action toolbar. The drive chooser and
  drive/location buttons occupy the next row, then tabs, path and file headings.
  The closed drive chooser fits its icon, short caption and arrow, bounded for
  long mount labels. The expanded menu still has room for labels and free space.
  Its `. As Other Panel` action opens the opposite panel's current folder, using
  the raw native path through normal backend navigation. The unmodified period
  key activates it only while the menu is open; mouse activation behaves the same.
  It remains a menu action, not a drive selection or a toolbar location.
- Enter in the file list opens the focused folder or invokes the host association
  for the focused file/executable. Multiselection does not execute every item.
- Remove the visible Type column and parent-row Type detail. Existing extension
  sorting remains available through its shortcut.
- A draggable central splitter sets independent panel widths. Keep usable minimum
  widths and preserve the ratio during window resize. Double-click centers it.
  Narrow panels adapt the size column; complete byte values remain in status details.
  The ratio and visual swap are session UI state, with no filesystem navigation.
- Tabs share one width: the existing preferred width until available space is
  exhausted, then all shrink uniformly to a readable minimum. At that minimum,
  show left/right scroll buttons beside the new-tab button. Reveal the active tab
  after activation, addition and resize; retain raw identities and Unicode names.
- The terminal control becomes a split button. Its main icon opens a new terminal
  in the active directory with the selected shell; it never submits the command
  draft. Its side arrow selects the shell. Each choice and the main button show a
  distinct shell icon. Enter/Run still explicitly submits the draft. Preserve the
  existing shell-grammar and elevation rules; do not recall elevated state.
- Windows status details show decoded attribute names, including compression,
  without hexadecimal masks. Decode known native flags; mark unknown flags as
  unknown without inventing semantics. Unix permissions retain symbolic text and
  chmod octal numbers. Each displayed field remains copyable.
- A nonempty multiselection reports separate selected file/folder counts. Folder
  links count as folders, other entries as files; the parent row is never counted.
- Ctrl+click toggles one item. Shift+click replaces selection with an inclusive
  anchor-to-click range; Ctrl+Shift+click adds that range. Plain click establishes
  the anchor. Rapid clicks, sorting, filtering, refresh and queue refusal must
  preserve identity and never apply selection to a replacement entry.
- Make the Light theme neutral gray, including selection/active surfaces, borders
  and accents, while retaining readable contrast and semantic error/search colors.

## Responsiveness and boundaries

Measure selection input-to-visible-feedback and resize/layout/render cost before
and after changes, using identical debug builds, data, viewports and repeat counts.
Record median/tail timings and workload size; do not describe debug measurements
as release performance. Retain a reproducible benchmark and meaningful regression
checks rather than machine-specific timing assertions in ordinary tests.

Selection feedback should be visible on the first UI update after an accepted
gesture, including while backend replies are delayed. A bounded presentation
overlay may predict pending selection; authoritative bitsets, range processing,
counts over large listings and I/O remain on backend workers. Reconcile by request
revision and navigation identity, including refusals and stale acknowledgements.
Do not copy or scan the complete directory on the UI thread. Resize must keep
virtualization and avoid unrelated model replacement or full-list formatting.

Approach: extend the current presenter/Slint components and bounded protocol.
Replacing the widget toolkit or moving filesystem work into the UI would discard
existing behavior and violate the established architecture. Splitter, focus and
visual panel placement are UI responsibilities; launching terminals and copying
text remain backend commands. Use existing shared controls instead of duplicated
per-panel markup. All new diagnostics/build artifacts stay under `_build`.

Profiling separates geometry from rasterization and native render submission.
The debug profile optimizes Slint/graphics/text dependencies at level 2 and the
generated application UI at level 1; backend/application debugging remains at
ordinary debug settings. Release keeps full optimization. Measurements and their
limits are recorded in the [verification report](../../verification/panel-interaction-and-layout.md).

## Viewer design extension

F3 chooses the applicable registered viewer using file extension and provider
capabilities. Alt+F3 explicitly requests the basic plain-text viewer, bypassing
syntax highlighting, Markdown rendering and extension-based viewer routing.
"ASCII viewer" describes the simple text mode, not an ASCII-only encoding limit:
Unicode and explicit encoding fallback remain required. Missing basic-viewer
plugins produce an unavailable action; never silently launch another viewer.
This step updates the viewer design only; viewer implementation remains a later
plugin milestone.

## Validation

Test real UI gestures, shortcuts, focus, tab geometry, splitter drag/reset and
swapped side-specific commands. Test selection under delayed/refused/out-of-order
backend responses and large views, counts, native attribute decoding, clipboard
identity and empty-command terminal launch. Compare selection/resize benchmarks
and inspect native screenshots at both densities. Update the main design,
keyboard reference, README and verification record before delivery.

Behavioral reference: local Salamander `src/drivelst.cpp:2084` adds the other-panel
entry with a period shortcut in its Change Drive menu. FM preserves native paths
without copying Salamander's fixed-size path buffers.
