# Compact Custom Commands settings verification

The user requested the Salamander User Menu arrangement. Reference inspection:
`D:/experiments/salamander-main/src/dialogs4.cpp`, `CCfgPageUserMenu` near
lines 2080–2290 (`EnableButtons`, `LoadControls`, `StoreControls`, list header
creation). The implemented settings page keeps the ordered list, header actions
and selected details structure while retaining this application's structured argv
and Options transaction model.

The list now has one control-height row per item, small +/−/↑/↓ header actions,
keyboard selection and existing identity-bound drag/reorder. Summaries remain in
accessible labels/tooltips. Direct program, terminal script and separator are
explicit choices. Required program arguments remain individual editable rows.
Working directory remains visible for executable modes. Optional toolbar label/
icon fields and terminal declaration checkboxes are collapsible.

Separator conversion preserves stable IDs/order, stores the core inert variant,
and refuses settings preview before command dispatch. Existing draft program/
script fields survive mode changes. Native icon resources use the dedicated
picker kind and retain the existing draft/configuration/revision guards; ordinary
image browsing remains available. The form stores the returned native PNG path,
not a UI display caption or executable icon resource reference.

Added tests cover compact header/row geometry, Options cancel after separator
conversion, retained program arguments including empty/quoted values, explicit
preview refusal, separator reorder, and stale native-icon replies. The initial
isolated layout RED attempt could not link a stale UI rlib against rebuilt core
metadata (`command-layout-red.log`); it is not behavior evidence. The completed coherent runtime gates are recorded below.

The actual Slint build script generated all current bindings successfully into an
isolated output directory, including the root toolbar separator integration.
Evidence: `_build/qa/find-actions-20261112/slint-check/log.txt`. This checks source
syntax and generation; the runtime gate below separately checks behavior.

A standalone Cargo bindings prebuild was canceled after identifying its narrower
Slint feature set (the real UI enables unstable-winit-030). It is not a completed
build gate. Final UI validation must use the actual midnight-ui target feature set.

## Managed artwork follow-up

The user required independent 128 × 128 PNG copies for every newly selected icon.
Both typed native picker kinds now import artwork; the generic Image picker is
unchanged. The editable icon path was removed and replaced by resource/image
choices and Clear. The production Form rejected new text paths only after a
causal isolated regression first demonstrated the bypass: `icon-form-red.log`
contains `A typed icon path must not bypass managed PNG import`; the same
production-source harness then passed (`icon-form-green.log`). Existing unchanged
legacy paths remain until artwork is reselected; there is no background migration
job. New choices must match the native picker's guarded
managed PNG return value. The UI adds no file reads.

A new window regression exercises Browse image's typed request, absence of an
editable icon path, returned native result and immediate Update availability.
Native picker completion now validates the draft rather than waiting for an
unrelated edit. The final runtime gates include this regression.
## Final Linux UI results

The coherent library filters passed all 53 Custom Commands tests and all 7 picker
tests. Actual software-window settings tests passed 17/17, and the command toolbar
passed 3/3. This includes managed artwork request/return, immediate Update,
separator conversion, retained literal argument drafts, preview refusal, reorder,
Options save/cancel, and clipping-aware drag cancellation.

Two fixture corrections were required. A larger thumbnail fixture now increases
both dimensions from the managed 128 × 128 frame, preserving the cache's existing
no-shrink assertion. The clipped-footer drag fixture now derives its release point
from the actual list/ancestor intersection after rows became compact. It still
requires a visible initial press and an off-body footer release, and additionally
asserts that every staged row ID remains in the original order. Neither correction
changes production drag or thumbnail behavior.

Evidence under `_build/qa/find-actions-20261112/linux-final`:
`ui-commands-cache-red.log`, `ui-commands.log`, `ui-editor-picker.log`,
`ui-windows.log` (the prior footer fixture failure), and
`ui-command-windows-retry.log` (all 20 window tests pass).

Visual inspection of `render/command-settings-compact-priority.bmp` confirms six
single-line rows and small header actions above the detail form. The footer remains
fixed while longer command details scroll. `render/command-settings-separator.bmp`
shows the inert separator form and disabled Preview. These are software-rendered
Linux window checks, not native GPU or installed icon-worker qualification.

The final app artwork filter passed all 5 Linux tests, covering managed-copy
routing, cancellation, decode/storage failure without original-path fallback,
resource import and presentation retries. The smoke shutdown timing regression
also passed. Strict Clippy completed successfully for all targets of the 11
affected packages with `--no-deps -- -D warnings`; evidence is `app-artwork.log`,
`app-smoke-timing.log` and `clippy.log` in the same final gate directory. These
routing tests do not claim an installed Linux image-worker qualification.
