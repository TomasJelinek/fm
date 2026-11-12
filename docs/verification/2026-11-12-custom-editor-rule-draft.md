# Custom editor rule creation

The reported plus-button workflow created a valid system-editor rule immediately,
so its selected draft hid the custom executable and argument controls. A probe
invoking the real public `EditorOptionsView.action(0)` callback against the existing
UI library failed at `Add editor rule must expose custom executable and argument
fields`. It used the software platform and performed no native launch. Evidence:
`_build/qa/find-actions-20261112/editor-behavior-red.log` and `editor_probe.rs`.

Add now creates an incomplete custom-editor draft with the ordinary `{file}`
argument. The executable picker and arguments appear immediately, the priority
list labels an unfinished custom rule, and an inline instruction explains how to
complete it. Selecting the system editor remains an explicit checkbox choice.
The accepted `EditorRule` schema/default and all existing saved rules remain
unchanged. Empty executables fail draft validation before any preference command;
existing picker native-path preservation, Options OK/Cancel and discard behavior
are retained.

All four editor draft unit tests pass, including new custom creation, incomplete
save rejection and explicit system-editor choice, alongside existing argument,
reorder, discard and fallback behavior. Evidence:
`_build/qa/find-actions-20261112/editor-unit-green.log`.
The coherent Linux editor filter passed all 5 matching tests, including the
existing restore-default workflow. All 3 actual `editor_options` window tests
passed. The new regression verifies the visible executable and argument controls,
rejects an incomplete save, retains two existing system rules and the custom
fallback, then commits the completed rule through the existing Options OK
transaction. The first version of the test incorrectly expected SavePreferences
before Options OK; the corrected test preserves the staged settings contract.

Evidence: `_build/qa/find-actions-20261112/linux-final/ui-editor.log` and
`ui-windows.log`. Visual inspection of
`render/editor-new-custom-incomplete.bmp` confirms the executable picker is shown
immediately and the system-editor checkbox is unchecked for the new draft.
