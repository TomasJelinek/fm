# Literal folder-path paste

Keyboard and context-menu Paste in the top panel path editor now use one backend
text-read operation. Only the pasted payload gains a native trailing separator;
ordinary typing, existing suffixes, spaces, quotes and Unicode stay literal.
Enter remains the only path submission action. See [behavior and integration](../developer/PATH_PASTE.md).

The implementation has causal regressions for separator/refusal policy, delayed
menu-focus notifications incorrectly invalidating an unchanged editor, eager
Linux clipboard initialization without a display, private-use Unicode rejected
by synthetic key input, and timer reentrancy while Presenter is borrowed.
Insertion uses the pinned native committed-text edit path, retaining selection,
IME and native Undo/Redo semantics. No panic catch, dependency patch or changed
generic text editor is involved.

Windows and Linux each pass all9 actual widget tests: keyboard/context Paste,
selection replacement/suffix, Undo/Redo, Enter-only navigation, stale edits,
selection/cancellation/navigation/supersession, queue/backend refusal, empty paste,
Shift+Insert, private-use/supplementary Unicode, TAB, old IME preedit, both panes,
320×200 layout and conservative timer retry. Presenter tests cover single-use
replies and host retirement. Both platforms pass8 focused engine clipboard tests;
Windows strict UTF-16 decoder1 and Linux strict UTF-8/lazy/transport tests pass.
Core/native/engine all-target strict Clippy passes on both platforms.

The final combined UI runs also include independently owned parent-navigation and
thumbnail changes. Their failures are recorded separately; the Windows UI/app
strict step was skipped after those failures. No fresh whole-workspace or native
compositor qualification is claimed here. Pinned Slint uses the same data-control
or X11/XWayland clipboard capability class as the new Linux backend route.

Artifacts: `_build/goal/path-paste/report.md`, `windows-ui-final9-retry.log`,
`backend-windows.log`, `backend-linux-final.log`, `linux-lazy-green.log`, plus
`_build/goal/parent-focus-linux-final3.log`. Tests use in-memory preferences and
clipboard fixtures; no actual user clipboard, preferences or release executable
is changed. Root owns final integration qualification and deployment.
