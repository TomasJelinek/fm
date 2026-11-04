# Selection and continuous folder-size verification

Scope: backend selection snapshots and Ctrl+A/Ctrl+D/Ctrl+I; folder-size
continuity across sorting and same-path refresh; marker-free partial totals;
inline drive/location buttons; four size presentation updates per second.
The [design](../superpowers/specs/2026-11-04-selection-and-continuous-sizes-design.md)
and [keyboard map](../KEYBOARD_SHORTCUTS.md) describe the current behavior.

## Regression coverage

Ordered per-pane selection tests cover bulk actions, rapid toggles, progressive
listings, queue refusal, navigation rollback, independent tabs, raw-path rebasing
and reused numeric entry IDs. Late destination results cannot restore a failed
destination's selection or retarget Properties. Selected-path expansion runs on
backend workers; partial/deferred selections cannot silently produce partial
Properties results. Idle actors do not occupy Tokio blocking workers, and a
saturated event queue does not prevent worker shutdown.

Actual keyboard and pointer tests cover both panes, exclusion of the parent
row, modified-key rejection and command/path/options editor isolation. A pointer
regression delivers a backend selection acknowledgment between pointer-down and
pointer-up on the next Ctrl-click. Data-only model updates retain delegates and
notify only the visible range; structural changes still invalidate old input.
Offscreen rows read current snapshots lazily when Slint materializes them.

Blocked-worker tests prove repeated sorting and same-directory refresh preserve
the original folder-size scan. Raw paths bind results across reordered entries;
new navigation, replacement scans and recycled IDs cannot accept stale totals.
Replacement scans have separate result caches. Rendering tests check partial
colors and tooltips without exclamation prefixes. A virtual-clock test floods
both panes with size updates and observes four presentation pulses in one
second, no idle repaint, and immediate handling of unrelated UI refreshes.
Another regression reverses the computed size relationship while Size sorting
is selected and proves both progress and completion preserve the displayed
snapshot without issuing a sorting command.

Compact-control tests cover inline drive overflow, native icons and tooltips,
both spacing modes and narrow windows. Existing pixel-scrolling and row-settle
tests remain part of the full workspace suite. The network-storage fairness
fixture crosses a traversal quantum and directly proves the small directory
finishes before the large root's remaining metadata work.
The live-size sorting fixture explicitly waits for both sibling completion
events before refreshing. Its isolated SSD classification permits those reads
while the third root is blocked; it does not assume root-worker or publication
order. Other controlled fixtures retain the conservative Unknown policy.

Independent read-only reviews examined selection ordering, stale-result guards,
scan lifetimes and the final Slint delegate update. Confirmed findings were
resolved; the final reviews reported no remaining P1/P2 findings.

## Final checks

Windows x64 workspace: **373 tests passed**, zero failures. Two subprocess-only
fixtures are directly ignored and exercised by their passing parent tests.
Workspace formatting, strict all-targets Clippy, application build, the
145-file/100000-byte source guard and backend smoke all passed.

Native Windows FemtoVG-WGPU smoke exited successfully. The inspected
[screenshot](../screenshots/selection-continuity-windows.png) shows the drive
shortcuts beside each drive dropdown, one navigation row per pane, native icons,
multilingual names, the active directory beside the command input and compact
bottom controls. It captures the saved Desktop profile; interaction tests also
exercise compact Touch geometry. The verified executable was launched in the
actual desktop session with the visible, responding title
**FM 0.1 — File Manager**. Native PID/title evidence is kept in
`native-window.json` outside the commit.

Linux workspace: **351 tests passed**, zero failures, followed by strict
all-targets Clippy, application build, the145-file/100000-byte source guard and
backend smoke. Linux interaction tests use the headless software-window harness;
native rendering was inspected on Windows.

Local evidence lives in ignored `.tools/selection-continuity-*` logs and
`.superpowers/sdd/selection-continuity/`; it is not part of the source commit.
Source content hashes are compared after the final checks to detect edits
during verification.

## Repository contents

The initial commit includes application source, meaningful tests, Cargo
manifests and lockfile, CI, portable build scripts, style/security/design
documents and selected documentation screenshots. Windows Cargo output is in
`target/`; this development environment's Linux output is in
`.tools/linux-target/`. Local toolchains, dependencies, logs, verification
metadata and temporary captures remain ignored. No compiled executable,
library or object belongs in the commit. A remote push was not part of that
baseline verification.
