# Panel navigation, details, command history and smooth scrolling

Date: 2026-11-05. Windows and Linux verification is complete.

## Scope

The implementation adds persistent command recall, focused-entry detail fields,
readable/exact-byte size preferences, Space/Insert advancement, independent
Space-key directory scans, breadcrumb and history navigation, Hot Paths menus,
numbered tabs, Options topics and native network-location identification.
Discrete mouse-wheel input is interpolated in the file list; native pixel and
touch gestures retain their own physics. Managed build products now use `_build`.

Network device/share discovery remains a separate plugin design, not an
implemented discovery service. Dynamic viewer/filesystem loading and internal
copy/move/delete operations remain later stages.

## Regression coverage

- Exact Unicode command text, 100-entry deduplication across shells, newest-first
  order, bounded retention, successful-launch recording, clear/disabled persistence,
  schema 1–4 migration and edits racing an initial settings load. Recall does not
  execute or restore elevation.
- Accepted/rejected selection, parent/last rows, pending filtered views, directory
  links, concurrent targeted scans, sorting/refresh identity, full-scan supersession
  and a scan finishing while its refresh is blocked or before UI acknowledgement.
- Exact `u64` bytes, partial directory totals, native attributes/Unix permissions,
  stale field-copy gestures, backend clipboard errors, cancellation and ownership.
- Unicode/raw-path breadcrumbs, failed-navigation restoration, history positions,
  stale choices, containing-volume selection, network generation checks, tabs,
  AltGr/editor scope and Options topic keyboard navigation.
- Real `MainWindow`/`FilePane` scroll offsets during presenter refreshes: fractional
  frames, accumulated/reversed wheel input, final row alignment, same-pane pointer
  movement, pane changes, popups, focus and explicit cursor reveals.
- Options interaction, compact geometry, font sizes, native/fallback icons,
  tooltip behavior and selected-row contrast across all three themes.
- Both density profiles: icons enlarged by 10% without taller rows, contiguous
  Unicode breadcrumbs, centered history chevrons and canonical drive captions
  below Home/special locations. The caption regression first reproduced the
  incorrect `Home` text against the earlier generated UI, then passed after the fix.

## Review and environment notes

Independent review identified and prompted fixes for a target-size cache lost
during refresh, wheel impulses discarded by small pointer movements, and Options
arrow keys using the previously focused topic index. A separate presenter review
identified selection of a hidden cursor while filtering was pending.
The real-panel scroll regression also exposed repeated restarts of the final
settling animation when virtualization refined content height. Stable targets
now retain their animation and commit the exact row position after it finishes.

The three Options checkboxes share one hinted native component. With the same
feature set, this reduced generated Windows Rust from 10,577,471 to 10,271,027 bytes
(306,444 bytes, 2.90%). This is generated-source size, not executable size or a
runtime performance measurement.

WSL fixtures for native inotify and POSIX filesystem behavior must live on a
native Linux filesystem. The same native-watch executable failed to receive its
expected hint using the Windows-mounted repository's temporary directory and
passed on `/tmp/fm-panel-refinement/_build/tmp`. This is a test-fixture exception;
compiler caches and QA logs remain in the repository's `_build` tree. Normal
Linux checkouts can use the provided helper's `_build/tmp` directly.

Cargo now writes directly into `_build`, with shared WSL outputs in `_build/linux`.
The root `target` and `.tools/linux-target` compatibility junctions were removed
without following their targets. Wrapper smoke checks covered arguments, working
directory, exit status and temporary-directory scoping. Toolchain/download caches
and historical evidence are documented exceptions in the README. No generated
build products or executables belong in the commit.

Earlier concurrent builds failed during severe disk pressure (under 1GB free), before
running the whole test suite. Linux reported a compiler I/O failure; Windows reported
[LNK1318](https://learn.microsoft.com/en-us/cpp/error-messages/tool-errors/linker-tools-error-lnk1318)
while writing a 337MB PDB. After initially removing obsolete outputs, the user
explicitly requested deletion of the entire build tree and a full rebuild.
The verified cleanup removed 185,361 files and 8,826 directories containing
199,686,469,109 logical bytes; the drive reported 190,427,918,336 free bytes afterward.
Source files, installed toolchains and dependency download caches were preserved.
Both platforms then started fresh compilation with one Cargo job per platform.
The shorter `_build/tmp` also exposed a long-path test's
fixed-component-count assumption; its fixture now grows until its actual UTF-16
length exceeds 260 before exercising the unchanged native operation.

Logs, executable manifests and screenshots use `_build/qa/panel-refinement/`.

## Clean-build results

Windows `cargo build --workspace --offline --locked -j1` completed from the empty
build tree in 12m57s. The full `cargo test --workspace --offline --locked -j1`
run passed 517 tests with no failures. Three ignored entries are subprocess
fixtures explicitly invoked by their parent argument/console tests, rather than
unexecuted feature coverage. This includes 208 UI unit tests and the complete
UI integration suite.

Windows strict `cargo clippy --workspace --all-targets --offline --locked -j1
-- -D warnings` and `cargo fmt --all --check` passed. The source-size check
passed for 214 handwritten files, all at most 100,000 bytes.

After the separator adjustment, all 231 `fm-ui` tests passed again. The final
icon/navigation refinements passed a normal debug workspace build and this
regression command with 403 passed, zero failures and two subprocess fixtures
ignored in the top-level listing (both exercised by their parent tests):

```text
cargo test --workspace --offline --locked -j1 --lib --test panel_refinements --test native_controls --test compact_controls --test tooltips
```

Strict workspace/all-targets Clippy, formatting and the source-size check passed
again after those changes. The normal debug build completed after the user
authorized closing the earlier running FM instance that locked its executable.
The final executable is `_build/debug/fm-app.exe`, SHA-256
`AB9387B20C6161E9F978D9EC43E54C1BCBF5B45D985D1D92AABF5EC297AF5913`.

Both final native Windows smoke runs exited 0 and left preferences unchanged.
The actual FemtoVG-WGPU screenshot displays Czech, CJK and emoji filenames,
both panels, native icons, breadcrumbs, tabs below drives and the one-line `>`
prompt. A second native capture at the actual Windows Home directory confirms
the closed drive picker still reads `C:`. Production-UI software captures of all three Options topics confirmed
readable labels and aligned controls; these are layout checks, not a substitute
for native Unicode rendering verification.

Visual inspection found the detail strip's one-pixel separator centered through
its text. Explicit top-edge coordinates fix it; final native captures verify the
separator above the text. Before/after image measurements show identical row
boundaries while the parent-folder artwork grows from 27 to 30 physical pixels.
Breadcrumb components meet without added gaps, and the Back/Forward split
controls align their chevrons. A separate review of these changes found no
remaining actionable issues.

## Linux verification

The initial Linux clean build completed while the last visual refinements were
being added. Its generated Slint output preceded the icon change, but the later
compiled geometry test already expected the larger icon. That assertion failed
against the old generated output; the other executed tests passed. Running the
remaining already-built integration binaries completed baseline coverage:
488 passed and one assertion failed against that earlier UI generation.
After freezing the sources and regenerating the UI, the final command below
passed 379 tests with zero failures or ignored cases. This includes 206 UI unit
tests and all four affected integration binaries; the icon-size assertion now
passes against the updated UI.

```text
cargo test --workspace --offline --locked --lib --test panel_refinements --test native_controls --test compact_controls --test tooltips
cargo test --workspace --offline --locked --doc
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
cargo build -p fm-app --offline --locked
bash scripts/check-source-size.sh
_build/linux/debug/fm-app --smoke-test --left /mnt/d/experiments/dev/FM --right /mnt/d/experiments/dev/FM/crates
```

Doc-test processing passed for all seven library crates (zero doctest cases).
Strict all-targets Clippy passed without warnings, and the normal debug app build
passed. Formatting and the 214-file source-size check passed. The headless smoke
reported 15 volumes and loaded both requested panels (20 and eight entries).
Linux checks use native backend tests and software UI integration tests; the
native graphical screenshots described above were captured on Windows.

This WSL/DrvFS build also regenerated some cached dependencies. Build-script
output timestamps had lost fractional seconds: an unchanged executable timestamp
ending in `.1911656` appeared newer than its output rounded to the same whole
second. This is consistent with Cargo treating the output as stale. Regeneration
was allowed to finish normally; cache timestamps and fingerprints were not patched.
These environment observations concern build behavior, not file-manager runtime
performance.
