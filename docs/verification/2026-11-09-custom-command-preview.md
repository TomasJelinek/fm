# Custom Commands: Preview remains in Options

The settings Preview path used the toolbar's automatic Jobs-opening signal. It
could leave Options behind another popup. Settings previews now carry their draft,
edit and preview-attempt identities through selection capture and backend replies.
Their result stays in a scrollable read-only area within Options. Only previews
requested outside settings use automatic Jobs presentation.

Program/arguments/folder and icon pickers remain available. Update command,
Discard changes and Preview are grouped together; Update command no longer
occupies the working-folder input row. The page reveals a newly received preview.
Preview does not save settings or launch a process; the existing OK/Cancel
transaction remains in force. The bounded formatter preserves separate native
argument values, Unicode and explicit truncation markers. Expansion and native
validation remain backend work. No dependency was added.

The Salamander reference remains `src/lang/lang.rc`, `IDD_CFGPAGE_USERMENU`,
lines 246–274, and the sources recorded in the
[earlier User Menu report](2026-11-07-user-menu-refactor.md). No source or artwork
was copied.

## Reproductions and review

Evidence is under `_build/qa/custom-command-preview/`.

- `red.log`: the original settings result requested automatic Jobs presentation.
- `review-red-and-network.log`: independent review reproduced an older failure
  overriding a newer success for the same draft, and a deferred selection failure
  opening an application notice. Preview-attempt identity and origin-aware
  selection-error routing correct both paths. Obsolete captured selections retire
  without submitting another preview.
- `green-ui.log`: 603 UI library tests, four Network tests, one drive geometry
  test, 13 command-settings tests and two command-toolbar tests passed. The new
  pointer test initially failed because its fake backend had never completed the
  initial panel listing. The production guard correctly rejected that request.
- `preview-pointer-ready.log`: after supplying the missing `ListingFinished`
  fixture, an actual Preview click, asynchronous result, continued keyboard
  editing and Cancel passed. Jobs stayed closed and neither Execute nor Save was
  sent. The regular-size rendered result was visually inspected.
- `waiting-status-red.log` / `waiting-status-green.log`: two failing regressions
  became 19 passing settings tests. Clearing status now changes its presentation
  revision; draft replacement, matching navigation cancellation and deferred
  submission remove the waiting text. Actual `UserCommandSettingsView` assertions
  cover these transitions and preserve a newer draft's status on stale callbacks.
- `final-ui.log`: **631 passed** (605 library, four Network, one drive geometry,
  13 settings, two toolbar and six form tests). The pointer preview test also
  verifies the exact edited name, scroll access after resizing to 720×500 and
  Cancel without Save/Execute. Both regular and compact preview screenshots were
  visually inspected. Existing form checks cover footer access down to 320×200.
- `clippy.log`: strict Clippy for `midnight-ui` and `midnight-explorer`, all targets,
  with `--no-deps -- -D warnings`, passed. The engine dependency still emits its
  13 previously recorded tree-action unused-code warnings; this is not a clean
  whole-workspace lint claim. Owned Rust formatting and the scoped whitespace
  check passed; all 11 affected source files are below 100 KB.
- `build.log`: the combined debug application build passed. `me.exe` is
  86,073,856 bytes, SHA-256
  `69A4A2C5A2E81DC93B82C74866594F836770E8DBEDACCDD974CDD8A490C2DFD1`.
  `source-hashes.json` records the 11 scoped source hashes. The build also contains
  the shared Network ordering and metadata-only archive folder-size sorting.
- The isolated Windows candidate is
  `C:/Users/tjelinek/MidnightExplorer-20261109-navigation-candidate/me.exe`.
  Its default-renderer `--ui-smoke-test` rendered both panels and exited 0 with
  empty stderr. The screenshot was visually inspected without a startup error.
  Real/legacy preference hashes and the previous frozen performance candidate
  remained unchanged. Deployment and smoke evidence are under
  `_build/qa/navigation-candidate/`; no shortcut was changed.

## Remaining qualification

Native installed-window Preview acceptance remains separate from the tested
software-renderer interaction and native startup smoke. The wider Custom Commands
and session Jobs backlog is not declared complete by this change.
