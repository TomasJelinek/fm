# Acceptance suite

The acceptance suite exercises Midnight Explorer end to end, close to real use: the
real `MainWindow`, `Presenter` and `Engine` on the native filesystem, driven by
synthetic keyboard and pointer events on software-rendered windows, against a
generated fixture tree. It complements the unit and integration tests; it never runs in
`cargo test`, `cargo test --workspace`, CI or `./build.sh --validate`.

## Running it

```text
./build.sh --acceptance                 # build everything, then run the suite
bash scripts/acceptance.sh [debug|release] [--dry-run] [--list] [--scale smoke|full]
     [--only NAME[,NAME...]] [--strict-review] [--keep-fixture] [--all-themes]
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
profile; the `debug|release` argument names the preceding build only. Options:

| Option | Effect |
| --- | --- |
| `--list` | List the scenarios and exit. |
| `--only A,B` | Run the scenarios whose names contain `A` or `B`. |
| `--scale smoke` | Smaller bulk fixtures (400 instead of 3,000 files, 1–2 MiB instead of 8–24 MiB). |
| `--strict-review` | UI review flags fail the run instead of warning. |
| `--all-themes` | Review every theme instead of the representative subset. |
| `--keep-fixture` | Keep the fixture and scratch folders after a passing run. |

The runner also reads `MIDNIGHT_ACCEPTANCE_FILTER` (comma-separated),
`MIDNIGHT_ACCEPTANCE_SCALE`, `MIDNIGHT_ACCEPTANCE_RUN_DIR`,
`MIDNIGHT_ACCEPTANCE_KEEP_FIXTURE=1`, `MIDNIGHT_ACCEPTANCE_STRICT_REVIEW=1` and
`MIDNIGHT_ACCEPTANCE_ALL_THEMES=1`. Exit status: 0 when every scenario passed or
warned, 1 when one failed, timed out or could not run, 2 for invalid arguments, 127
when Cargo is missing.

### Results

Each run writes `_build/acceptance/<YYYYMMDD-HHMMSS>/` (UTC) and records its path in
`_build/acceptance/LATEST`:

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
`TIMEOUT`, `NOT RUN`), its duration and headline, the counts, the report path and
`Result: PASS|FAIL`. Only the five newest run folders are kept; older ones are deleted
when a run starts. Checkouts that share one `_build` (parallel worktrees using a
common target directory) share this pruning; pass `--run-dir` to keep a run elsewhere.

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
- `cargo clippy --workspace --all-targets` skips the target as well. Lint it with
  `cargo clippy --workspace --locked --features midnight-acceptance/acceptance --test acceptance`.

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

Deliberate differences from the application, to keep runs hermetic and headless:

- Settings live in memory: the user's profile is never read or written.
- Browsing uses the host-native `LocalFileSystem` service, not the installed native
  filesystem plugin, so archive browsing is not covered yet.
- The operating system's file chooser cannot be driven headless. A scripted
  `PathPickerService` (`app::ScriptedPicker`) stands in: it returns answers a scenario
  queued with `app.picker.answer(..)`, otherwise the suggested name in the suggested
  folder, as a user pressing Save does, and records every request.
- Desktop launching, the clipboard, drag-and-drop hand-off, external editors, user
  commands, viewers, comparators and thumbnails are not installed. Viewer and
  comparator windows need the installed plugin packages staged beside the test
  executable; see "Extending" below.

## Scenarios

| Scenario | Area | What it proves |
| --- | --- | --- |
| `startup` | panels | Both panels open on the fixture root and list every top-level folder. |
| `navigation` | panels | Enter/Backspace into every fixture folder (and a level deeper, and the 12-level chain); each listing equals the folder on disk; Backspace returns the cursor to the folder left. |
| `copy_move_delete` | transfers | F5 and F6 of a single file and a folder with awkward names, Shift+F8 of a file and a folder, confirmed in the real dialogs; bytes, trees and panel rows verified. |
| `create_rename` | operations | Shift+F4 creates an empty Unicode-named file and asks before replacing an existing one (which it then empties); F7 creates a folder; F2 renames with the stem preselected and the panel shows the new name at once, cursor on it. |
| `tree_expand_all` | tree | Ctrl+click on the `deep` folder's expander expands the whole 12-level branch inline (the tree lists folders), each `level-NN` at depth NN, and no other branch. |
| `folder_sizes` | tree | Ctrl+Shift+S measures every top-level fixture folder; each size (shown in bytes) equals the logical bytes on disk, links not followed. |
| `find_by_mask` | find | Alt+F7 Find with `*.md`, `*.webp` and `*.sha256` returns exactly the matching files on disk; Go to file puts the panel cursor on a result. |
| `checksums` | checksums | Ctrl+F8 on four marked files: Calculate (SHA-256 checked against an independent implementation), Save manifest…, the Replace prompt, Verify file; then the fixture's MD5, SFV, mismatching and incomplete manifests. |
| `ui_review` | ui | Screenshots of the main window and its dialogs across themes, densities and window sizes, with automatic layout checks. |

`scripts/acceptance.sh --list` prints the current list.

### UI review checks

For every rendered window, `inspect::check_layout` walks the live item tree (popups
included) and flags:

- **control clipped by window**: an interactive control (button, check box, combo
  box, text field, slider, spin box, tab, switch, radio button) extends outside the
  window;
- **overlapping controls**: two interactive controls overlap by more than 2 px in both
  directions, unless one contains the other;
- **zero-size label**: a visible non-empty text has no width or height;
- **label elided to nothing**: a visible text is narrower than about one character
  while it needs more;
- **text does not fit its button**: a button's caption needs more width than it has.

Flags make the review scenario `WARN`; `--strict-review` turns them into failures.
A dialog that does not open (or a step that fails) is recorded as a `dialog step
failed` flag and the sweep continues after Escape.

The review starts the application once per combination (the first loaded preferences
choose theme and density): Blue/Desktop and Dark/Touch at 1600×1000 and 960×600,
Light/Desktop and HighContrastDark/Touch at 1340×820; `--all-themes` adds the main
window of every other theme. Each full combination captures the main window, the main
menu (F10), every Options page, Copy (F5), Move (F6), Delete to Trash (F8), Permanent
delete (Shift+F8), Rename (F2), Create directory (F7), New empty file (Shift+F4),
Compare directories (Ctrl+F10), Panel filter (Ctrl+F12), Make File List (Ctrl+M),
Attributes and its Times page (Ctrl+F2), Hot Paths, About, Drive information
(Ctrl+F1), Find (Alt+F7), Checksums (Ctrl+F8), Help (F1) and Jobs. Options, Hot Paths,
About and Jobs have no default shortcut and open through the window callbacks the main
menu invokes. Every dialog is cancelled with Escape, never confirmed, and the review
works on a scratch copy of `checksums/`. Tool windows are sized to 70 % × 75 % of the
main window (clamped to 480×360…1000×720).

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
| `docs/` | Markdown (headings, lists, code, table, quote, link), HTML with a local stylesheet and image (and a script), minimal HTML, a one-page PDF. |
| `images/` | PNG gradient, greyscale JPEG, palette GIF, lossless WebP, SVG, a truncated PNG, a zero-byte `.png`. |
| `archives/` | Stored ZIP with nested and empty folders and a Unicode member, ustar and tar.gz of the same tree, a ZIP inside a ZIP, an empty ZIP, a text file named `.zip`. |
| `sizes/` | Zero, one and 4096 bytes, and a large random file (2 or 24 MiB). |
| `deep/` | `level-01/…/level-12`, one file per level. |
| `many/` | 400 or 3,000 small files (`item-NNNNN.{dat,txt,log,bin}`) and ten folders. |
| `names/` | Spaces (leading, trailing before the extension), leading dot and dash, Latin/Greek/Hebrew/Japanese, emoji, NFD combining and NFC precomposed forms, upper/lower case, shell punctuation, no extension, many dots, a 224-byte name, hidden folder, folder with spaces. |
| `links/` | Symbolic links to a folder, to a file, dangling, and to its own folder (a cycle); Unix only. |
| `checksums/` | Four data files (one empty, one name with a space) with valid `.sha256`, `.md5` and `.sfv` manifests, one with a wrong hash and one naming a missing file. |
| `readonly/` | A mode-0444 file and a mode-0555 folder (Windows: read-only attribute). |

`index.html` lists every generated entry with its purpose.

## Adding a scenario

1. Write `fn name(context: &mut Context) -> Result<(), String>` in a module under
   `tests/acceptance/scenarios/` (declare it in `scenarios/mod.rs`). Return `Err` for
   a failure; a panic also fails the scenario with its message, and an
   `Err("SKIP: reason")` reports `SKIP`.
2. Start the application with `App::start(Config::new(left, right))`; `Config` also
   selects the theme, density and window size.
3. Never change `context.fixture`: copy what you change into `context.work` (a fresh
   folder per scenario) with `transfers::copy_tree`.
4. Act only through input: `panel::select_row`, `panel::enter`/`leave`,
   `input::key`/`chord`/`type_text`/`click`/`click_button`/`fill`/`drag`. Wait with
   `app.until("what", |app| condition)`; its timeout message includes both panels,
   the notice and the last events.
5. Verify the outcome on disk and in the UI, add `app.screenshot(context, name,
   caption)` where a picture helps a reviewer, and call `app.shutdown()`.
6. Register it in `scenarios::all()` with an area, a one-line summary and a timeout,
   and add it to the table above.

Scenarios run one at a time, each on its own thread (Slint platforms and windows are
per thread); a scenario that exceeds its timeout stops the run and the remaining
scenarios are reported `NOT RUN`.

## Extending

- **Viewers and comparators.** Including `viewers.rs`, `comparator.rs` and their
  helpers from `midnight-explorer/src` (as `command_reclaim.rs` is) works, but plugin
  discovery is relative to the executable: copies of `_build/debug/{viewers,filesystems}`
  must be staged beside the test binary before the first F3, as
  `scripts/validate.sh` does for its native viewer tests.
- **Installed native filesystem plugin.** `InstalledProvider::from_installation`
  can replace `LocalFileSystem` for browsing once archives are in scope.
- **Windows.** The suite code is platform-neutral apart from symbolic links and
  permissions; `build.bat` and a PowerShell runner do not exist yet.
