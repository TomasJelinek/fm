# Acceptance suite

The acceptance suite exercises Midnight Explorer end to end, close to real use: the
real `MainWindow`, `Presenter` and `Engine` on the native filesystem, driven by
synthetic keyboard and pointer events on software-rendered windows, against a
generated fixture tree. It complements the unit and integration tests; it never runs in
`cargo test`, `cargo test --workspace`, CI or `./build.sh --validate` (CI only lints it,
on Windows and Linux).

## Running it

```text
./build.sh --acceptance                 # build everything, then run the suite
bash scripts/acceptance.sh [debug|release] [--dry-run] [--list] [--scale smoke|full]
     [--only NAME[,NAME...]] [--strict-review] [--keep-fixture] [--all-themes]
     [--stress] [--run-dir DIR]
```

`./build.sh --acceptance` runs every normal build step and then
`scripts/acceptance.sh`, like `--validate` runs `scripts/validate.sh`. Unlike
validation it also works with the Docker build backend (`FM_BUILD_BACKEND=docker`, or
`auto` on a host without the GUI development packages): the suite renders with Slint's
software renderer, so it needs no display, and the container runs the whole chain,
acceptance included. `--dry-run` prints the build plan followed by the suite's Cargo
command. `--validate --acceptance` runs both; the first failure decides the status.

`scripts/acceptance.sh` builds and runs only the suite:

```text
cargo test --workspace --locked --features midnight-acceptance/acceptance --test acceptance -- ARGS
```

`--workspace` resolves features exactly as the application and the workspace tests
do, so the shared crates are reused rather than rebuilt with a different feature set;
`--test acceptance` builds only the suite's target. Tests always use the debug test
profile; the `debug|release` argument names the preceding build only, whose plugin
packages the viewer scenarios stage: the script exports `MIDNIGHT_ACCEPTANCE_PLUGINS`
as `${CARGO_TARGET_DIR:-<checkout>/_build}/<profile>` unless it is already set (made
absolute; `--dry-run` prints it). Options:

| Option | Effect |
| --- | --- |
| `--list` | List the scenarios and exit. |
| `--only A,B` | Run the scenarios whose names contain `A` or `B`. |
| `--scale smoke` | Smaller bulk fixtures (400 instead of 3,000 files, 1–2 MiB instead of 8–24 MiB). |
| `--strict-review` | UI review flags fail the run instead of warning. |
| `--all-themes` | Review every theme instead of the representative subset. |
| `--keep-fixture` | Keep the fixture and scratch folders after a passing run. |
| `--stress` | The opt-in stress sizes instead of the default ones, for the scenarios that have one (`Options::stress`); the script exports `MIDNIGHT_ACCEPTANCE_STRESS=1`, which staged children read. |
| `--run-dir DIR` | Create the run's folder in the absolute folder `DIR` (exported as `MIDNIGHT_ACCEPTANCE_RUN_DIR`; see "Results"). |

The runner also reads `MIDNIGHT_ACCEPTANCE_FILTER` (comma-separated),
`MIDNIGHT_ACCEPTANCE_SCALE`, `MIDNIGHT_ACCEPTANCE_RUN_DIR` (or its own `--run-dir DIR`,
see "Results"), `MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE=1`,
`MIDNIGHT_ACCEPTANCE_STRICT_REVIEW=1`, `MIDNIGHT_ACCEPTANCE_ALL_THEMES=1`,
`MIDNIGHT_ACCEPTANCE_STRESS=1` (or its own `--stress`; staged children read only the
variable), `MIDNIGHT_ACCEPTANCE_PLUGINS` (the folder whose `viewers/`, `filesystems/` and
`thumbnailers/` the viewer scenarios stage; default: the test executable's profile
folder, `_build/debug`) and `MIDNIGHT_ACCEPTANCE_STAGE_ROOT` (the folder in which the
viewer scenarios create their stages; required on Windows, see "Windows"). The built
test executable (`_build/debug/deps/acceptance-<hash>`) can also be run directly, with
scenario names as plain arguments. Exit status: 0 when every scenario passed or warned,
1 when one failed, timed out or could not run, 2 for invalid arguments (also a gated
folder that is not absolute, below, and on Windows run folders on a volume that is not
qualified for Copy and Move, see "Windows"), 127 when Cargo is missing.

Gated volumes are kinds of volume a generated fixture cannot provide. Each is named by a
variable holding an absolute folder the account owns, which the scripts check, pass on
unchanged and print, and staged children inherit:

| Variable | Folder on |
| --- | --- |
| `MIDNIGHT_ACCEPTANCE_FAT_DIR` | a FAT or exFAT volume (it stores no links) |
| `MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR` | a removable drive |
| `MIDNIGHT_ACCEPTANCE_NETWORK_DIR` | a network share (Windows) |
| `MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR` | another ext4 filesystem than the run's (Linux) |
| `MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR` | a filesystem without `O_TMPFILE` (Linux, such as WSL's `/mnt/c`) |
| `MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS` | a folder holding symbolic links that an administrator made (Windows) |

A scenario takes one with `fixtures::gated(context, variable)`: a new folder
`midnight-acceptance-<run>-<scenario>` there, which the returned guard removes with
everything in it (owner permissions given back first). Without the variable it returns
`Err("SKIP: <variable> not set")`, so the scenario reports `SKIP` with that reason;
`fixtures::gated_dir` names the folder itself. In WSL such volumes are loop-mounted
images prepared as root, for example `mkfs.vfat` or `mkfs.ext4` on an image file and
`mount -o loop,uid=$(id -u)`; `/mnt/c` has no `O_TMPFILE`.

### Windows

```text
pwsh -File scripts\acceptance.ps1 [debug|release] [--dry-run] [--list] [--scale smoke|full]
     [--only NAME[,NAME...]] [--strict-review] [--keep-fixture] [--all-themes] [--stress]
     [--run-dir DIR] [--plugins DIR | --validation-packages] [--test-binary FILE] [--offline]
```

`scripts/acceptance.ps1` is the Windows runner (PowerShell 7; `build.bat` has no
acceptance step). It builds the suite with the same Cargo command as
`scripts/acceptance.sh` (plus `--no-run`, and `--offline` when asked), or takes an
already built executable with `--test-binary`, and runs it from a private installation:
a new folder `%USERPROFILE%\MidnightAcceptance-<guid>` whose DACL grants only this
account, SYSTEM and Administrators (not inherited), as `scripts/validate.ps1` and CI
make their private test installations. The Windows plugin host loads a plugin only
when no other account can change it or any folder above it, up to the drive's root, and
a checkout's drive usually grants Authenticated Users write access, so stages below
`_build` could never load a plugin. The folder holds a copy of the test executable in
`bin\` (a rebuild cannot replace the running one, and staged scenarios start it again)
and the plugin packages in `packages\{viewers,filesystems,thumbnailers}` (network
providers are left out; a kind's folder only when one of its packages is staged, so that
without packages the viewer scenarios report `SKIP`, as on Linux); the runner sets `MIDNIGHT_ACCEPTANCE_PLUGINS` to `packages\`,
`MIDNIGHT_ACCEPTANCE_STAGE_ROOT` to the folder itself and `TEMP`/`TMP` to `_build\tmp`,
and removes the folder afterwards. When it ends, however it ends, it restores the
working directory and every variable it set (also `MIDNIGHT_ACCEPTANCE_RUN_DIR`,
`MIDNIGHT_ACCEPTANCE_STRESS`, `CARGO_HOME` and `RUSTUP_HOME`), so a run in-process (`& scripts\acceptance.ps1`)
leaves the PowerShell session as it found it and the next run there stages the same
packages. Packages come from `<target>\<profile>` (a normal
`build.bat` build; `CARGO_TARGET_DIR` is honoured), from `--plugins DIR` with that
layout, or with `--validation-packages` from what `scripts\validate.ps1` left in
`_build\validate` (`viewer-packages`, `filesystem-packages` and
`archive-ci\package\archive`). `--list` runs the executable in place. Run folders follow
the rules under "Results" (`--run-dir` must be absolute).

A run stops before it builds or stages anything (exit status 2) when the volume of its
run folders (`--run-dir`, `MIDNIGHT_ACCEPTANCE_RUN_DIR` or `_build\acceptance`) is not
qualified for Copy and Move as the platform decides it
([TEST_EXECUTION.md](TEST_EXECUTION.md#windows-copymove-qualification-volume)): NTFS
whose persistent volume state disables 8.3 short-name creation. The runner reads that
state as the platform does (`FSCTL_QUERY_PERSISTENT_VOLUME_STATE` on the volume's root
folder, from a small C# type that the script compiles with `Add-Type`; no elevation),
not the registry, and not whether a new file got a short name. On any other volume
(system volumes such as `C:` usually still make 8.3 names; ReFS, Dev Drive and FAT never
qualify) the engine takes its opaque lane, where Copy and Move jobs wait for each other,
so the scenarios that run jobs side by side would fail for no fault of the product. The
message names the run folders, the volume and why; pass `--run-dir` on a qualified
volume. `--dry-run` prints the verdict (`Run folder volume: D:\ (NTFS, 8.3 short-name
creation disabled)`, or the reason and "a run there stops") and `--list`, which runs no
scenario, is not checked. The script tests stand in for the probe with
`MIDNIGHT_ACCEPTANCE_SCRIPT_TEST_VOLUME` (`qualified` or `unqualified`); nothing else
sets it.

The other options and the exit status are those of `scripts/acceptance.sh`. Without `MIDNIGHT_ACCEPTANCE_STAGE_ROOT` a
viewer scenario on Windows reports `SKIP` with that reason. The Windows runs, their
results and findings are recorded in [acceptance/windows.md](acceptance/windows.md).

What differs on Windows, and why:

- Staged children get a private profile instead of the XDG folders: `USERPROFILE`
  (the application's settings resolve through the LocalAppData known folder,
  `%USERPROFILE%\AppData\Local`), `LOCALAPPDATA` (thumbnail cache, browser sessions,
  plugin workers), `APPDATA` and `TEMP`/`TMP`, all below the child's stage or work
  folder. A child kills its own process tree (`taskkill /T`) where Unix kills its process
  group; plugin workers of a child that already exited end with it, in their kill-on-close
  job objects.
- Windows keeps one Recycle Bin per volume and account, which no variable redirects:
  `files_trash` reports `SKIP`, `files_delete` cancels F8 without reading the Recycle
  Bin, and `ui_polish_dialogs` never confirms the Trash dialog (Enter and Move to Trash
  are exempt, with that reason, in its notes).
- Permissions (`fixtures::deny_read`, `deny_write`, `tools_support::set_mode`): an
  unreadable folder or file (mode 000) is an explicit deny entry for this account's own
  SID on listing or reading data and on writing or adding to it (`icacls /deny
  <SID>:(RD,WD,AD)`), as mode 000 refuses both; a folder without owner write (`5xx`)
  denies adding files and folders and deleting its children (`(WD,AD,DC)`), and each
  entry it holds denies its own deletion (`(DE)`), because Windows lets an entry go when
  either it or its folder allows that; a file without owner write gets the read-only
  attribute. The owner can always change its own DACL, so the per-scenario restore
  removes the deny entries (and clears the read-only attribute) wherever a folder cannot
  be listed or refuses a new entry, and on the files the process made unreadable (an
  unreadable file never keeps its folder from being deleted). A listable folder whose
  entries cannot be reached (mode 0444 on a folder) has no equivalent (traverse checking
  is bypassed): `tools_compare_dirs` notes that part as not run.
- Links: creating a symbolic link needs administrator rights or Developer Mode, so the
  fixture's `links/` is empty. `files_links` and `tools_sizes` use directory junctions
  instead (no privilege). Through the Copy/Move dialog's Links choice, "Keep links as
  links" recreates the junctions as junctions with the same target, and "Copy the files
  links point to" follows the junction to a folder (its tree arrives), keeps the
  dangling junction as a junction with a note and follows the junction to its own folder
  once (see `files_links`). `journeys_links_readonly` reports `SKIP`: its checks are Linux
  relative symbolic links and permission modes, Windows symbolic links need Developer
  Mode or administrator rights and folders are protected by ACLs.
  `fixtures::junction` sets them natively with `FSCTL_SET_REPARSE_POINT`
  (`tests/acceptance/junction_windows.rs`, one audited FFI call), as
  SECURITY.md requires of test fixtures: no shell reads the paths, so `%`, `!`, `^` and
  `&` in them are plain characters. Fixture generation first checks that on Windows: a
  junction whose link and target names hold all of them and a non-ASCII letter must read
  back as its target (the `cmd /c mklink /J` it replaced refused `%`, and the `/d` it ran
  with does not turn off delayed expansion set in the registry, which changes `!`).
- Unsafe code: the suite denies it (`#![deny(unsafe_code)]` in `tests/acceptance.rs`;
  deny, not forbid, because the `slint!` fixture windows of the polish gate's red cases
  are generated bindings with their own narrow allowance). Three Windows-only places
  allow it, each with its reason and a `SAFETY` comment per call: the junction's
  `FSCTL_SET_REPARSE_POINT` (`junction_windows.rs`), the video scenario's
  `FSCTL_SET_COMPRESSION` (`scenarios/viewers_video.rs`), both `DeviceIoControl` calls
  declared in the file, and the H.264 + AAC movie written through Media Foundation's
  Sink Writer (`scenarios/viewers_video_h264.rs`), the only user of the suite's Windows
  dependency `windows` 0.62.2 (features `Win32_Foundation`,
  `Win32_Media_MediaFoundation`, `Win32_System_Com`).
- External programs: Windows has no `/bin/cp`, `touch`, `true` or `/bin/sh`, so the
  scenarios that configure editors, comparators and Custom Commands get stand-ins
  (`tests/acceptance/tool.rs`): the test executable as `cp.exe`, `touch.exe`, `true.exe`
  and `sh.exe`, which act as the program their name says (`sh` runs only the suite's
  literal list script and `-c true`). The executable is over 100 MB, and a hard link
  needs its volume: run from the private installation (or a stage in it), the stand-ins
  are hard links in a `.tools-<run>` folder beside it, which goes with the installation
  or the stage. Only otherwise, or when such a link fails, do they go into the run's
  `work\.tools\`, as hard links when the run folder shares the executable's volume and
  as copies when not (the private installation lies below the user profile, usually on
  another drive than the checkout's run folders, so every stand-in used to be a copy).
- Hidden files: Windows hides by the hidden attribute, not by a dot-name, so the
  scenarios that hide dot-names give them that attribute too (`fixtures::hide`). The
  product's own `.midnight-recovery-*` folder gets no hidden attribute, so on Windows
  `journeys_filtered_conflicts` expects it listed while hidden files are hidden.
- Attributes: the Ctrl+F2 dialog shows Windows attributes (three-state Read-only,
  Hidden, System, Archive, Compression boxes) instead of permission boxes and octal
  modes, so `files_attributes` sets and clears attributes there and `panels_dialogs`
  sets Read-only where Linux types octal 600; times are checked to NTFS's 100 ns.
- Sparse files: NTFS allocates the whole length of an extended file, so
  `tools_support::sparse` marks files of 1 MiB or more sparse first (`fsutil sparse
  setflag`, no privilege needed). Smaller ones take their space: `find_budgets`' 100,001
  files of 1 to 100,001 bytes hold about 5 GB until the run folder is removed.
- Another volume: `files_transfer_errors` copies and moves to a folder in the private
  installation (`MIDNIGHT_ACCEPTANCE_STAGE_ROOT`) when it is on another drive than the
  run; otherwise it notes that the cross-volume checks did not run.
- Packages: `--validation-packages` holds no HTML viewer and no thumbnailers
  (`scripts\validate.ps1` packages neither), so the HTML cases of the viewer scenarios
  report themselves skipped with that reason and `viewers_thumbnails` reports `SKIP`; a
  normal `build.bat` build has both. Those packages are as old as the last
  `scripts\validate.ps1` run: viewer checks of behavior added since then fail with
  them, so a viewer result counts only with packages built from the source under test.
- Paths: the run folder is the canonical path without Windows' verbatim `\\?\` prefix,
  as a user would type it; relative paths in expectations use `/` on every platform
  (`files_support::slashed`); the filesystem root is the fixture drive's root
  (`files_support::filesystem_root`).

### Results

Each run creates a new folder `_build/acceptance/<YYYYMMDD-HHMMSS>/` (UTC; `-2`, `-3`, …
are appended when a run that started in the same second took the name) and records its
path in `_build/acceptance/LATEST`. `--run-dir DIR` or `MIDNIGHT_ACCEPTANCE_RUN_DIR=DIR`
names another folder in which the run creates its new folder, `DIR/<YYYYMMDD-HHMMSS>/`;
`DIR` must be absolute and not a filesystem root (an empty, relative or root value is
refused with exit status 2). A run never uses an existing folder as its own and deletes
only inside the folder it created, apart from pruning (below); it replaces a `LATEST`
file only when that holds a run folder's path (one line ending in a run folder's name):

| Output | Content |
| --- | --- |
| `index.html` | Summary table, per-scenario notes, failure messages, screenshot galleries with their review flags, and the fixture catalogue. |
| `summary.json` | Status, duration, message, screenshot and flag counts per scenario. |
| `review-flags.tsv` | Every UI review flag: scenario, screenshot, kind, detail. |
| `shots/<scenario>/*.png` | Compressed PNG screenshots (the application's own snapshot encoder). |
| `fixture/`, `work/` | The generated tree and the scenarios' scratch folders; removed after a passing run unless `--keep-fixture`. |

Screenshots use the fonts of the machine that runs the suite. The Docker build image
has only DejaVu fonts, so Japanese names and emoji render as missing-glyph boxes there;
that is the environment, not a layout fault (the review's checks measure layout, not
glyph coverage).

The console ends with one line per scenario (`PASS`, `WARN`, `FAIL`, `SKIP`,
`PENDING`, `TIMEOUT`, `NOT RUN`), its duration and headline, the counts, the report
path and `Result: PASS|FAIL`. `PENDING` means every check passed except checks of
announced behavior that has not landed yet; each is named with its reason in the
message and notes, and it is not a failure. After every scenario, passed or failed,
the runner gives owner write permission back to everything below its work folder
(links not followed): folders get owner read, write and search, files owner write. A
work folder that still has an entry without them fails its scenario, and at the end the
whole run folder is restored and checked the same way, so a kept run folder can always
be deleted, in a file manager too.

Each run folder holds a marker, `.acceptance-run`, from its start. When a run starts it
prunes the folder its own folder is in (`_build/acceptance` or the `--run-dir`): it keeps
the four newest finished runs there (those with their `summary.json`), so that five
remain with its own, and deletes older finished ones; a run without `summary.json` is
still going (or was killed) and is deleted only once it is two days old. Pruning deletes
only folders named like run folders: in a `--run-dir` only those that hold the marker,
so other folders there stay; in `_build/acceptance` also those without it, which earlier
versions made. Runs that share a folder (parallel worktrees with one `_build`) therefore
never delete each other's running runs; a package or person that wants its own five
keeps them in its own `--run-dir`.

## How it is kept out of normal runs

`crates/midnight-acceptance` is a workspace member whose only test target is declared
with `required-features = ["acceptance"]` and `harness = false`. No other package
enables that feature, and every dependency of the crate is optional behind it, so:

- `cargo test`, `cargo test --workspace` and `cargo build --workspace` compile an
  empty library (`test = false`, `doctest = false`) and skip the target; other
  packages' feature sets are unchanged.
- `scripts/ci-workspace-tests.py` (CI and `--validate`) only selects targets whose
  `required-features` are active, and lists `midnight-acceptance` among the excluded
  libraries of its `--lib`/`--doc` gates.
- `cargo clippy --workspace --all-targets` skips the target as well. CI's
  `scripts-and-lint` job therefore enables the feature in its one workspace Clippy
  step, on both platforms:
  `cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance -- -D warnings`.
  Every other package then resolves exactly as without the feature (checked with
  `cargo tree --workspace -e normal,build,dev` for both targets), so the step checks
  each dependency once. A separate `cargo clippy -p midnight-acceptance` step would
  resolve features without the workspace's dev-dependencies (tokio without
  `test-util`, other `regex-automata` and `tracing` features, on Linux other `x11rb`
  ones) and check tokio and every crate above it a second time.

The native input seams ("What it drives") are compiled only into this suite:
`midnight-ui` and `midnight-viewer-ui` each have an `acceptance-seams` feature, and only
this package's `acceptance` feature enables them (`"midnight-ui/acceptance-seams"`,
`"midnight-viewer-ui/acceptance-seams"`). The application and every other build compile
neither the seams nor the observer handles they keep: for the application,
`cargo tree -p midnight-explorer -e features -i midnight-ui` (and
`-i midnight-viewer-ui`) lists no `acceptance-seams`.

A dedicated feature-gated target was chosen over `#[ignore]` because ignored tests
still compile into a normal test binary (and `--include-ignored` would run them), and
over a separate binary because the suite needs the test profile's dependencies only.

## What it drives

`tests/acceptance/app.rs` starts the application the way
`crates/midnight-explorer/src/main.rs` (`run_window`) does: the Engine with
`LocalFileSystem` installed for navigation fallback, audited transfers, Find,
locale, mutations, watching and in-memory settings; `install_with_owned` with the
production owned-command scope (`command_reclaim.rs`, included by path); and the
work the application does around each frame: the event pump of its `backend_ready`
callback (claim, dequeue, apply, refresh), the retry pass's flushes (selection,
filters, filename colours, search, comparison), the visible-listing acknowledgement,
and the visible-row sampling of its thumbnail pulse, which also tells the presenter
each tree viewport (inline-tree expansion waits for it). Every window the
application opens gets a `MinimalSoftwareWindow`; scenarios send key, pointer and
wheel events to those windows (`input.rs`, `panel.rs`) and never call presenter
methods. `input::click` scrolls a control that a scroll view shows only in part into
view with the mouse wheel and clicks its visible part; `panel::click` waits out the
double-click interval before a second click on the same row. A wait that times out
reports both panels, the open notice and the last 32 events and commands sent.

**Native input (acceptance seam A1).** The panels and viewers take a wheel notch, the
system's lines per notch, touchpad reports and their momentum, the reduced-motion setting
and (viewers) an inactive window's selection colours only from native Winit events, which
software windows never receive, so synthetic Slint wheel events bypass them. With the UI
crates' feature `acceptance-seams` (enabled by this crate's `acceptance` feature; the
application never enables it), `midnight_ui::acceptance_seams` (handle `&MainWindow`) and
`midnight_viewer_ui::acceptance_seams` (handles `&viewers, &window`) hand native-equivalent
input to the same observer closure the Winit hook runs:

- `wheel(..., at, NativeWheel::Lines { x: 0.0, y: -1.0 })` delivers one notch down at
  `at` (logical pixels; Winit's lines and signs, fractional lines for a high-resolution
  wheel or a precision touchpad), after moving the pointer there;
  `NativeWheel::Pixels { x, y, phase }` is a pixel touchpad's report in physical pixels
  with its phase. It returns `Some(true)` when the panels or the viewer took it; what they
  leave alone reaches Slint as Winit's backend converts it (a pointer move, then a wheel of
  lines × 60 or physical pixels ÷ scale, with its phase).
- `focus(..., focused)` delivers a native focus change, then Slint's activation change.
- `override_settings(SystemSettings { wheel: Some(WheelSetting::Lines(5)),
  reduce_motion: Some(true) })` sets what they read as the system settings
  (`WheelSetting::Page` is one screen per notch) until the returned guard drops (guards
  end in the reverse order of creation). Settings are read with the first native event
  and on each focus gain, so call `focus(..., true)` after overriding them.
- While the content moves (a wheel's glide, touchpad momentum), the seams send the
  per-frame `RedrawRequested` Winit would (for the viewers every 8 ms, until 250 ms after
  the last seam input), carried by the frame pump's timers, so the usual waits apply. The
  viewer seams return `None` for a window the viewer manager did not open and for a hidden
  or closed one. Seam events do not give a software window native shortcut authority: its
  Slint key events keep resolving digit shortcuts.

Time, as the event loop and the product see it:

- `app.settle()` is one frame's worth of work: pending Engine events applied, the
  retry pass's flushes, Slint's timers and animations brought up to the clock, a
  redraw. `app.until(what, condition)` settles until the condition holds (60 s hang
  guard; `until_for` names another); `app.pump(duration)` lets real time pass as the
  event loop does, settling about every 10 ms (`app::FRAME`), so timers fire and
  animations move frame by frame; `app.idle(duration)` is one sleep and one settle
  (timers jump once) and is not a synchronisation.
- Slint's timers and animations follow the platform clock, an `app::Clock` that runs
  with real time. `app.hold_clock()` stops it; `app.advance(duration)` then moves it
  in 10 ms steps, settling after each, so a timer fires in the step that reaches it;
  `app.resume_clock()` lets it run on from there (it never goes back; the next
  `App::start` on the thread finds it running). Everything the product times on that
  clock follows: the drag bridge's 0.4 s quick-drop rule, the time picker's repeat, the
  panels' 100 ms rest check and 200 ms glide to a whole row, startup's settings
  deadline. Engine events still arrive in real time, so a step that needs the Engine
  pumps real time while the clock is held. Two clicks at one spot within the
  double-click interval of the clock's time are a double click whatever real time
  passes between them, so advance the held clock past it between separate clicks.
- `App` counts what a step may wait for: `copies` and `operations` (progress
  reports), `listings`, `checksum_runs` and `manifests`, `links_inspected` (each answer
  to the Copy/Move dialog's question how links fare at its destination,
  `Event::TransferLinksInspected`), `archive_unlocks` (each answer to an archive
  password, `Event::ArchiveUnlocked`) and `settings_reads`.
- A tool window that closed (a job window whose job ended) keeps its adapter but not
  its content; Slint 1.18.1 panics (`window.rs:781`) when such a window is resized or
  rendered. `app::shown(window)` says whether it is still shown; `app::layout_tool`
  then does nothing and returns false, `app::render` returns an empty frame and
  `app.screenshot_window` notes that the window had closed.

Deliberate differences from the application, to keep runs hermetic and headless:

- Settings live in memory: the user's profile is never read or written. Scenarios
  that test persistence opt into `Config::settings`, an
  `options_support::SettingsStore`: the first run seeds it from its `Config`, every
  save replaces it, and a later `App::start` on the same store loads what was saved,
  like a restart on one profile. Its font list is fixed (`DejaVu Sans`, `DejaVu Sans
  Mono`, `DejaVu Serif`). `Config::real_settings` installs the application's own
  store (`LocalFileSystem`, as `run_window` does, without its save-until-shown gate)
  instead; only a staged private process (`viewers_stage::run_private`) may use it,
  since it reads and writes `$XDG_CONFIG_HOME/midnight-explorer` (Windows: the
  LocalAppData known folder, `%USERPROFILE%\AppData\Local\Midnight Explorer`), which is
  private there. `App::start` checks that those folders (`XDG_CONFIG_HOME`, or
  `$HOME/.config`; Windows: `USERPROFILE` and `LOCALAPPDATA`) lie below the process's
  private `<work>/staged` folder, as `startup_paths` does before it seeds the store;
  that the variables are merely set would prove nothing (Windows always sets
  `USERPROFILE`). A LocalAppData known folder redirected in the registry to a path that
  does not follow `%USERPROFILE%` is not detected.
- The tab session: by default the presenter keeps its defaults and the first read
  restores the saved tabs of both panels. `Config::restore_tabs` starts as `run_window`
  does without paths (`set_tab_restore_overrides([false; 2])`), and
  `Config::explicit_paths` as it does with `--left`/`--right` (`true` for each panel
  given a path). `App::start` waits, as production keeps its window hidden, for the
  first settings read to apply (read or failed) and then for the restore of each panel
  with a saved session and no path of its own, or for production's 10 s settings
  deadline (`app::SETTINGS_DEADLINE`, from `startup.rs`): past it the window shows the
  defaults and its status says "Settings are still loading; defaults are shown."
  (`App::settings_late`), and the read applies when it arrives. Then it waits for both
  listings; a start that is not ready within 60 s panics with the presenter's state.
  `Config::settings_delay` makes every read of whichever settings store answer that
  late, as from a slow profile folder.
- Browsing uses the engine's host-native `LocalFileSystem` service, not the installed
  native filesystem package that the application browses through
  (`InstalledProvider::shared_composition` in `main.rs`). Only the staged scenarios that
  set `Config::plugins` browse through the package, archives included: the `viewers_*`
  scenarios, `files_archive`, `find_archives`, `find_viewers`, `panels_root_keys`,
  `ui_states`' scenarios and the staged journeys (see "Viewers, comparators, thumbnails
  and archives"). In every other scenario the package's part of the product (listing,
  `stat`, volumes and locations, activation, refresh and the resolution of a typed path)
  is the engine's own, so a difference only the package has (such as how the native host
  maps "not a directory") passes there and fails in the application: check such
  behaviour in a staged scenario. Transfers, mutations, Find and watching use
  `LocalFileSystem` in the application too.
- The operating system's file chooser cannot be driven headless. A scripted
  `PathPickerService` (`app::ScriptedPicker`) stands in: it returns answers a scenario
  queued with `app.picker.answer(..)`, otherwise the suggested name in the suggested
  folder, as a user pressing Save does, and records every request.
- No terminal is ever started, and no file association or system location is ever
  opened (`tests/acceptance/launches.rs`). By default no desktop service is installed:
  the command line reports "Terminal unavailable" and opening a file is unavailable.
  `Config::desktop` installs a stand-in instead: the platform's read-only parts
  (locations, icons, property details, link targets, the shell's flavour), with the
  command line's and the terminal button's terminals (standard or elevated), files
  opened with their association and system locations (the Recycle Bin) recorded in
  `app.launches` (`Launch::Terminal`, `Launch::Open`, `Launch::SystemLocation`) and
  never started; the Engine gets a process that has already ended successfully, with
  id 0; Properties shows the application's own summary, never the system's dialog. The
  desktop drag-and-drop hand-off is not installed either. Drag and drop between the two
  panels needs none of it: the application's internal drag path runs as in production,
  driven by real pointer events (`panel::drag_row`).
- The desktop clipboard is replaced, opt-in (`Config::clipboard`), by an in-process
  `ClipboardService` (`app::MemoryClipboard`), installed where `run_window` installs
  the native one: Ctrl+C, Ctrl+X and Ctrl+V run the application's own capture, intent
  and paste flows, and a scenario can read what was put on the clipboard.
- Viewers, comparators and thumbnails are installed only in the `viewers_*` scenarios,
  which run in staged processes (below).
- External editors (F4) and Custom Commands are opt-in (`Config::editor`,
  `Config::user_commands`). The editor launcher is the platform's own; Custom Commands
  go through `launches::UserCommands`: the platform's validation runs as in production,
  a program is recorded (`Launch::Program`) and started, and a terminal-mode command is
  recorded with its environment (`Launch::Terminal`) and never started. Programs start
  real processes, so only scenarios that configure harmless programs writing into their
  own work folder enable them (`/bin/cp`, `/bin/sh` with a literal script, `touch`,
  `/bin/true`; on Windows the stand-ins of `tests/acceptance/tool.rs`, see "Windows");
  none runs a real editor, and no terminal starts.

## Scenarios

| Scenario | Area | What it proves |
| --- | --- | --- |
| `startup` | panels | Both panels open on the fixture root and list every top-level folder. |
| `harness_checks` | harness | The harness itself: the held clock stands still while real time passes, a 400 ms timer started on it fires when it is advanced 400 ms and not after 390, and it runs on from there when resumed; a 20 ms timer fires again and again in 300 ms of `pump` and at most twice in 300 ms of `idle`; a closed F1 help window counts as not shown, `layout_tool` and `render` leave it alone and `screenshot_window` notes it; F5's Copy dialog asks how links fare at the other panel's folder (`links_inspected`); no terminal starts: without the desktop stand-in the command line's Enter reports "Terminal unavailable" and launches nothing, with `Config::desktop` it is recorded (`Launch::Terminal` with the command, the panel's folder and the standard mode) and no notice follows, and a terminal-mode Custom Command handed to the `Config::user_commands` launcher is recorded with its environment and ends at once with success (process id 0), never reaching the platform's terminal; a gated variable that is not set gives `SKIP: <variable> not set` and an unknown one an error; with settings that answer after 13 s the start shows the defaults (Blue) with "Settings are still loading; defaults are shown." and the read then applies (Dark); pruning a folder of run folders keeps the four newest finished runs besides the new one, a run still going, a run folder without the marker and another folder, and deletes older finished and old unfinished runs. |
| `startup_paths` | panels | Starts as production does with `--left` and `--right`, with only one of them and with neither, against the application's own settings store (preferences read asynchronously from `$XDG_CONFIG_HOME/midnight-explorer/preferences.json`) in the private profile of a staged process (`viewers_stage::run_private`), seeded each time with a saved tab session (two tabs on the left, one on the right). Each start must be ready (settings applied, both listings done) within production's 10 s settings deadline; a panel given a path shows only that path, the other restores its saved tabs and active tab, and the saved tabs of a panel without a path are still in the profile afterwards. |
| `navigation` | panels | Enter/Backspace into every fixture folder (and a level deeper, and the 12-level chain); each listing equals the folder on disk; Backspace returns the cursor to the folder left. |
| `copy_move_delete` | transfers | F5 and F6 of a single file and a folder with awkward names, Shift+F8 of a file and a folder, confirmed in the real dialogs; bytes, trees and panel rows verified. |
| `create_rename` | operations | Shift+F4 creates an empty Unicode-named file and asks before replacing an existing one: a second Return in the name field does not replace it, the Replace file button does (emptying it), and the notice names the recovery copy that holds the original; F7 creates a folder; F2 renames with the stem preselected and the panel shows the new name at once, cursor on it. |
| `tree_expand_all` | tree | Ctrl+click on the `deep` folder's expander expands the whole 12-level branch inline (the tree lists folders), each `level-NN` at depth NN, and no other branch. |
| `folder_sizes` | tree | Ctrl+Shift+S measures every top-level fixture folder; each size (shown in bytes) equals the logical bytes on disk, links not followed. |
| `find_by_mask` | find | Alt+F7 Find with `*.md`, `*.webp` and `*.sha256` returns exactly the matching files on disk; Go to file puts the panel cursor on a result. |
| `checksums` | checksums | Ctrl+F8 on four marked files: Calculate (SHA-256 checked against an independent implementation), Save manifest…, the Replace prompt, Verify; then the fixture's MD5, SFV, mismatching and incomplete manifests. |
| `files_copy_kinds` | transfers | F5 of every entry of `text/`, `code/`, `docs/`, `images/`, `archives/`, `sizes/`, `names/`, `readonly/` and of a folder of distinct permission modes, one at a time to the other panel; each folder's files as a Ctrl+A multi-selection into a typed subfolder with "Preserve exact modified time"; the 12-level `deep/` tree. Bytes, exact names, trees and nanosecond times (where promised) verified; the new row is listed and the sources are unchanged. Every copied file and folder, inside copied folders too (`modes/tools/inner.sh`), must have the source's permission bits masked by the process umask, as SECURITY.md promises (`[AF-2]` otherwise; a folder may keep an inherited set-group-ID bit). |
| `files_move_kinds` | transfers | F6 of the same entries one at a time and as Ctrl+Shift+A multi-selections into typed subfolders; the source is gone, content, exact name, mode and modified time are those of the moved entry, both panels update, and the number of moved entries is checked. The owned write-protected `readonly/locked-folder` (mode 0555, with contents) moves as it is and keeps its mode, as every moved folder must; any question fails the scenario (it is answered with Cancel job, so the next move starts clean). |
| `files_conflicts` | transfers | Copy and Move onto existing names in the decision window: Replace (original kept in the recovery folder), Skip, Copy as, Replace all and Skip all (one question), Merge folders then Skip a child, Cancel job midway (earlier copy kept, later file not started), a copy into its own folder, Move with Skip, Replace and another name, and moving a folder into itself or its subfolder (refused, nothing changes). A deliberate skip must not be reported as a failure. |
| `files_links` | transfers | F5 and F6 of a link to a folder, to a file, a dangling link and a link to its own folder (a cycle) through the Copy/Move dialog's Links choice. On a fixed local folder the dialog proposes "Keep links as links": every link arrives as a link with the same target text, and the job details name each link whose relative target leads outside the copied items. "Copy the files links point to": the folder link arrives as the folder's tree, the file link as the file's content, and a link that cannot be followed (the dangling one, and the self link met again inside the followed folder) stays a link with a note in the job details. F5 of a folder holding only a link to itself copies the folder once with the link kept. F6 keeping links moves each link itself (source gone, same target). The choice is made by keyboard (Tab to Links, Up or Down, Enter), by pointer (a click on the entry) and with the open list's own keys (Enter there chooses, it does not confirm). Every run must complete without a question, notice, skip or failure, confirmed with the Links choice it was given (`[AF-3]` otherwise), and sources and link targets must not change. Windows: directory junctions to a folder, to a missing folder and to their own folder; kept as links, all three are recreated as junctions with the same target; copying the files, the folder junction arrives as the folder's tree, the dangling junction stays a junction with a note, and the junction to its own folder is followed once (the junctions met again inside that copy stay junctions, each with a note); F6 moves all three. |
| `files_delete` | operations | Shift+F8 of every entry of the kind folders one at a time, of each link (its target stays), of a mode-0444 file and a mode-0555 folder, of a Ctrl+A multi-selection and of the 12-level tree; F8 only opens the Trash confirmation, which Escape cancels, and the Trash folder is unchanged. It runs in a staged child process (`viewers_stage::run_private`) whose `XDG_DATA_HOME`, and so the application's home Trash, is a private folder in its work folder: the account's own Trash is neither used nor read. |
| `files_trash` | operations | F8 for real, into the private Trash of a staged process (`viewers_stage::run_private`): a file (bytes, `[Trash Info]`, percent-encoded `Path=`, `DeletionDate=`, `Trash`/`files`/`info` mode 0700), a folder (tree, modes and times kept), a Ctrl+A multi-selection, awkward names (Unicode, spaces, `#&$%;`, a 224-byte name shortened to a 32-byte label), a mode-0444 file (mode kept) and a nested row of the inline tree; a permanent delete of 3,000 files paused (its window holds still), resumed and cancelled (every file left is whole); and, strict checks since 21 November: a mode-0555 folder with contents goes to the Trash (owner write is added inside the deleted tree); a file of a mode-0555 folder asks Retry/Skip/Skip all/Cancel and Retry works once the folder is writable; two such files end with Skip all and stay; Shift+F8 of a tree holding a mode-0555 folder deletes it whole without a question. |
| `files_copy_options` | transfers | The Copy dialog for one file: summary (the name; the source folder is its tooltip), destination (the other panel), the policy "Verification: full SHA-256." (the Copy button's tooltip), "Preserve exact modified time" and "Wait for earlier transfers" shown unchecked, no former link check box ("Follow links", "Follow directory links"), and, once the destination's link facts arrived, the Links choice labelled "Links" proposing "Keep links as links" of its two entries on this fixed local folder, with no reason or warning; a relative path, an empty destination and a missing folder are refused in the dialog ("Enter the full path of an existing destination folder."; a missing folder admitted as a job fails the scenario with finding ACC-OPT-01, and its question is answered with Cancel job), Escape copies nothing; exact times are not offered for a folder; the Move dialog's policy line and the same Links choice. The large file copied with full verification (every byte read back) and, after clearing "Verify entire copied file with SHA-256 (recommended)" in Options › General, with size and samples (at most three 64 KiB samples read back; the dialog's policy line says so); `many/` in one job (3,000 files, each synced and read back: 5 min allowed), tree verified; rows marked at depths 1 and 2 of two inline tree branches copied together, each as its own folder. |
| `files_transfer_jobs` | jobs | "Wait for earlier transfers" off and on with three jobs: job 1 (`many/`) is paused; job 2 (off) runs at once and completes beside it while job 1 stays paused; job 3 (on) reports `EarlierTransfers` and "Queued — waiting for earlier transfers" and copies nothing until job 1 ends after Resume (its 3,000 files may take minutes on a busy disk: 5 min allowed); the Jobs window lists the three states. A 1 GiB sparse file (only its copy takes space, removed afterwards) is paused in the middle of its copy: its window shows the files and bytes totals, Time remaining, the speed chart's Scale and, under Details, the current file's progress; it is copied whole after Resume. Cancel operation (question, Cancelled, only whole files kept); the Jobs window's Cancel job stops a paused job at once without the question; with a job that ended with an error (an unreadable file skipped) in the list, Clear finished without errors removes the cleanly completed jobs and keeps it, and Clear all finished empties the list; no job window, Jobs window or notice says "published"; the progress, Jobs and decision windows, framed by their title bars, have no × of their own. After Resume, without Details, the large file shows its own bar ("Current file progress") beside its name once it has been copying for 2 s of running time (pauses do not count); a copy that ends sooner is noted as not observable. A job whose "Wait for earlier transfers" is off starts at once (a strict check since 21 November). |
| `files_transfer_errors` | transfers | Copy into a mode-0555 folder: the error offers Retry, Skip, Skip all this error type and Cancel job; Retry after the folder is made writable copies the file. Skip all this error type: one question for three files, nothing written. Cancel job at the error: Cancelled, nothing written. An unreadable source file: Retry once it is readable. An unreadable folder inside a copied tree: Skip, the rest is copied. An owned mode-0555 folder with contents moves without a question and keeps its mode. A file in a mode-0555 folder (outside the moved item) asks Retry, Skip, Skip all this error type and Cancel job (still checked as pending: T-E06, a failure reports `PENDING`); Retry once the folder is writable must move it (finding ACC-ERR-01 otherwise). Decide later leaves the job waiting for a decision; Resolve in its window brings the question back. Merge all folders: one question for two folder conflicts. An unreadable folder inside a copied folder: one question, Skip, the outcome counts it. A copy and a move to a private folder on another filesystem (`$XDG_RUNTIME_DIR`), removed afterwards; Move must either move or keep the source untouched. The job window of a job with errors shows the error itself, no notice follows it or Cancel job, Details shows more, and an outcome counts a skipped item and never lists notes one by one. Strict checks since 21 November: the rest of a folder is copied after Skip of an unreadable subfolder; an unreadable selected folder, which fails while the transfer is prepared (Retry and Cancel job only), can be skipped so the rest of the selection is copied; an outcome counts its notes. |
| `files_clipboard_drag` | transfers | With the in-process clipboard: Ctrl+C of two marked files puts them on the clipboard to copy and Ctrl+V in the other panel copies them; Ctrl+X then Ctrl+V moves a file. Drag and drop with the mouse: a plain drag to the other panel on one volume offers and does Move, Ctrl+drag Copy, a folder dropped onto a folder row lands inside it, and Ctrl+Shift is refused ("Not allowed here") and starts nothing. |
| `files_archive_transfers` | transfers | Through the installed archive plugin (a staged process): a file, a nested folder and an empty folder marked inside a ZIP are copied out (known bytes); the same tree copied out of a tar.gz equals it; F6 of an archive member is refused before any dialog with its reason ("cannot be moved"); F5 of a native file into an archive panel is refused or reported and leaves the archive byte for byte unchanged. |
| `files_rename` | operations | F2 to a Unicode name (listed at once, cursor on it), a case-only change, a collision (Skip, then Replace), a name with `/`, an empty name, a 255-byte and a 256-byte name, a folder, and Down Down typed with the confirming Enter (held, then replayed from the renamed row). |
| `files_create` | operations | F7 with one name (cursor on it), with nested segments (cursor on the created ancestor) and with `..`, F7 on an existing folder, Shift+F4 with nested segments, on an existing file (asks; Escape keeps it; the Replace file button empties it, and a recovery copy a notice names must hold the original bytes) and with leading and trailing spaces, Escape in both dialogs. |
| `files_attributes` | operations | Ctrl+F2: permission check boxes and the octal field (Windows: the Hidden, Archive and Read-only attributes, three-state boxes set and cleared and read back as attribute bits); Modified and Accessed typed as date, time (nanoseconds) and UTC offset for one file, for three files and for a folder (its contents unchanged; NTFS keeps times in 100 ns steps). Each change is reviewed, applied and read back from disk; no notice or window stays open. |
| `files_selection` | selection | Insert, Space (with the folder size), Ctrl+A, Ctrl+Shift+A, Ctrl+D, Ctrl+I, Ctrl+Shift+I, Ctrl+click, Shift+click ranges; Select and Deselect by mask with the keypad's + and − (masks with `;`, `\|` and case, Include folders, the remembered mask, Escape) and from F10, the keypad's * with and without folders, the number row's + still searching; and F5 of a multi-selection (exactly the marked files arrive; completed roots are deselected). The keypad keys are sent through the window's physical-shortcut entry, which the native key hook calls; synthetic Slint key events carry only the typed character. |
| `files_sorting` | panels | Ctrl+F3/F4/F5/F6 pressed twice each: name, extension (extensionless last), modified time and size in both directions, compared with the disk; folders first. |
| `files_filter_search` | panels | Show hidden files: Ctrl+H, the View menu and F10 hide and show dot-names (status count, cursor to the next shown row, dropped mark, Ctrl+A of shown files, a hidden child left out of an inline tree), Options → General saved across a restart; then the Ctrl+F12 panel filter in Masks (`*.txt;*.TXT`, `*.*\|.*`) and Expression mode, Show all; filename search with Down and Escape. |
| `files_navigation` | panels | Enter and Backspace, Alt+Left/Right history, Ctrl+Backspace to the filesystem root, Ctrl+G typed paths (including a missing one, which must report an error: a notice or the panel's error), a breadcrumb click, Ctrl+U swap and back, Alt+Enter Properties of a folder (Linux has no system dialog; the application's own summary must name the folder, its type, permissions and time and count its files, folders and bytes as on disk; owner, group and the other times come from the desktop service, which this harness does not install), Ctrl+R refresh keeping cursor and selection. |
| `files_tabs` | panels | Ctrl+T, Ctrl+Shift+Tab, switching by click, the tab close button that appears only on hover, the + button, Ctrl+W and the last tab that stays. |
| `files_hot_paths` | panels | Ctrl+Shift+Plus adds the current folder as Hot Path 1, the toolbar's Hot Paths menu opens it, and Manage Hot Paths (from F10) renumbers, removes and adds again. |
| `files_file_list` | panels | Ctrl+M with a `$(FileName)$(TAB)$(FileSize)$(CRLF)` template: the preview shows the tabs expanded to eight-column stops, the File output is written byte for byte and the selection is cleared. |
| `files_tree_ops` | tree | Expand and collapse with the expander, then F5, F2 and Shift+F8 of a folder only visible three levels deep in the inline tree; the renamed row stays at its depth with the cursor on it. |
| `files_busy_folder` | panels | While a writer creates, modifies and deletes files in the shown folder: unchanged rows keep identity and content, cursor and selection stay, no loading state; a Rename and a Copy finished meanwhile are listed within 1.5 s; afterwards the listing equals the disk. |
| `panels_busy_ops` | panels | While a writer churns the shown folder: F7, Shift+F4 and Ctrl+F2 (octal 600, on Windows the Read-only attribute) succeed on disk (Ctrl+F2 on the two marked files, not the focused one), created entries are listed within 1.5 s with the cursor on them, nothing stays open, marks stay, an expanded branch stays expanded, and the listing equals the disk afterwards. |
| `panels_dialogs` | panels | Every panel dialog (F7, Shift+F4, F2, Ctrl+F2, Ctrl+F7, Ctrl+F12, Ctrl+=, Ctrl+−, Ctrl+M, Hot Paths manager, Alt+Enter, Ctrl+F1) opened by its key and by the pointer (toolbar, footer with its Ctrl layer, Hot Paths menu, F10) and closed with Escape, the header's × and Cancel/Close: one × and one Cancel/Close per dialog, no notice or window afterwards, Down moves the panel cursor (the keyboard is back in the list), the folder is unchanged; then each dialog does its job once, and Properties of three marked items counts their kinds and bytes. |
| `panels_notices` | panels | An unreadable folder at the end of a 400-character path, entered with Enter, a double click and Ctrl+G, a missing typed path and a failed inline branch: the notice's title, reason and folder, every line readable whole (no scrolling needed), Details and Hide details, Copy details (the clipboard starts with the title, reason and whole path, as the notice shows them), Escape, Close and ×; the panel keeps its listing without a duplicate banner; Retry lists the branch once the folder is readable; no notice from the background probe; touch density in a 900×640 window; a control proves the whole-path check fails in a 480×260 window. |
| `panels_times` | panels | Ctrl+F2 Times: Up/Down per date, time and offset part; the calendar (Alt+Down, arrows, Page Up/Down, Enter, Previous/Next month, a day, Today) and time picker (Alt+Down, arrows, Page Up/Down, Enter keeping the picked time, Escape restoring the time it opened with, ▲/▼); Now; the picked moment read back from disk to the second, Accessed untouched; Escape closes a picker only, the next Escape the editor without applying; Ctrl+F7 lowercase on the Case page; Include subfolders on a folder (every descendant gets the time) and Back to editing keeping the draft. |
| `panels_view_sort` | panels | Ctrl+F3–F6, every column header (both directions, its accessible state) and the sort menu (each column, Reverse order) give the exact order with folders first and extensionless files last, marks and cursor kept and in view; the right panel sorts on its own; View menu and Alt+2/3/5 keep cursor and marks, arrows move by column (Brief) and cell/row (Thumbnails); the splitter dragged, clamped at its minimum with names readable, double-clicked, moved with its keys, restored after a restart; Ctrl+F11 and Tab; a 600×480 window's rail; touch density. |
| `panels_paths` | panels | Toolbar Back/Forward (disabled without history), Parent, Folder sizes and Refresh; the Back and Forward history menus; Ctrl+PageUp, Ctrl+PageDown (a file is not opened), Shift+Backspace, Ctrl+Backslash; double clicks on a folder and `..`; Tab and Shift+Tab; path segments (hover highlight, right-click copy, paste with a separator), a long path's middle marker, the blank space after the path, Escape; Alt+F1, Alt+F2, `.` As Other Panel and the root by pointer; the details strip copying name, path and size; empty folders and everything hidden or filtered say so below `..`; a quick filter without matches. |
| `panels_tabs` | panels | Middle click (not on the last tab), a double click after the tabs, Ctrl+Shift+digit (a missing number does nothing), scroll arrows consistent with the strip and the active tab in view, the 32-tab limit, right-panel tabs, Ctrl+U moving tabs with their panel, F7/Shift+F4/F2 in second tabs of both panels. Restoring tabs after a restart is written but switched off (`RESTORE_TABS_READY`), so it is not checked here; `startup_paths` checks the restore. |
| `panels_search_filter` | panels | Filename search: Up/Down without wrapping, Home/End and Page Up/Down in match order, Backspace and an empty query, Space, Insert, Unicode without case, kept through sorting and Ctrl+R, ended by opening a folder. Panel filter: the funnel, Regex, Case sensitive, the indicator and its ×, the history menu, Help and F1 (Escape closes only the help), Cancel, an invalid regular expression; Ctrl+A, Ctrl+Shift+A, sorting and search in a filtered list, hidden files with a filter, each panel's own filter. |
| `panels_tree_keys` | tree | Inline branches: arrows, Home and End through nested rows; Insert and Space (measuring) on nested folders; Ctrl+Shift+I, Ctrl+A and Ctrl+D across levels; branch order by name both ways and by time; collapsing with the cursor inside; Enter on a nested folder and Backspace back; an Expression filter's context row cannot be marked. |
| `panels_selection_more` | selection | Ctrl+=, Ctrl++ and Ctrl+− with the remembered mask and Include folders, invalid masks (two `\|`, nothing after `\|`) that must change nothing and say why, Ctrl+Shift+click ranges, Alt+Down/Alt+Up over marked rows without wrapping, Insert and Space at the last row, `..` never marked, the rubber band from the gutter (plain, Shift, Ctrl); the details strip's counts and bytes after each step. |
| `panels_root_keys` | panels | Staged with the installed plugins (native browsing through the plugin): Shift+Backspace to the root from deep folders in the left and right panel (each focused in turn; Tab, a mouse click), Ctrl+Backspace, after a search ended with Escape (and during one), in a second tab (the first stays); inside bundle.zip/bundle/nested/deeper it goes to the archive's root, and the path bar shows the archive's and folder's names whole (a native folder is the control); Make File List to the Internal viewer shows exactly the list. |
| `panels_hot_digits` | panels | Physical digits: 1–9 and 0 (number 10), Shift+ and Ctrl+digit, Ctrl+Alt+ and Alt+Shift+digit into the other panel, an unassigned number swallowed, a digit during search stays text, Ctrl+Shift+0; the Hot Paths star (first free number), Add path beyond ten, the Hot Paths menu and the drive menu. |
| `panels_file_list` | panels | Ctrl+M: Clipboard output; FileNamePart, FileExtension, FileSize, `$$`, `:max` and `<DIR>`; Browse… through the scripted chooser; an existing file asks first, Cancel keeps it, Replace and Append do so and a notice names the recovery copy holding the original bytes; the focused entry; a panel filter limits and orders the list; the template history; every output byte for byte. |
| `tools_compare_dirs` | compare | Ctrl+F10 on identical, asymmetric, nested and unreadable trees with every dialog option (size, time, tolerance 0/1/2, exact case, unmatched folders, Compare contents, Include subfolders): exactly the expected rows become each panel's selection (same-sized files with different bytes on both sides; folders that contain differences on the differing side), the footer counts the pairs read and the folders with differences, the dialog closes at the click while the comparison still runs, no result message or window appears, the reopened dialog shows the summary, invalid tolerances are reported in the dialog. A long content comparison (two 24 GiB sparse files) shows its progress on the status line, refuses F7 with "Directory comparison running", shows its progress and Cancel in the reopened dialog, and Cancel ends it selecting nothing; the next comparison runs normally. |
| `tools_find_criteria` | find | Alt+F7 on a generated tree with known answers: masks (case, exclusions, `?`, several, folder names), Subfolders off, Regex (with case), Expression (size, size range, kind, date, date range, name and size, Linux attributes), Containing text (case, whole words, UTF-16, regular expressions with case and whole words, the same text literal), Modified within, Identical files; an invalid content regular expression fails with its reason on the details page; every results column resized by dragging. |
| `tools_find_folders_help` | find | Search folders: Add a typed folder, Browse… (scripted chooser, folder kind), tick boxes in the dropdown, choose and Remove, the nothing-ticked error and its details; syntax help for each mode, F1, an example fills the query without searching; an invalid expression fails with a "Click for more details" status line (tooltip) that opens the details page, no Details button. |
| `tools_find_results` | find | Every entry of `many/` (the virtualized list scrolled through with the wheel and compared with the disk), Sort by Name/Size, column resizing, Go to file with the button; 10,050 files stop at the 10,000-result limit (Incomplete, limit named in the details); Stop cancels a running content search; Search inside archives without an archive provider (none in this harness) is Incomplete with its reason in the details; Feed to panel lists the results over the left panel and closes the Find window, Insert marks, F5 copies the marked real files to the other panel, Shift+F8 deletes a result's file and removes its row, Backspace shows the folder again. |
| `find_query_syntax` | find | The query language with known answers: masks (`;;` literal, `*.*` with extensionless names and folders, several exclusions, an excluded folder still searched, Unicode letter case), Regex (`(?i)` against Match name case), Expression (`kB`/`KiB`/`B`, `=~`, `not (…)`, a UTC offset at an inclusive and an exclusive bound, `present`/`missing`); Containing text in UTF-8 with and without BOM and UTF-16 LE/BE with BOM (BOM-less UTF-16 not decoded), Unicode case, whole words at a combining mark and an underscore, per-line regular expressions with CRLF, malformed UTF-8 reported per file (also after a match); Modified within in minutes, hours and days (the future never, 366 days in each unit), invalid ages that disable Find, the collapsed options summary; Identical files (empty, small and binary groups, a hard-link pair counted once, groups together under Name and Size both ways, Search inside archives disabled). |
| `find_roots_windows` | find | Search folders: Add typed with Enter and OK, a listed folder added again (ticked, not doubled), relative and parent-step paths refused with the entry open, a missing folder (others searched, Incomplete, named), Remove, 16 folders at most; Subfolders on and off over three folders; Find from the right panel (its folder; Go to file there); Ctrl+F, the toolbar button and F10 › Find files…; four windows at most; searches in several windows at once with their own results, run again with the kept draft, a third search waiting while two run; the window is modeless and keeps its draft; Escape closes the Add entry, the help, then the window; Close during a search. |
| `find_result_actions` | find | Every column sorted both ways against known orders (folders first and missing extensions last under Extension, equal keys by path, a new search in the window's order), Name order equal to the panel's; Home/End/Up/Down/Page Down in the results; Go to file by Enter, double click and button, for files and folders, Open on a folder; View, F3 and Alt+F3 ask for a viewer of the file (recorded here; `find_viewers` opens them), Edit and F4 run a harmless configured editor, Open reaches the launcher (none here); deleted and changed results refused with the panel unmoved; Stop keeps 20 results sorted, the folder being read named only while the search runs, Feed to panel disabled before and during a search, a new search replaces the results. |
| `find_feed_panel` | find | Feed to panel: the Find window's order, title, status, Folder column; cursor keys, Alt+Right and Ctrl+R inert; Insert, Space, the bulk keys with their panel meaning, keypad * (as Ctrl+I, and as Ctrl+Shift+I once Select by mask last had Include folders, chosen in the right panel), Ctrl+click, Shift+click, Alt+Up/Down; Enter, Ctrl+Page Down and a double click to a file or folder; Backspace, Alt+Left, Ctrl/Shift+Backspace, Back to folder and Ctrl+G leave the list, Tab keeps it; F3, Alt+F3, F4 and folders; Alt+Enter; F5, F6 (the row stays), F8, Delete and Shift+Delete asked, Shift+F8 of a folder and its result; every refusal (keys, keypad, toolbar, a custom command, function bar, main menu) with the folder unchanged; the right panel. |
| `find_budgets` | find | Budgets named in the details of an Incomplete search: 128 folder levels (the files of 127 nested folders found), a sparse file over 1 GiB not read, an identical-files group over the 10,000-result limit omitted whole while a later group is listed, 100,000 duplicate candidates. |
| `find_layout` | find | The Find window in Blue, Dark and Touch with 24 px text at its preferred, a narrow and a compact size: the UI review's layout checks; narrow stacking (Remove under Add, Browse… and OK under the path), sideways column scrolling and shortened Path cells that keep their last folder; compact "Query" label, the query area hidden while searching, the idle status in the query area (by the area that clips it) and in view with large text; a column drag never sorts and its width is kept. |
| `find_archives` | find | Staged with the installed plugins: Search inside archives finds members of a ZIP, a tar, a tar.gz and a gzip stream by name (Unicode, member folders), content (literal, per-line regex), expression and Modified within (tar times only); a member archive is not opened; with Subfolders off an archive in the searched folder is still searched completely and one in an ordinary subfolder is not reached; two folders with archives at once; a broken archive is reported; Go to file opens the archive at the member and Backspace returns; View/Edit/Open refused for members; Feed to panel leaves members out; Find refused inside an archive; 300 ZIPs and one of 20,000 members searched completely; Stop while a panel opens an archive; 1,000,000 visited entries; an archive over 100,000 entries (one folder of 120,000 members, searched completely: neither the archive plugin, the plugin host nor the engine limits archive members). |
| `find_viewers` | find | Staged with the installed viewers: View, F3 and Alt+F3 on Find results open the selected file (Markdown rendered, or its source) and only that file (Space walks no further); F3 and Alt+F3 in a fed list open the focused result, and Space and Backspace walk the list's results. |
| `tools_checksum_algorithms` | checksums | The algorithm dropdown's tick boxes (SHA-256 alone by default): all five calculated at once, each digest checked against an independent implementation (SHA-1 and SHA-512 in `tools_support.rs`); in the results Ctrl+C copies the shown SHA-256, Ctrl+Shift+C the whole row, and Shift+F10 and a right click open the copy menu whose Copy MD5 copies the MD5 (the in-process clipboard's text compared); a manifest saved per algorithm with the suggested name and exact lines, the Replace prompt's Cancel keeps the file, Replace writes it, Escape then closes the window; Include subfolders; nothing starts before a button is pressed. |
| `tools_checksum_manifests` | checksums | Escape closes a window opened on the Verify page; the page tabs are Calculate and Verify; Verify preselected (and not started) for BSD SHA-512, GNU binary SHA-1 with a mismatch and a missing file, hash-only SHA-1 and MD5 sidecars, UTF-16 SHA-256 and SFV with a wrong CRC and a missing file; a manifest mixing algorithms fails and says why. |
| `tools_checksum_large_stop` | checksums | The large file and a 2 GiB sparse file with all algorithms: progress (bytes read growing), Stop, Cancelled with finished rows kept and no manifest written. |
| `tools_folder_sizes` | tree | Ctrl+Shift+S on marked `links/` (0 bytes: links inside are not followed), `readonly/` and `many/` only; marked folder links inside `links/` (measured as their targets); a folder with an unreadable subfolder (partial size, warning button); cancelling a running scan, then a complete one. |
| `tools_jobs_copy` | jobs | F5 of the large file and `many/`: the progress window, Pause (the transfer holds still), the Jobs window while paused (it should say Paused), Resume, Cancel operation with its confirmation (only whole files remain); the window stays open saying "Copy cancelled; files already copied are kept." and no notice appears while it is open or after it is closed; a second copy that completes and closes its window, Jobs listing Completed and Cancelled, Clear all finished. |
| `tools_help_drive_info` | tools | F1 keyboard help lists the tools' shortcuts, describes folder size scans correctly, and closes with Close and Escape; Ctrl+F1 drive information names the folder, volume, filesystem, total and free space and closes with Close. |
| `tools_main_menu` | tools | F10: the category picker narrows the list; every item of every category is clicked and its dialog, window, popup or effect checked, then cancelled or undone (nothing is confirmed, the folder on disk is unchanged); items without a harness service are recorded; Exit hides the main window. |
| `ui_review` | ui | The UI polish gate's default matrix: the main window's states, menus and popups, every embedded dialog and tool window across themes, both densities and every size class (each window's minimum size included) at scale 1, and the main window, one dialog, a notice and Find at scale 2, through every static and contrast check; any flag fails (see "UI review checks"). |
| `ui_themes` | ui | The polish gate in every theme the default matrix leaves out: the main window's states and popups, Copy, an error notice and Find. |
| `ui_polish_checks` | ui | Red cases: every polish check flags a deliberately broken fixture window and passes its corrected twin. |
| `ui_polish_dialogs` | ui | The polish gate's interactive checks on every dialog and tool window: Tab and Shift+Tab, visible focus, Escape, Enter, every button's effect, no extra window or notice, hover feedback and tooltips (private child process). |
| `ui_polish_jobs` | ui | The polish gate on Copy job windows in every state, the name-conflict and failure (Retry/Skip) decision windows and Jobs, at preferred and minimum size. |
| `ui_polish_viewers` | ui | The polish gate on viewer windows (text and source with a selection, image, Markdown, hex) and a comparator, dark and light (staged with plugins). |
| `ui_matrix_full` | ui | The polish gate's full matrix, with `--all-themes` or `MIDNIGHT_ACCEPTANCE_UI_MATRIX=full` only (`SKIP` otherwise). |
| `viewers_files` | viewers | F3 on 40 fixture files (every text encoding, CRLF, the 200,000-character line, the large log, empty and no-final-newline text, eleven source languages including CSS and a Makefile, Markdown, HTML with its local stylesheet and image, PDF, PNG/JPEG/GIF/WebP/SVG, archives and binaries in the hex view): each viewer shows the content (text, source colours, rendered Markdown, page and image pixels, hex bytes); truncated and zero-byte images give "This image is damaged or incomplete:" followed by the decoder's reason. Records each file's open latency, flags opens over 500 ms and fails opens over 10 s; opens a 9-byte text file and a 4 KiB binary 15 times each for the spread. The two HTML cases are skipped (listed in the scenario's message) only when the HTML viewer reports that the machine refuses its browser: AppArmor denies Chromium's sandbox (no `midnight-explorer-html-viewer` profile covers the stage) or no writable cgroup-v2 delegation exists (a container); any other viewer error fails. Then generated files in the right panel: every extension the Source viewer claims (Source mode with syntax colours; CSV and TSV with five column colours), `.markdown`, `.mdown`, `.mkd`, `.xhtml`, BMP, ICO, TIFF and `.jpeg`, and damaged files (garbage and empty PDFs, truncated GIF, WebP, BMP and TIFF and a broken SVG refused with "This … is damaged or incomplete" and the reason; a truncated PDF or JPEG shown as far as it decodes or refused; invalid UTF-8 and UTF-16 shown as `\xFF`/`\uD800` escapes; NUL bytes never raw; broken Markdown, HTML and JSON shown). Every viewer window gets the UI review's layout checks (`viewers_probe::review`). |
| `viewers_controls` | viewers | Status bar; Find (Ctrl+F, Enter, F3, Shift+F3) with the find bar's Aa (case sensitive); Wrap (Ctrl+W and its box) on the long line; the Font and presentation row; text and binary modes from the mode picker; Alt+F3 plain text of Markdown; PageDown, Ctrl+End, Ctrl+Home and the mouse wheel in the large log; image zoom (Ctrl+1, Ctrl+Plus, Ctrl+Minus, the buttons, Ctrl+wheel, Fit); PDF layout; Space/Backspace to the next and previous file; Escape. Alt+Right and Alt+Left as aliases of Space and Backspace, and Backspace on the first file wrapping to the last. |
| `viewers_compare` | viewers | Insert marks two files, Ctrl+Shift+C compares them: identical text, three differences (N, F7, P, Alt+End, Alt+Home, Next difference), a 3.5 MiB log, the same text as UTF-8 and UTF-16, images in Side by side, Swipe and Onion skin; binaries in the hex view (summary of 3 differing bytes in 3 regions, the same keys and button between them); the 4.25 MiB log as text (its appended lines at Alt+End) and as bytes with the Hex button; Escape and Close. Also every other difference key (Shift+F7, Alt+Down, Alt+Up, Alt+Right, Alt+Left) and Previous difference; Find in both files (Find… and Ctrl+F, Enter, F3, Shift+F3, Shift+Enter, "Match N of M · File B · line L", Aa, Whole words, Clear Find); the font chooser; scrolling (an arrow moves one row, PageDown the visible rows, a synthetic 60 px wheel delta exactly 60 px, the position bar); the image sliders, dragging the divider, Zoom in and out, Ctrl+wheel and Fit; text over 32 MiB, 1,000,000 lines or with a 17 KiB line compared as bytes with the reason; File A / File B by hand (the panel's side prefilled, a missing path refused in the form with drafts kept, Browse… with the scripted chooser, Compare, Cancel). |
| `viewers_thumbnails` | viewers | Alt+5 thumbnail view of 60 coloured PNGs: each visible tile shows its own colour, no tile vanishes and returns while the view is still, End and the wheel scroll to tiles that load, Home shows cached tiles at once; PNG, JPEG, GIF, WebP and SVG thumbnails required, broken images (placeholders) recorded; `docs/report.pdf` must show its first page: a white page as wide as the thumbnail, with dark text. `SKIP` only when the stage has no thumbnailers package; with one, no thumbnail at all fails. BMP, ICO and TIFF in a generated folder in the right panel get thumbnails too; AVIF is not covered yet (licensed AVIF fixtures exist in `plugins/image-decoder/tests/fixtures/`). |
| `viewers_video` | viewers | Video as an installation runs it: the thumbnail and playback helpers start from the stage's `thumbnailers/` beside the FFmpeg libraries, in the restricted workers' sandbox. A generated MP4 (20 uncompressed 64 × 48 frames, magenta then green) and, on Windows, an NTFS-compressed copy, plus on Windows an H.264 + AAC MP4 that Windows' own encoders write at test time (the bundled profile has no H.264 decoder, so it plays and gets its thumbnail through the Media Foundation helpers) and its NTFS-compressed copy: Alt+5 must show each video's colour, not the fallback icon; F3 must open the first frame, a seek to 75 % show the green frames and a seek back the magenta ones, and Play run to the end. A helper that cannot start fails with the viewer's sentence ("The video helper stopped: a required library is missing or cannot be read (STATUS_DLL_NOT_FOUND, 0xC0000135)"), never a system dialog. `SKIP` when the stage has no video thumbnailer. The stage must grant the restricted workers' AppContainer read and execute on the copied FFmpeg libraries and both playback helpers as `scripts/deploy-private.ps1` does; without that grant the helpers cannot load them, as in an installation deployed with the wrong SID. |
| `viewers_archives` | viewers | Enter into a ZIP, a tar, a tar.gz and an empty ZIP: every folder's listing equals the members; F3 on members; F5 copies a member out byte for byte; Enter on a ZIP inside a ZIP opens it: its listing, the nested path in the path bar, F3 and F5 (byte for byte) on its member, then Backspace to the outer archive with the inner archive under the cursor and on to the native folder; Enter on a text file named `.zip` gives a clear error. A second start browses generated archives: `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`, `.tbz2`, `.tbz`, `.tar.zst`, `.tzst`, 7-Zip and a really deflated ZIP (made with the host's `xz`, `bzip2`, `zstd`, `7z` and `zip`; a missing program skips that format), each walked like the fixture's with F3, Space to the next member and F5; raw `.gz`, `.xz`, `.bz2` and `.zst` streams; 7-Zip ⊃ tar.gz ⊃ ZIP entered and left level by level with the path bar checked; the four-level limit explained; a truncated ZIP and 7-Zip, garbage after a 7-Zip signature and a corrupted tar.gz refused clearly; a member with a wrong CRC whose F3 and F5 fail clearly and leave no file; Compare files on two members (the File A / File B form); 100,100 entries, 8.9 MB of names and one folder of 100,001 files browsed. |
| `viewers_scroll` | viewers | Every viewer kind scrolled as a person scrolls: the scrollbar dragged to every 1 % (large log, CSV, TSV, CSV as plain text, Markdown with tables) or 2 % (Rust source, hex view, a 24-page PDF in the Continuous layout, an 800×3000 image at actual pixels), each drag landing without an error status, the thumb and the shown content agreeing, a text page holding the byte the position names; the scrollbar's thumb drawn when the content is taller; a synthetic wheel delta of 60 px down and up and a burst of five (300 px), each moving exactly its pixels, the same in every viewer (measured from the rendered pixels; a synthetic delta is not the product's notch, which only native wheel events carry); Down and Up 28 px, PageDown and PageUp one screen, Ctrl+End and Ctrl+Home to the ends; HTML keys (Chromium's own); the long line's horizontal scrollbar; HTML wheel travel (skipped: the browser surface takes the wheel only as a native window event); kinetic coasting skipped (no touchpad phases). |
| `viewers_selection` | viewers | Ctrl+A selects the page's rows and Ctrl+C copies exactly them (a prefix of the file for text and source, without line numbers; rendered text for Markdown; hex rows for the hex view); a mouse drag and a Shift+click select runs of the file's text. In every theme (a fresh start each), selected text keeps at least 4.5:1 contrast (WCAG) against the selection painted over the content: Rust source and CSV columns in all themes, plain text, Markdown, the hex view, Python, JSON, XML, YAML, CSS and SQL in Blue, Dark and Light; unselected text colours below 3:1 are flagged. |
| `viewers_presentation` | viewers | The Find button, Enter, F3, Shift+F3 and the Next and Previous match buttons walk four matches in order (the highlight moves line by line and column by column), "No further match.", Whole words, Aa, Clear find text, Escape from the search field; Find in the hex view and in the HTML viewer; Source ↔ Text and Markdown ↔ Text modes; Lines off (gutter gone, kept for the next source file); the Markdown presets; the font size + button and family picker (kept for the next text file); PDF pages with Ctrl+PageUp/PageDown, the page buttons and counter; image 1:1, Ctrl+0 and dragging the picture; HTML zoom buttons, Ctrl+Plus, Ctrl+0 and the Online and JavaScript boxes. |
| `viewers_combinations` | viewers | F3 on a result in the Find window and in its results fed to a panel (Space and Backspace follow the results; Compare files refused with the reason; Backspace leaves the list); F3 while an inline branch is expanded (Space follows the visible files); four viewer windows at once (a fifth refused with the reason, closing one keeps the others); a theme previewed in Options recolouring an open viewer and Cancel restoring it; a viewer moved to twice the scale. |
| `options_general` | options | Options › General: each check box and the disk I/O picker with its visible effect (byte sizes in the panel, filtered type-to-search, the quick-verification note), Clear command history; Cancel discards a previewed change, OK saves, a restart shows the saved values (a cleared, unremembered history recalls nothing with Alt+Up), Reset to defaults restores them and keeps the tabs; Escape discards a preview like Cancel, Enter saves like OK. |
| `options_appearance` | options | Options › Appearance: Light/Dark theme (panel luma), desktop and touch font sizes and touch spacing (row heights), font family (rendered text changes), text rendering (stored and shown after a restart; it configures the FemtoVG renderer, not the software renderer used here); Cancel restores the previewed theme, OK saves, a restart shows the same appearance, Reset to defaults restores it. |
| `options_custom_commands` | options | Custom Commands: create grouped (`${selected.paths}`), per-item (`${file.name}`), separator and failing items; drag to reorder, rename, delete; toolbar runs verified on disk; a missing program's notice; Preview starts nothing; Cancel discards; the keyboard editor's `user-command.<id>` bound to a key runs it before and after a restart; F9 opens the User menu. |
| `options_keyboard` | options | Keyboard editor: the capture field records Tab and Escape instead of running them; Ctrl+Space (kept for the input method) and a fifth shortcut are refused with a reason, and Cancel discards the session; an occupied key names its owner and asks before replacing (declined; accepted then cancelled with Options); Create folder rebound from F7: the new key opens it, F7 does not, also after a restart; Restore command brings F7 back. |
| `options_colors` | options | Colors: an added rule colours matching names in the panel (row colour and rendered pixels); first match wins until the rule is moved up; Cancel keeps a removed rule; OK saves; a restart keeps the colours, which stay on their rows sorted by size (Ctrl+F6) and filtered to `*.dat` (Ctrl+F12). |
| `options_archives` | options | Archives: custom extension chips per format (added, case-folded, removed), invalid input and built-in suffixes refused with reasons, a built-in association; Cancel, OK, restart; Escape in the extension field. Browsing a custom extension needs the native filesystem plugin and is not covered. |
| `options_editors` | options | Editors: a custom fallback editor and a `*.md` rule (`/bin/cp {file} <work>`); F4 runs the matching one, verified on disk, also after a restart; a rule without `{file}` keeps Options open with its reason. |
| `options_comparators` | options | Comparators: built-in text and external-program overrides (argument help, never run) saved and listed after a restart; an invalid regular expression keeps Options open with its reason. |
| `options_previews_plugins` | options | Previews: thumbnail size seen in the Thumbnails view's rows, the dependent switches disabled without previews, Cancel, OK, restart. Plugins: the sections without staged plugins (their empty-state texts are recorded, not asserted), Rescan, the Archive associations link. |
| `options_hot_paths` | options | Hot Paths manager: add three folders, give one an occupied number (exchanged), remove one (folder kept); Ctrl+digit opens them, the removed number does nothing, also after a restart. |
| `options_visual` | options | Every Options page in Light and Dark (1340×820) and a narrow touch layout (720×560, topic picker), through the UI polish gate's checks; any flag fails. |
| `options_themes` | options | Every theme the Appearance picker offers, chosen in turn: the theme and the panels behind Options change, each theme renders differently from every other, themes named Light are bright and Dark ones dark; OK saves the last, a restart keeps it. In a narrow touch window (720×560) the topic picker replaces the page list: a theme chosen and saved through it, a cancelled check box not saved. |
| `options_buttons` | options | Every enabled button on every Options page, clicked on a freshly opened page with a Custom Command and command history seeded, must change something a user can see (a text, a control's value or state, a popup, a window, a notice, the file chooser or a command to the backend); Escape closes Options and nothing is ever saved. Footer buttons are not clicked here (Reset to defaults is covered by the page scenarios; Export and Import by no scenario yet); Clear thumbnail cache is not clicked (its service exists only in staged viewer processes and would clear a real cache). |
| `shortcuts_catalog` | keyboard | Every default binding of the shortcut catalog, alternates included, pressed in the context its scope names (cursor row, marks, filename search, command editor, drives popup, seeded tabs, Hot Paths, history or Brief view as each needs): the action's own effect (folder, cursor, marks, dialog, window, popup, menu, notice, sort, view, tabs, Hot Paths, in-process clipboard, command draft) and no other change of the whole visible state; a catalog action without a probe fails. Ctrl+F8 is pressed on the parent row with nothing marked, as right after opening a folder, and the Checksums window must open on Calculate with Calculate enabled, not on an empty Verify page. Number-row digits, the keypad's + − * and modified brackets go through the window's physical-shortcut entry as the native hook delivers them. Keys outside their context: Space, a digit and Insert in filename search; F7, Ctrl+A and a digit in the command editor; the number row's + − * and an uppercase letter typed into a search. Enter in the command editor reports "Terminal unavailable" (no desktop services here). Viewer and comparison keys and F3, Alt+F3 and Ctrl+Shift+C need staged viewer processes (`viewers_*`). |
| `shortcuts_footer` | keyboard | The F1–F12 footer in the plain, Ctrl, Shift, Alt and Ctrl+Shift layers (modifiers held): every cell's click does what its key does (the same probe as `shortcuts_catalog`), cells without an action are disabled and do nothing, the network cells are hidden on Linux. |
| `shortcuts_hints` | keyboard | F1 keyboard help lists every catalog action with all of its keys (or "Unassigned"); the F10 main menu shows each item's key (Ctrl+= beside Select by mask, Num * beside the invert, nothing beside Options); after Create folder is rebound to Ctrl+Alt+K in Options › Keyboard the menu, the help and the footer follow at once (F7's cell unassigned and disabled), the new key opens the dialog and F7 does not. |
| `about_box` | tools | About from the toolbar's application icon: the version in the title strip and the Version row, at least 6 px between the About tab's lines, Left/Right between the tabs; Licenses: the count matches the list, rows set the pointer cursor, hover changes a row and shows the "›" mark on it alone, a click selects (the mark stays, View license enables); View license, Enter, a double-click and a click on the mark open the detail (title, License, Used by, authors, text); Back and Escape return with row and filter kept; Up, Down, Page Down, End, Home; the filter ("K of N", only matches, no match, Down into the list, Enter opens the first); reopening keeps filter and selection; Escape in the list, Close and a click outside close About; dragging the sky moves it. The scenario sets the application version as `main.rs` does. |
| `tools_user_menu` | tools | F9 with seeded Custom Commands: the items in order (built-in actions, the commands with the separator as a rule and the disabled command disabled, Close); Command line, Bookmarks and Hot Paths, Properties, folder sizes, Checksums and Close each do what they say; Tab then Enter runs the first item, Escape and a click outside close it; the grouped and the per-item command run on three marked files and on a folder shown only in an expanded tree branch, verified on disk; the disabled command runs nothing; without commands the empty state's Set up opens Options › Custom Commands. |
| `tools_notices_jobs` | tools | Notices: entering an unreadable folder shows "Cannot open folder" with the folder; Details expands the diagnostic details and Hide details collapses them; Copy details sends the message to the in-process clipboard ("Copied to clipboard"), starting with the notice's title, reason and folder; Close, Escape and the header's close button each close it and no other notice follows within a second; a 1,500-character folder path keeps Close in view and its box scrolls to the end. Jobs: create, rename, delete and a failed create listed with their states and count; a click and Up/Down select; Open and Enter show the job's window; Copy details; Cancel job disabled for finished jobs; Clear finished without errors keeps the failed job, Clear all finished leaves "No background operations"; Close and Escape close it. |

Registered, not written yet (each reports `SKIP: not written yet`): `panels_scrolling`,
`files_links_volumes`, `tools_checksum_unlimited`, `viewers_passwords`, `viewers_office`,
`options_settings_file` and `options_palette`.

`scripts/acceptance.sh --list` prints the current list. The Find scenarios' coverage
record, item by item with its status, is [acceptance/find.md](acceptance/find.md); the
Options and keyboard record is [acceptance/options-tools.md](acceptance/options-tools.md),
and the tools record (menus, Checksums, Help, Drive information, About, Jobs and
notices) is [acceptance/tools.md](acceptance/tools.md).

### UI review checks

The UI polish gate checks every rendered window, dialog, popup and state
automatically and strictly. Its coverage record lists every check with the red case
that proves it can fail, every surface and state with its status, the exemptions and
the product defects found: [acceptance/ui-polish.md](acceptance/ui-polish.md). The
checks are `scenarios/ui_gate.rs` (static and contrast) and `scenarios/ui_interact.rs`
(interactive); `inspect::check_layout` keeps the original layout checks, which other
scenarios also call through `options_support::review_shot`.

Static checks run on every capture: the window's item tree, or only its open popups
while one is open (the window beneath was checked without it), and the software-rendered
frame of the same moment:

- the layout checks of `inspect::check_layout`: **control clipped by window**,
  **overlapping controls**, **zero-size label**, **label elided to nothing** and **text
  does not fit its button** (file system paths are exempt: they are elided in the
  middle by design and carry the full path in their tooltip and accessible label; a
  text whose own box holds it, cut at the edge of a view that scrolls sideways, such as
  a table's last column, is scrolled out of view, not elided);
- **internal close button in a framed window**: an icon-only Close, Close dialog,
  Cancel dialog, Dismiss or Decide later button in the top band of a tool window whose
  Window element is framed;
- **overflow without scroll bar**: a viewport whose content does not fit shows no
  scroll bar thumb beside it and does not scroll when the wheel turns over it (the probe
  scrolls it back before the frame is taken);
- **content cut off without scrolling**: text or a control reaches outside a clipping
  container that does not scroll;
- **text clipped without ellipsis**, **wrapped text cut off**, and **elided text without
  full text** (neither its accessible label, an accessible ancestor's label or
  description, nor a tooltip holds the full text);
- **control without accessible name** and **icon-only button without tooltip** (a list
  row scrolled to a sliver at its viewport's edge is not icon-only);
- **overlapping text** and **text outside the window** (glyph areas, placed by each
  text's alignment);
- **uneven button heights** and **uneven button spacing** within a row, and **dismiss
  button not last** in a dialog's footer;
- **surface shows no text**;
- **low text contrast**: WCAG 2.2 AA in the rendered pixels. Every visible text is
  measured against each background colour that covers at least 15 % of its glyph area:
  4.5:1, or 3:1 for large text (24 px, or 18.67 px bold) and symbols; a text field's
  selected text is measured with its selection colours; text of disabled controls and
  text scrolled to a sliver at its viewport's edge (less than half of its height in
  view) are exempt (WCAG 1.4.3).

Interactive checks (`ui_polish_dialogs`, `ui_polish_jobs`): Tab from the dialog's first
focus reaches every enabled control (one member of a tab or radio group reaches the
group) and never leaves the dialog, Shift+Tab visits the stops backwards, every stop
looks focused with an indicator of at least 3:1 (WCAG 1.4.11), compared with the frame
before the Tab that reached it (a Tab that leaves the focus where it was, as in a dialog
whose only stop already has it, leaves nothing to compare); Escape closes the dialog;
Enter in a freshly opened dialog has an observable effect; every button and tab, each
clicked in a freshly opened dialog on freshly restored scratch folders, has an
observable effect (windows, notices, popups, texts, control states, the folders on disk,
commands to the backend, file-chooser requests, jobs, the panels, copies to the
clipboard, or the still window's pixels outside the clicked and focused controls;
changes between two quiet samples before the click do not count, and a surface still
changing is sampled again, for up to 2 s, until two samples agree); buttons that clear a
list (Jobs' Clear finished without errors, then Clear all finished) are clicked after
the others, which reopen on the same list; no window or notice appears after Escape,
Cancel, Close, the header's close or Decide later, nor after a job ends; hovering a
button, tab, check box, radio button, combo box or switch changes how it looks; hovering
an icon-only button with a tooltip area shows tooltip text. The main window gets the
hover and tooltip checks; Tab switches its panels by design.

The gate is strict: a gate scenario fails on any flag, with or without
`--strict-review`, unless the flag is an exemption listed with its reason in
`ui_gate::EXEMPTIONS` and the coverage record. Flags of checks whose product change is
still in progress (`ui_gate::PENDING`, also listed there) make the scenario `PENDING`
when nothing else is flagged. A step that fails (a dialog that does not open) is a
`dialog step failed` or `job step failed` flag and the sweep continues.
`--strict-review` keeps its meaning for the review flags of other scenarios.

| Scenario | What the gate covers |
| --- | --- |
| `ui_polish_checks` | Red cases: each check on a deliberately broken window (`slint!` fixtures) and its corrected twin. |
| `ui_review` | The default matrix (below): the main window with the cursor, marked rows and a filename search; the F10, F9 and Alt+F1 popups; every Options page; Copy, Move, Delete to Trash, Permanent delete, Rename, Create directory, New empty file, Compare directories, Panel filter, Make File List, Attributes and Times, Hot Paths, About (both tabs and a license's detail view), Drive information, Properties, an error notice naming a long path with its details shown; Find, Checksums, Help and Jobs. |
| `ui_themes` | Every theme the default matrix leaves out: the main window's states and popups, Copy, the error notice and Find. |
| `ui_polish_dialogs` | The interactive checks on every dialog and tool window above, in a private child process (`viewers_stage::run_private`): buttons are really pressed, so Delete to Trash uses a private Trash. |
| `ui_polish_jobs` | Copy job windows paused, waiting, running, with details shown, asking to cancel, cancelled, completed, completed with errors (3 and 40) and with notes; the name-conflict and the failure (Retry/Skip) decision windows; Jobs listing them; each at its preferred and minimum size, Blue/Desktop and Dark/Touch. |
| `ui_polish_viewers` | Viewer windows (text and source with everything selected, image, Markdown, hex) and a comparator, Dark/Desktop and Light/Touch, staged with the installed plugins. |
| `options_visual` | Every Options page in Light and Dark (1340×820) and narrow touch (720×560). |
| `ui_matrix_full` | The full matrix; runs with `--all-themes` or `MIDNIGHT_ACCEPTANCE_UI_MATRIX=full`, `SKIP` otherwise. |

The default matrix (`review::default_matrix`) starts the application once per
combination (the first loaded preferences choose theme and density). At scale 1, with
every surface: Blue/Desktop 1340×820, Dark/Touch 960×600, Light/Desktop 1600×1000,
HighContrastDark/Touch 1340×820, and Blue/Desktop and Dark/Touch at the minimum size
(the main window 320×200, each other window its own minimum). At scale 2, in the colour
scope (the main window's states and popups, one dialog, a notice and Find):
Light/Desktop 1340×820 and HighContrastLight/Touch 960×600. Tool windows are sized from their own layout for the combination's
size class: their minimum size, three quarters of their preferred size, their preferred
size, or 1.4 times it. The full matrix adds every theme in both densities at 1340×820
and the base themes (Blue, Dark, Light and both high-contrast themes) in both densities
at every size class and both scale factors. Every dialog is cancelled with Escape in
the sweeps, which work on a scratch copy of `checksums/` with an unreadable folder for
the error notice.

### Viewers, comparators, thumbnails and archives

Plugin discovery is relative to the running executable: the viewer engine, the
Engine's archive catalogue, the comparator's image viewers and the thumbnail host look
for `viewers/`, `filesystems/` and `thumbnailers/` beside it. The packages are never
put beside the shared test binary, where other runs would discover them. Instead each
`viewers_*` scenario (`tests/acceptance/viewers_stage.rs`):

1. stages a private installation in a new folder
   `<build>/acceptance-stage-<run>-<scenario>/`: a hard link to the test executable with
   the build's packages beside it (`_build/debug/{viewers,thumbnailers}` and every
   `filesystems/` package except the network providers, or the folder named by
   `MIDNIGHT_ACCEPTANCE_PLUGINS`). `<build>` is the folder `MIDNIGHT_ACCEPTANCE_STAGE_ROOT`
   names, else the `_build` folder holding the packages (`_build/debug`), else the test
   executable's build folder. A run therefore writes beside the packages it stages:
   packages that another checkout or package owns (a shared build used read-only) need a
   folder of your own in `MIDNIGHT_ACCEPTANCE_STAGE_ROOT`, or a copy of the packages in
   your own `_build`. `scripts/acceptance.sh` names the checkout's `_build` itself when
   the packages are outside the checkout and the variable is not set (`--dry-run` prints
   `Stages:`). A source checkout's HTML-viewer AppArmor profile
   (`sudo scripts/install-html-viewer-apparmor.sh`, which covers
   `<checkout>/_build/*/viewers/midnight-viewer-html/midnight-viewer-html`) therefore
   covers the stage as it covers a build. Files that already have every owner
   permission (executables and libraries, most of the bytes) are hard-linked, the rest
   (about 115 MiB) copied, so deleting a stage cannot change the build's own files. Its
   folders are mode 0700: plugin trust checks reject group- or world-writable
   ancestors. Without packages the scenario reports `SKIP`.
2. runs that executable as a child process for just this scenario
   (`MIDNIGHT_ACCEPTANCE_STAGED`), on its own fixture of the same scale and seed in a
   run folder it creates in `work/<scenario>/staged/run`, with its console in
   `work/<scenario>/staged/console.log`. The child's `XDG_CACHE_HOME`,
   `XDG_CONFIG_HOME` and `XDG_DATA_HOME` are private folders in the stage (`xdg/`), so
   Chromium's sessions, the archive spool and anything else the plugins keep there never
   reach the account's own folders. The child gets its own process group.
3. adopts the child's status, message, notes, screenshots and flags
   (`staged-result.tsv`) and deletes the stage. The parent kills a hung child's process
   group 20 s before the scenario's timeout, counted from the scenario's start as the
   runner counts it (the child's own watchdog reports and kills its group 25 s before
   that), and still adopts its screenshots, so a plugin crash or hang fails one scenario,
   not the run. After a child exits, whatever it left in its process group is killed.
   A staged scenario whose thread has not reported by its timeout gets another 60 s to
   come back (its child is stopped by then); if it does, it is reported `TIMEOUT` with
   its own result in the message and the run goes on. When the runner gives up on a
   scenario, it kills every staged child's process group and removes every stage
   itself, since the scenario's thread never returns; a child whose parent is gone
   (Ctrl+C) sees its standard input, a pipe the parent never writes, end, removes its
   stage and kills its own group.

`files_delete` runs the same way without a stage (`viewers_stage::run_private`): a
child of the test executable itself whose private `XDG_*` folders are in
`work/files_delete/staged/xdg/`, so the application's home Trash
(`$XDG_DATA_HOME/Trash`) is a private folder on the fixture's filesystem.

In the child, `Config::plugins` makes `App::start` compose what
`crates/midnight-explorer/src/main.rs` does (`tests/acceptance/viewers_support.rs`):
`Engine::with_shared_native_filesystem` on the installed native plugin with archive
capture through it; the viewer service and `ViewerWindows` behind F3 and Alt+F3, with
local resources (stylesheets, images) from a prepared parent folder; the comparison
service and `ComparatorWindows` behind Compare; the content-appearance bridge; the image
spare and the preference restore once the settings are applied; and a 100 ms thumbnail
pulse. The application's own modules cannot be included by path (their unit-test
modules need the explorer crate), so these are mirrored, with deliberate differences:
viewer resources have no parent-folder pin cache, external comparator programs are not
installed, and the thumbnail pulse presents native files only (no archive members,
command artwork, animation or upload retries) with its cache in the scenario's folder.
A viewer or comparator window gets `WindowActiveChanged(true)` when it opens, as a
desktop activates a new window; without an event-loop proxy, their own timers deliver
content (the viewer's 16 ms timer, the comparator's polling).

Open latency is the time from the F3 key event to the viewer's own report of its first
content frame (`open_with_ready`), split into window creation (inside the key event)
and the backend's first frame; only timers run meanwhile (no main-window frames). It is
measured in the debug test build with Slint's software renderer, so it is an upper bound
for a release build. Opens over 500 ms are flagged `slow viewer open`; the
`latency` notes of the report list every file.

Native viewer input (the notch conversion by the system's lines per notch, touchpad
reports and their momentum, reduced motion, the inactive selection colours) can be
driven through `midnight_viewer_ui::acceptance_seams` ("What it drives"); no scenario
uses it yet, so kinetic (touchpad) scrolling is not covered. The application has no
quick-view or preview panel; the thumbnail view is the closest feature.

## Fixture catalogue

`tests/acceptance/fixtures.rs` writes the tree below `<run>/fixture` from a fixed seed
(`SEED`); the same scale gives byte-identical files. All formats are produced in
process by `tests/acceptance/encode.rs` from their specifications (no new
dependencies): stored-deflate PNG/gzip/ZIP, LZW GIF, baseline JPEG, lossless WebP,
PDF with an exact cross-reference table, ustar, and SHA-256/MD5/CRC-32 for the
manifests (with known-answer self-tests).

| Folder | Contents |
| --- | --- |
| `text/` | ASCII, UTF-8, UTF-8 BOM, UTF-16 LE/BE (BOM), CRLF, a 200,000-character line, a large log (1 or 8 MiB), an empty file, no final newline. |
| `code/` | Rust, Python, JavaScript, CSS, SQL, shell, Makefile, JSON, XML, TOML, YAML. |
| `docs/` | Markdown (headings, lists, code, table, quote, link), HTML with a local stylesheet and image below its folder (and a script, and a parent-relative `../` image that the HTML viewer denies by design), minimal HTML, a one-page PDF. |
| `images/` | PNG gradient, greyscale JPEG, palette GIF, lossless WebP, SVG, a truncated PNG, a zero-byte `.png`. |
| `archives/` | Stored ZIP with nested and empty folders and a Unicode member, ustar and tar.gz of the same tree, a ZIP inside a ZIP, an empty ZIP, a text file named `.zip`. |
| `sizes/` | Zero, one and 4096 bytes, and a large random file (2 or 24 MiB). |
| `deep/` | `level-01/…/level-12`, one file per level. |
| `many/` | 400 or 3,000 small files (`item-NNNNN.{dat,txt,log,bin}`) and ten folders. |
| `names/` | Spaces (leading, trailing before the extension), leading dot and dash, Latin/Greek/Hebrew/Japanese, emoji, NFD combining and NFC precomposed forms, upper/lower case, shell punctuation, no extension, many dots, a 224-byte name, hidden folder, folder with spaces. |
| `links/` | Symbolic links to a folder, to a file, dangling, and to its own folder (a cycle); Unix only (empty on Windows, where `files_links` makes junctions in its work folder). |
| `checksums/` | Four data files (one empty, one name with a space) with valid `.sha256`, `.md5` and `.sfv` manifests, one with a wrong hash and one naming a missing file. |
| `readonly/` | A mode-0444 file and a mode-0555 folder (Windows: the file with the read-only attribute, no folder). |
| `compare/` | Comparator pairs: identical text; three differences (a changed, an inserted and a removed line); a 3.5 MiB log pair differing in two lines and a 4.25 MiB log over the comparator's former 4 MiB text limit; the same text as UTF-8 and UTF-16 LE; two 64×64 PNGs differing in a red square; two binaries differing in three bytes. |
| `thumbs/` | 60 PNGs of 96×72 pixels, one colour each with a white diagonal, a one-page PDF and a JPEG. |

`compare/` and `thumbs/` come from their own seed (`viewer_extras`), so every file
generated before them is byte-identical to earlier runs.

`index.html` lists every generated entry with its purpose.

### Scenario-local content of the tools scenarios

The `tools_*` scenarios leave the fixture unchanged and build what they need in their
own work folder (`work/<scenario>/`), so their expectations are exact at either scale:

| Scenario | Generated content |
| --- | --- |
| `tools_compare_dirs` | `compare/identical/{left,right}` with equal names, sizes and times; `compare/asymmetric/{left,right}` with same-named pairs differing in size, in time (100 s, 2 s, 1 s) or only in letter case, one-sided files and folders, and a shared folder whose contents differ two levels deep; `compare/unreadable/left` with mode 0444 (listable, metadata unreadable); `compare/special/{left,right}` with a dangling link opposite a file of the same name. Times are fixed (from 2023-11-14). `compare/slow/{left,right}` with a 24 GiB sparse file each (same size and time). |
| `tools_find_criteria`, `tools_find_folders_help` | `findtree/`: 16 files and 3 folders with known names, sizes (2 KiB, three 18-byte `.dat` files of which two are identical), contents (`quick` in several cases, `quickly`, UTF-16 LE) and times (`old.txt` from 2020), a dot-file and a mode-0444 file; `second/` and `third/` as extra search folders. |
| `tools_find_results` | `limit/` with 10,050 empty files; `many-search/`, a copy of `many/` plus a 1 GiB sparse text file; `feed/` with four small files in three folders and a placeholder `bundle.zip`, and an empty `feed-target/`. |
| `find_query_syntax` | `query/`: `names/` (extensionless, `;`, Unicode and case variants, excluded types and folder), `sizes/` (1000, 1010 and 1024 bytes, a 2021 file), `text/` (every supported encoding, BOM-less UTF-16, CRLF, combining mark, underscore), `bad/` (two malformed UTF-8 files), `times/` (30 minutes to 400 days old and one in the future), `dupes/` (three identical groups, a hard-link pair, a same-size different file). |
| `find_roots_windows` | `roots/a`, `b`, `c` (a file at two levels each), `extra/r00`…`r15`, `slow/` with a 1 GiB sparse file. |
| `find_result_actions` | `act/` (six files of known sizes and 2022 times, a folder with an extension), `collate/` (mixed-case, accented and numbered names), `stop/early` (20 matching files) and `stop/late` (two 1 GiB sparse files), `edited/` for the harmless editor. |
| `find_feed_panel` | `feed/` (five files in three folders and a subfolder), `feed-target/`, `edited/` for the harmless editor and the custom command's marker. |
| `find_budgets` | `budgets/`: 130 nested folders with a file each, a 1 GiB + 1 byte sparse file, 10,001 empty files and an identical pair, 100,001 sparse files of different sizes. |
| `find_layout` | `layout/`: 40 files with long names in four folders, `slow/` with a 1 GiB sparse file. |
| `find_archives` | `arch/` (a ZIP with a Unicode member and a member ZIP, a tar retimed to an hour ago and its tar.gz, a gzip stream), `arch-broken/`, `arch-folders/` (a ZIP at the top and one in a subfolder), `arch-many/` (300 ZIPs and one of 20,000 members), `arch-nodes/` (one 99,910-entry ZIP64 and ten hard links), `arch-big/` (a ZIP64 of 120,000 members). |
| `find_viewers` | `view/`: a Markdown file and three text files, one in a subfolder. |
| `tools_checksum_*` | Copies of `checksums/` with a `nested/` folder and generated BSD, GNU binary, sidecar, UTF-16, SFV and mixed manifests; `large/` with a hard link to `sizes/huge.bin` and a 2 GiB sparse file. |
| `tools_folder_sizes` | `sizes-warning/` with a mode-000 subfolder; `sizes-cancel/` with `bulk/` (100 folders of 1,000 empty files) and two small folders. |
| `tools_jobs_copy` | `source/` with a hard link to `sizes/huge.bin` and a copy of `many/`; empty `target/` and `second/`. |
| `tools_main_menu` | `menu/` (three files and a folder) and `menu-right/`. |
| `tools_user_menu` | `user-menu/panel` (three files, `project/sub-a` and `project/sub-b`) and `user-menu/out`, where the seeded commands (`/bin/sh` with a literal script, `touch`) write. |
| `tools_notices_jobs` | `notices/panel` with a mode-000 `locked/` and a seven-level chain of 190-character folder names ending in a mode-000 folder; `notices/other`. |

Sparse files read as zeros and use almost no disk space; the work folder is removed
after a passing run like the fixture.

## Adding a scenario

1. Write `fn name(context: &mut Context) -> Result<(), String>` in a module under
   `tests/acceptance/scenarios/` (declare it in `scenarios/mod.rs`). Return `Err` for
   a failure; a panic also fails the scenario with its message, and an
   `Err("SKIP: reason")` reports `SKIP`. Checks of announced behavior that has not
   landed yet keep their full strength in a `files_support::Pending` list
   (`pending.check(what, reason, result)`); `files_support::report_with` then reports
   `PENDING` (an `Err("PENDING: …")`) when nothing else failed, and notes a pending
   check that started to pass so its mark can be removed.
2. Start the application with `App::start(Config::new(left, right))`; `Config` also
   selects the theme, density and window size.
3. Never change `context.fixture`: copy what you change into `context.work` (a new
   folder per scenario, inside the run's own folder) with `transfers::copy_tree`.
4. Act only through input: `panel::select_row`, `panel::enter`/`leave`,
   `input::key`/`chord`/`type_text`/`click`/`click_button`/`fill`/`drag`. Wait with
   `app.until("what", |app| condition)`; its timeout message includes both panels,
   the notice and the last events. A panel row's accessible label is its name followed
   by its annotations (` · Filesystem compressed`, ` · Encrypted`, …) when it has any:
   find a row's item with `panel::is_row_label`, not by comparing the label with the
   name.
5. Verify the outcome on disk and in the UI, add `app.screenshot(context, name,
   caption)` where a picture helps a reviewer, and call `app.shutdown()`.
6. Register it in `scenarios::all()` with an area, a one-line summary and a timeout,
   and add it to the table above. A module listed in `scenarios::OWNED` registers its
   own scenarios instead (`pub fn scenarios() -> Vec<Scenario>`), so its owner changes
   them without touching `scenarios/mod.rs`; names must be unique (the runner refuses
   to start otherwise, exit status 2).

Scenarios run one at a time, each on its own thread (Slint platforms and windows are
per thread). A thread cannot be stopped from outside: an in-process scenario that
exceeds its timeout stops the run, and the remaining scenarios are reported `NOT RUN`,
so its timeout is an honest upper bound, not a target. A staged scenario (it ran
`viewers_stage::run` or `run_private`) stops its child at its own deadline; when its
thread comes back within 60 s after the timeout, it is reported `TIMEOUT` and the run
goes on; otherwise its child is killed, its stage removed and the run stops the same
way. A scenario that needs the account's folders kept out of reach (such as the Trash)
runs its body through `viewers_stage::run_private(context, TIMEOUT, body)`.

### Options and settings helpers

`tests/acceptance/options_support.rs` drives Options and the other settings dialogs
with real input:

- `open` (the toolbar's Options button), `page` (its topic button, or the topic picker
  in narrow layouts), `ok` (waits until Options closes and reports whether settings
  were saved, or the error that kept it open) and `cancel`.
- `choose` and `pick` open a picker and click its popup item; `set_checked` clicks a
  check box's caption; `fill` types into a field; `press` clicks an enabled button.
- `reveal` scrolls a control into view with the mouse wheel, over the innermost scroll
  view whose viewport does not hold it and never over a nested one, so controls below
  the fold of a long page or popup list are reached as a user reaches them.
  `find_any`, `texts`, `labels` and `list_rows` see controls wherever they are
  scrolled; `click_row` reveals and clicks a list row.
- `Frame` renders the main window for pixel checks (mean luma of an area, a digest,
  pixels near a colour); `review_shot` takes a screenshot and runs the UI review's
  layout checks on it.
- `wait_notice`, `notice_open` and `dismiss_notice` follow the notice dialog itself
  (its `NoticePresentation` token), as `App::notice` does.
- `SettingsStore` holds what the application saved, for verification and restarts.

Pitfalls the tools scenarios ran into (helpers in `tests/acceptance/tools_support.rs`):

- The main window's message title is not cleared when a notice is dismissed, so
  whether a notice still shows is `NoticePresentation.token`: `App::notice()` and
  `tools_support::shown_notice` read that.
- `ItemRc::is_visible()` is geometric: a control scrolled out of a scroll view is not
  found by `inspect::find`. Scroll the list with the wheel first
  (`tools_support::scroll_to`, `scrolled_labels`). Virtualized lists create rows for
  their viewport while they are drawn: render a tool window (`app::render`) before
  reading its rows.
- Size a tool window (`app::layout_tool`) before sending it keys: in an unsized
  window the focused control is not visible, and Slint then drops the key.
- A scenario that exposes an application bug reports it at its end, after its other
  checks ran, as `finding <id>: …`, so one finding does not hide the rest.

## Extending

- **Viewers, comparators, thumbnails and archives.** A scenario that needs them
  calls `viewers_stage::run(context, TIMEOUT, body)` with its registered timeout and
  sets `Config::plugins` (a scratch folder for plugin state) in `body`; see "Viewers,
  comparators, thumbnails and archives". `viewers_support` has the window helpers
  (`view`, `Viewing::wait`, `Viewing::canvas`, `Viewing::close`). `viewers_support::next_viewer` waits for a viewer
  opened by any route (Find results, a fed list, warning reports, Make File List);
  `viewers_support::probe` drags scrollbars, turns the wheel, waits for settled content
  and measures how far rendered content moved and the WCAG contrast of text;
  `viewers_support::generated` writes scenario-local content (long tables, Markdown,
  source, multi-page PDFs, every Source extension, damaged files, BMP/ICO/TIFF,
  archives made with the host's own compressors). The application's
  `viewers.rs` and `comparator.rs` are mirrored, not included: their unit-test modules
  need the explorer crate.
- **Installed native filesystem plugin.** `Config::plugins` browses through it, archives
  included; the other scenarios keep `LocalFileSystem`.
- **Windows.** `scripts/acceptance.ps1` runs the suite (see "Windows" under "Running
  it"). Code that needs Unix semantics stays behind `cfg(unix)` with a Windows
  equivalent beside it, or a `SKIP` or a note in the report that names the reason; a
  check is never dropped silently.
