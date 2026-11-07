# Modeless Find qualification

Implemented native Windows/Linux search is described in
[FIND.md](../developer/FIND.md). This report qualifies backend and Slint software
window behavior; a rebuilt installed application/native desktop pass remains
separate. The existing installed release is not claimed to include these changes.

## Focused backend and contract evidence

- The backend report `_build/goal/find/report.md` records Windows contract2,
  native4, worker6, actor4, content4, then the additional native locale regression
  (native5 total); the complete Linux Find group18 and strict Clippy passed.
- Captured raw roots, native identity/revision validation, bounded multi-session
  cancellation, literal streamed Unicode content and incomplete/error reporting
  are tested independently of the UI. Windows raw-surrogate and Linux raw-byte
  root presenter tests preserve unchanged native paths.
- A new shortcut regression first failed because a previously assigned Alt+F7
  conflicted with the Find default. The new default now yields to saved explicit
  overlapping bindings. Explicit conflicting assignments still fail; all three
  core Find tests passed after the correction.

## Window and interaction evidence

Final focused Windows command:

```text
cargo test --locked --offline -p midnight-ui --lib --test find_windows
  --test tooltips --test dialog_layout --test file_actions_ui
cargo clippy --locked --offline -p midnight-ui --all-targets -- -D warnings
```

Results: **392 library + 2 Find window + 1 shared tooltip + 2 layout + 7 file
action tests passed**. Strict all-target Clippy passed. The library group includes
two contemporaneous archive-selection provenance tests and two tooltip host tests;
it is not a count of Find-only tests.

Linux WSL2/Ubuntu24.04 focused execution passed **389 library + 2 Find window +
1 shared tooltip tests**. The following strict Clippy run caught a missing field
in the explicit preferences test initializer after the parallel checksum schema
addition. That initializer is updated; final combined UI Clippy remains pending.
These are software-rendered Slint windows, not claims about
native Wayland or every GPU/driver.

The actual window test creates two independent native-window adapters plus the
main window, streams Unicode results and exercises Start/Stop/Close. A full queue
keeps the result owner visible, Details can copy complete diagnostics, closing one
session leaves the other and the main window alive, and host shutdown cannot
recreate closed windows on a late refresh. A second test exercises all nine themes,
both densities and 860x620/320x200, including Query/Details/Copy/Close controls.

The first Details/Escape regression failed on both platforms: hiding the focused
query editor left Escape without a focused recipient. Details switching now
explicitly focuses the enclosing Find key scope. The same real-window sequence
passes on Windows and Linux. No exception suppression or global key interception
was added. Old rendered result tokens include their search generation and cannot
retarget a replacement run's same numeric result ID.

BMPs at `_build/goal/find/ui/find-860x620.bmp` and `find-320x200.bmp` were visually
reviewed: compact input/result layout, visible error badge and reachable bottom
actions. Content/details regions scroll at minimum size. Tests reuse these names
across variants; a retained BMP is the last rendered case, not a gallery of every
theme. Logs are `ui-final.log`, `ui-clippy.log`, `ui-linux.log` and
`ui-linux-clippy.log` in `_build/goal/find/`.

## Deliberate current boundaries

Native roots only; no archive search, automatic execution or editing from results.
Content searches UTF-8 with optional BOM; UTF-16/legacy encodings and binary/regex
content are explicitly unavailable. Go to file validates its retained result and
navigates only; it does not authorize a later mutation. Resource/link/device/access
exclusions remain counted and do not become silent complete results. Native app
launch, end-to-end Find, release packaging and macOS qualification remain pending.
