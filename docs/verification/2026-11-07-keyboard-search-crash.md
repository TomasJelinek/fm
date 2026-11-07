# Keyboard search: causal first-character abort fix

The first-character abort is now reproduced and fixed with a native release
regression. The missing precondition was a visible tooltip owned by a command
row: keep Search focused, hover a row until its tooltip appears, then type `x`
so filtering removes that row. Earlier typing-only checks missed this state.

The fresh Windows release aborts with `0xc0000409`; stderr identifies
`i-slint-core-1.18.1/window.rs:2252:52`, the mutable `active_popups` borrow in
`WindowInner::close_popup`. Slint materializes popup repeaters while holding an
immutable borrow of that list. Removing a command delegate unregisters its
tooltip and attempts the nested mutable borrow. A targeted inspection of the
historical dump found the same source Location; the retained historical runtime
executable matches the dump's PE/CodeView identity. This is not a fully symbolized
historical unwind, but the fresh explicit panic provides causal confirmation.

The keyboard search callback now collects tooltip popup IDs, releases the list
borrow, and retires those popups before mutably borrowing Presenter or replacing
the filtered command model. Options and other popup kinds remain open. The change
does not swallow panics, patch dependencies or change the shared tooltip system.

`tests/keyboard_options.rs` adds a regression that opens a real timed tooltip,
releases its command-row handle, types the first character and clears it. It
checks stable command identity, the surviving Options dialog and retired tooltip.
The Windows combined debug gate passed 495 tests: 468 library, 20 keyboard, 5 Copy,
one panel and one status regression.
After the parent's shared color helper extraction, current-source selected scopes
also passed: 496 Windows tests and 492 Linux tests, including all 20 keyboard tests
on each platform. These are focused UI scopes, not a fresh whole-workspace run.

The final native release probe passed 657 query/focus assertions across standard,
128-command,320×200 and Light/Arial 32/Touch configurations. Every profile performs
two alphabet/digit rounds, Unicode/empty-match queries and 100 rapid edit/clear
rounds. Standard,128-command and large-font runs additionally reproduce the
visible-tooltip precondition and verify F8 capture. Compact capture is explicitly
skipped when clipped. All four processes exit successfully. Supplementary Unicode
and IME composition are not qualified by this matrix.

Evidence and the replayable pre-fix binary are retained under
`_build/goal/keyboard-current/`, especially `causal-red/`,
`build-release-green-final.log`, `*-native-green-driver.log` and `report.md`.
The qualified probe SHA-256 is
`81FEF343FA17B3A5658A965D0E2F392AB87AD779F9A876E079EA8E3E11082667`.
Both actual user preference profiles and the reference application SHA remained
unchanged. This qualification uses in-memory preferences; deployment and final
strict gates are recorded separately by the final checkpoint.
