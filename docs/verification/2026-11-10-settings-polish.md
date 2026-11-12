# Settings and panel controls qualification

Status: the described regression checks passed and the Windows debug application
was built, deployed and startup-tested. This record does not mark the broader
implementation goal complete.

## Behavior

- General replaces Panel and contains Panels and Command line sections. The
  separate Command page is removed. Stable internal topic identities keep
  asynchronous picker and validation replies on their intended page.
- Reset stays on the left; OK and Cancel stay on the right. Real save errors
  occupy their own row. Theme/history/argument changes remain drafts until OK;
  Cancel and the close button discard the draft.
- Colors keeps full expressions in its priority model and uses compact heading
  actions. Editors and custom commands share an ordered argument list with
  selection, one value editor, and compact add/remove/reorder actions. Literal
  Unicode, spaces, quotes and empty arguments survive edits and reordering.
- Archives labels its custom extension section and centers the add icon.
  Thumbnail settings explicitly identify the longest side in logical pixels.
- Plugins separates Viewers and Filesystems, expands viewer priority rows,
  puts reorder actions in the heading and Refresh below the catalog. Discovery
  no longer presents a Chromium package as an extra invalid native HTML plugin.
  Filesystem rows describe installed manifests; selecting one does not load it,
  enable unsupported configuration or dispatch filesystem operations.
- Copy, Move and Delete use the action as the default. Transfer dialogs retain
  initial destination editing; Enter proceeds. Explicit Cancel or close-button
  focus, Escape and clicking the close button cancel. Opening a confirmation
  never starts an operation.
- Hidden/System native attributes dim both file and folder artwork, including
  badges, in Detailed, Brief and Thumbnails modes. Filename, selection and error
  presentation remain opaque. Windows attribute policy stays in a separate file.
- Detailed rows and headings share the same left edge and scrollbar reservation.
  Localized modified date and time occupy separate aligned columns; both headers
  sort the original timestamp. They are never derived by splitting display text.
- Ctrl+A and Ctrl+Shift+A select all top-level files and folders, excluding the
  parent and expanded descendants. Ctrl+I inverts files; Ctrl+Shift+I inverts
  top-level files and folders. Existing nested marks survive inversion.
- Drive menus expand to fit their labels and paths within the popup surface;
  their toolbar trigger remains compact.

## Evidence and remaining checks

Logs and screenshots are under `_build/qa/settings-polish/`. Initial failures
were retained rather than overwritten: stale shortcut/navigation expectations,
the old argument-editor assumptions, a repeated Slint platform initialization
in the new tree fixture, and the Size/header alignment regression. The overlay
pixel test was corrected to account for Slint software rendering's inherited
per-primitive alpha; exact solid-image blending and unchanged text pixels remain
checked.

Independent review found a further P1: the ancestor Return capture consumed Enter
on the header's focused X before its own cancel handler. The actual-focus test
failed before correction (`focused-close-red.log`) and passed afterward for both
ordinary and permanent deletion. Return confirmation is now scoped to the input,
option checkbox and primary button, preserving explicit cancel controls.

The refreshed argument, Colors, footer, archive-extension and plugin screenshots
were visually reviewed. Tests cover actual pointer/keyboard paths, compact and
large-font layouts, pending validation, refused queues, explicit cancellation,
and native/archive inline-tree operation boundaries.

Final results:

- Windows libraries: core **51**, engine **670** (seven documented opt-ins
  ignored), UI **747** passed (`windows-libs-green.log`).
- The latest runs of all **30** selected UI integration targets pass **127**
  tests in total. `integration-results.json` maps each target to its final log;
  it does not sum repeated runs. The final operation-specific rerun includes
  the focused-close regression and native/archive Copy/Move workflows.
- Linux libraries: core **51**, engine **661** passed, with seven documented
  opt-ins ignored (`../plugin-options/full-backend-linux.log`).
- Actual installed Windows viewer and filesystem catalogs: **one test each**
  passed as the installation's desktop owner. The sandbox account correctly
  cannot treat that owner's private plugin directory as its own trusted runtime.
- Windows junction activation: platform **11**, desktop integration **10**,
  presenter activation **14** passed, including the actual compatibility junction.
- Final scoped strict Clippy for core, engine and UI libraries passed; complete
  workspace formatting and whitespace checks passed. Source-size checks passed
  for **1,682** handwritten files, each at most 100,000 bytes.

## Exact Windows artifact

The debug executable at `D:\experiments\dev\FM\_build\debug\me.exe` and the private
runtime at `C:\Users\tjelinek\MidnightExplorer-20261110-html\me.exe` were read back
with the same SHA-256:

`E71ED919E0B0EAE14DEC5B3062213F9DA6978F287206AC849CB20FA0BCAC6E11`

The matching runtime registration was refreshed using the existing private ACL
checks. Both actual build-path startup checks exited zero with empty stderr:

- Backend smoke: three volumes, 30 left entries and 132 right entries.
- Native Windows Winit/software UI smoke: project/crates panels loaded, actual
  toolbar focus routing passed, and `native-startup.png` was captured and visually
  inspected. Date/time/header alignment, hidden-folder artwork, both panels and
  the complete footer were visible. The smoke mode uses in-memory preferences
  and does not overwrite the user's settings or tab history.

`build-hash.json`, `smoke-result.json` and `ui-smoke-result.json` identify the
tested artifact. This is a debug build, not a newly published release. No new
dependency or license choice was made.

The yellow clickable folder-size warning report is a separate unfinished slice.
Standalone bounded report storage is tested; scanner, result-lifetime and UI
integration are not claimed here. This batch does not claim complete workspace
or physical-display qualification on Linux/macOS.
