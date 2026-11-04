# Compact controls, tooltips and Options verification

The main window replaces permanent keyboard hints and command placeholder text
with plain-text tooltips. The shell is a compact icon selector left of the
command input. Its menu, tooltip and accessibility value identify the current
shell/mode; selecting never launches a command or bypasses draft grammar checks.
The active folder remains visible in a bounded read-only prompt, separate from
the submitted command text, with the full path available in a tooltip.

Drive choices show native roots/letters, Windows volume names when available and
free space; their closed controls remain compact. Windows native root/label and
quota-aware capacity APIs use dynamically sized UTF-16 buffers and preserve
missing-path failures. Linux has no invented Windows volume labels. Two backend
metadata workers refresh cached data about every 45s without taking listing
capacity. Missing/free-space errors never become a false zero.

Desktop uses 22px rows, 24px controls, 14px file icons, 6px padding and 4px gaps.
Touch retains 44px rows/controls and descriptive location captions. File/folder
icons are centered in their row. Small drive captions and icon-only special
locations retain complete plain-text tooltips and accessible names.

Options explicitly uses `close-on-click-outside`. Its previous Slint default
closed after any inside click. The actual pointer regression reproduced that
failure, then verified heading clicks, Dark→Touch→Light selections, continued
interaction, Escape, reopening and Done. No direct callback invocation stands
in for the popup interaction.

Other regressions cover actual shell/drive pointer and keyboard operation,
authoritative selection after host rejection, disabled-button tooltip display,
raw native roots with identical visible strings, stale metadata, duplicate roots,
blocked metadata isolation, coalesced discovery and cache expiration.

Native QA caught a missing font glyph for the dropdown arrow and an icon-layout
overflow. The picker now uses a centered vector chevron and one shared caption/
icon slot. Full UI input tests were repeated after those changes.

The volume TTL test additionally exposed Tokio's upward millisecond deadline
rounding. A timer barrier replaced the brittle exact-clock advance; production
code was unchanged. The corrected boundary test passed 50 isolated repetitions.

Fresh read-only review found no remaining confirmed P1/P2 issues. Salamander
behavioral references were inspected at `src/toolbar6.cpp:241` and
`src/drivelst.cpp:2561`, `:2621`; no implementation/assets were copied.

Evidence: `.superpowers/sdd/compact-tooltips`, with final workspace logs under
`.tools/compact-desktop-windows-*` and `.tools/compact-desktop-linux-*`.
Final Windows x64 workspace verification: **240 tests passed**, zero failures;
two existing subprocess-only fixtures remain directly ignored and are exercised
by their passing parent tests. Strict Clippy, workspace formatting, application
build, engine smoke and the 99-file/100000-byte source guard passed.

The native FemtoVG-WGPU UI smoke exited successfully. The inspected
[desktop screenshot](../screenshots/compact-desktop-windows.png) shows centered
icons, compact location controls, visible cwd prompt, no permanent help footer,
and multilingual filenames. The updated window was opened as the desktop user,
PID 1972, title `FM — File Manager`, visible and responding at verification.

Linux workspace verification: **222 tests passed**, zero failures or ignored
tests. Strict Clippy, application build, the 99-file source-size guard and engine
smoke passed. A fresh post-fix volume integration run additionally passed all
five tests, including the timer-boundary regression. No macOS implementation or
test run is claimed.
