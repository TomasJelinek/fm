# Navigation and appearance verification

Scope: independent per-pane tabs with folder-based captions; durable named and
numbered native bookmarks; font family and density sizes; compact controls and
partial-size warnings; native drive icons; pixel scrolling and row settling;
safe command insertion and implemented Salamander shortcut aliases.

The specification is [navigation and appearance](../superpowers/specs/2026-11-04-navigation-and-appearance-design.md).
Copy parallelism remains [design only](../superpowers/specs/2026-11-04-copy-concurrency-design.md).

Meaningful regressions cover raw Unicode/native identities, long paths, settings
v1/v2 migration and malformed encodings, tab history/sort/cursor independence,
stale results and IDs, queue failures, last-tab protection and32-tab overflow.
The installed bridge exercises actual IDs, active-pane focus, persistence and
quoted Ctrl+Enter/Ctrl+P insertion without terminal/file activation.

Startup settings tests preserve untouched loaded values while local fields are
edited, including explicit same-index choices. New bookmarks and saved paths
merge without ID reuse or lost slots. A temporary128-entry reconciliation never
emits an invalid save. Read errors block writes. App shutdown tests prove that a
late initial read can produce the merged save within the shared2500ms grace,
including saturated command/event queues; timeout keeps large data owned for
backend retirement.

Actual keyboard/pointer tests cover Options interaction and large base sizes,
virtualized16K font lists, drive selectors, tabs and bookmark menus. They caught
a Slint popup lifetime bug: closing destroyed the delegate's weak parent before
the callback lookup. A root-owned function now captures the identity before
closing for mouse, keyboard and accessibility activation. A legacy focus/mouse
fixture also omitted the new tab-strip height; corrected hit coordinates
exercise the intended scrolled row without changing scrolling behavior.

Physical-digit tests cover US/Czech shifted text, RightAlt/AltGr rejection,
repeat/release suppression and editor/focus isolation. Scroll tests cover native
wheel interpolation, held touches/inertia, subpixel input, low frame rates,
stable model refresh, cancellation on cursor reveal, bounds and virtualization.
Per-drive tests cover distinct native cache roots, event ordering, Windows Shell
flags/root validation/class fallbacks and nonempty bounded rendered pixels,
plus Linux host-theme network/local selection.

Independent read-only review found and prompted the explicit-field startup and
shutdown fixes, and traced the popup bug through generated Slint code and its
pinned runtime. No further confirmed P1/P2 findings remained in that review.

Final workspace and native-rendering results are recorded after completion in
`.superpowers/sdd/navigation-appearance` and `.tools/navigation-appearance-*`.
Earlier scoped evidence includes `navigation-*`, `font-backend-*`, `commander-*`
and `target/verification/tabs_bookmarks-session61466.log` /
`native_shortcuts-session37459.log`. Source hashes make later edits detectable.

Windows x64: **338 tests passed**, zero failures; two subprocess-only fixtures
are directly ignored and are exercised by their passing parent tests. Workspace
formatting, strict all-targets Clippy, the application build, engine smoke and
the135-file/100000-byte source guard passed. The later cosmetic version addition
was included in the fresh build and native title check.

Native Windows FemtoVG-WGPU smoke exited successfully. The inspected
[screenshot](../screenshots/navigation-appearance-windows.png) shows native drive
icons, folder-caption tabs, compact controls, multilingual entries, visible cwd
and no pane status bars. The current executable was launched as the actual
desktop account with title **FM 0.1 — File Manager**; its window was visible and
responding. Exact PID/title/account evidence is in `native-window.json`.

Later compact-Touch request:40px file rows/controls/bottom buttons,48px main
bands,10px outer padding,6px gaps and20px icons, preserving existing font choices
and Desktop geometry. Seven affected Windows UI binaries passed, followed by
the existing scroll-settle test using40px rows. Evidence:
`.tools/compact-touch-targeted.log` and `.tools/compact-touch-panel-scroll.log`.
Fresh strict Windows workspace Clippy also passed after this change.
The fresh compact-Touch application build and135-file source guard passed;
native Windows smoke passed again and the updated executable was launched as
the desktop user with the verified FM 0.1 title. The captured current Desktop
profile is [here](../screenshots/compact-touch-windows.png); Touch-specific
geometry is exercised by the eight interaction/render/scroll tests above.

Linux baseline: **316 workspace tests passed**, zero failures, plus strict
all-targets Clippy, application build,135-file size guard and engine smoke.
The final Touch-specific rerun is recorded under `.tools/compact-touch-linux-*`.
All eight affected Linux UI binaries passed after the final density change,
followed by strict workspace/all-targets Clippy, the current application build,
source-size guard and engine smoke (exit0). Final source hashes remained
unchanged throughout this last verification. Windows and Linux builds are
complete; native rendering was inspected on Windows, while Linux UI interaction
tests used the headless software-window harness.
