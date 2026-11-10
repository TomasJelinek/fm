# Attributes review crash and compact chooser

The reported Review changes crash was reproduced with the real Presenter bindings
and asynchronous metadata events. A tooltip over a Case field remained visible
while the backend reply replaced editing controls with the review. Slint 1.18.1
instantiated that conditional tree while borrowing its popup list; destruction of
the tooltip owner called `WindowInner::close_popup` with a nested mutable borrow.
The test aborted with `0xc0000409`; the diagnostic is `window.rs:2252:52`,
`RefCell already borrowed`. Ordinary review without a tooltip passed, including a
native Winit/WGPU probe, so the visible-popup precondition matters.

The repair retires hover observers (including pending tooltip timers) and tooltip
popups before metadata callbacks/structural publication, without replacing or
weakening the backend's exact-revision checks. Metadata additionally closes only
popups whose parent belongs to its own marked control group, preserving the modal
dialog and unrelated popups. This covers accessible actions invoked while a field
dropdown is already open. Case dropdowns cannot open during a pending review.
The existing keyboard-search workaround now shares this helper. Back to editing
and topic selection also dismiss hints before changing the tree, including mouse,
keyboard and accessibility activation. An extended regression reproduced the same
abort on the reverse transition before these hooks were added.

Attributes use five compact tri-state controls instead of fifteen action buttons,
with explicit Unchanged/Set/Clear text, keyboard cycling/reset, capability checks
and responsive columns. The default dialog target is 640 × 440 logical pixels.
See [the design](../superpowers/specs/2026-11-08-compact-attributes.md).

Artifacts: `_build/qa/metadata-review/`. `tooltip-red.log` and
`transitions-red-trace.log` preserve causal failures. The pre-fix native probe used
in-memory previews and no native file mutations or user preferences. No dependency
changes, third-party patches or panic suppression are used.

Windows metadata backend filter: 25 tests passed. The compact-control red test
failed on the missing single checkbox; its first green run passed. The real
asynchronous review tooltip regression passed after the first boundary fix.
An open Case dropdown during pending review also reproduced the same abort on
Windows and Linux. Regressions cover unchanged attributes (Apply stays disabled),
an already-open dropdown followed by accessible Review, and a due tooltip timer
before painting. Broader transition/layout/native checks are recorded below when
complete.

Follow-up UI repairs replace text-based close buttons with one square vector-icon
control in notices, dialog headers and checksum warnings. The old stock button's
text padding could displace the glyph. The Hot Paths primary button now explicitly
starts at the left edge of its split control instead of being automatically centered
under the arrow. Dialog headers supply contextual icons; modeless tools have native
window icons. Delete/F8 remains Trash; Shift+Delete and Shift+F8 request permanent
deletion, with explicit confirmation and Cancel as the default action.

The independent review found an editing regression: closing every owned popup on
`dirty_metadata` also closed a ComboBox on its first arrow-key selection. The
Linux regression failed for that ordering, then passed on Windows and Linux after
restricting child-popup cleanup to structural transitions. Ordinary editing keeps
its menu; entering review, switching topics and closing still retire dependents.
The final static review reported no remaining findings.

Coordinated UI verification: 588 Windows UI tests passed, followed by 21 focused
Windows tests after the last header/selection corrections. Linux passed the 528 UI
unit tests, checksum suites, and a final 16-test metadata/layout/selection pass.
The six real-presenter metadata scenarios pass on both systems. Screenshots were
inspected for the compact controls, centered dismiss glyph, separate Hot Paths
targets and unobstructed Jobs header. Artifacts: `ui-consolidated.log`,
`ui-final-green.log`, `linux-consolidated.log`, `linux-final-green.log`, `render/`.
The intermediate Linux run intentionally retains the reproduced arrow-key failure.

The final native Winit/FemtoVG-WGPU metadata probe completed all 13 steps with exit
0, including a live field tooltip during asynchronous review, Back and Cancel.
The debug app and that probe built successfully. `me.exe` is 83,528,704 bytes,
SHA-256 `a76fd28ae1f6b28632317455a54e1e0f47a8bce982475602bf3403765e835056`.
Its matching private plugin runtime is registered at
`C:\Users\tjelinek\MidnightExplorer-20261108-170505-7c201158`. The workspace debug
executable passed `--ui-smoke-test` against owned folders, produced `startup.png`,
exited 0 with empty stderr and left the user's preferences hash unchanged.
The release executable was not rebuilt.

Scoped rustfmt/diff checks and the 100,000-byte source limit passed (1,286 files).
Linux Clippy passed for the UI and engine libraries with `-D warnings`, allowing
only the pre-existing `dead_code`/`unused_imports` tree-code diagnostics for that
invocation; no repository lint settings were weakened. Full workspace Clippy is
not claimed clean because those unrelated diagnostics remain.

## Shared scope and single-column follow-up

Attributes now always form one column. One shared `Include subfolders` checkbox
sits above the footer, outside the scrollable Attributes/Times/Case content. Its
position and draft binding are common to all three topics. The tooltip explicitly
describes that scope. Native metadata behavior and exact review/Apply validation
are unchanged; `CChangeAttrDialog::Transfer` in Salamander also uses one shared
`RecurseSubDirs` value.

The existing editor and real-presenter review suites pass: 4 + 6 tests. The editor
test now explicitly verifies that recursion survives topic changes and can be
cleared for the whole draft. Rendered captures cover all topics, mouse/touch,
11/32-point preferences and 320×200 minimum size. Artifacts are in
`_build/qa/metadata-scope/` (`tests.log` and `render/`).

The first layout run exposed a compact-window Escape failure: the focused Case
tab became fully clipped, so Slint discarded its keyboard input. When the dialog
shrinks below its compact-height threshold it now focuses the visible Cancel
action. The same failing test subsequently passed without weakening its Escape
assertion. `layout-red.log` and `focus-diagnostic.log` retain that evidence;
temporary focus instrumentation was removed. Rustfmt, scoped whitespace checks
and the source-size check passed (1,336 files, each at most 100,000 bytes).

The final native Windows Winit/FemtoVG-WGPU metadata probe passed all 13 steps
and exited 0 (`native-stderr.log`, `native-result.txt`). The debug application
and probe were rebuilt; the existing 39 unused inline-tree warnings remain.
The new app SHA-256 is
`DFDA43F9770E3384FBE6676E98FC80EC0A704CD9502504B3DBC0D8EA757FDC06`.
Its matching protected plugin runtime is registered at
`C:\Users\tjelinek\MidnightExplorer-20261108-215348-44c23679`.
The startup smoke exited 0 with empty stderr, produced a snapshot and preserved
the saved preferences hash. The normal `_build/debug/me.exe` window was then
reopened successfully (`smoke-result.json`, `reopened.json`).
