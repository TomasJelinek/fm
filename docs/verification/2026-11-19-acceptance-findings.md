# Acceptance suite foundation — verification and findings (2026-11-19)

Scope: the new opt-in acceptance suite (`crates/midnight-acceptance`,
`scripts/acceptance.sh`, `./build.sh --acceptance`; see
[the acceptance suite](../developer/ACCEPTANCE.md)) and what its first runs found.
Every result below names the command that produced it; nothing is claimed from a test's
existence alone.

## Environment

Linux host without the GUI development packages, so Cargo ran in the project's Docker
build image (`midnight-explorer-build:9049146550…`), worktree mounted read-only at
`/workspace`, the shared `_build` and `.tools` writable, `CARGO_INCREMENTAL=0`.
Several other sessions built concurrently into the same target directory.

## Results

### Fixture generator

The generator (`tests/acceptance/fixtures.rs` and `encode.rs`) was compiled standalone
with the host `rustc` and its output decoded with independent tools: PIL 10.2 opened
`photo.png` (160×120 RGB), `banner.jpg` (128×64 greyscale, block levels as generated),
`icon.gif` (48×48 palette) and `pattern.webp` (64×64, checkerboard colours as
generated); `pdfinfo` read `report.pdf` (1 page, 300×200 pt); Python's `zipfile`,
`tarfile` and `gzip` listed `bundle.zip`, `nested.zip` (including the inner ZIP's
member), `empty.zip`, `source.tar` and `source.tar.gz` (whose payload equals
`source.tar`); `hashlib`/`zlib` agreed with every line of `all.sha256`, `all.md5` and
`all.sfv`; both UTF-16 files decode. Smoke scale: 5.5 MB in 33 ms; full scale: 45 MB,
3,131 entries, 0.2 s.

### Suite runs (Docker build image, this worktree's sources)

The test executable was built with
`cargo test --workspace --locked --features midnight-acceptance/acceptance --test acceptance --no-run`
(plus the per-crate isolation flags of finding 11, so no other worktree's artifacts
were linked) and run inside the build image from `crates/midnight-acceptance`:

| Run | Command | Result |
| --- | --- | --- |
| `20261120-002743` | all scenarios, `--scale smoke` | 8 passed, `ui_review` WARN (16 flags in 186 screenshots, 31 captures × 6 combinations), 364.6 s, `Result: PASS` |
| `20261120-003405` | the eight functional scenarios, full scale | 8 passed in 71.1 s, `Result: PASS`; the fixture (45 MB) was removed afterwards |
| `20261120-061627` | the eight functional scenarios after rebasing onto `5b344754`, smoke scale | 8 passed in 72.3 s, `Result: PASS` |

Per scenario at full scale: `startup` 1.2 s, `navigation` 38.4 s (every top-level
folder, six sub-folders and the 12-level chain, each listing compared with the disk),
`copy_move_delete` 8.0 s, `create_rename` 6.7 s, `tree_expand_all` 2.2 s,
`folder_sizes` 1.6 s, `find_by_mask` 3.7 s, `checksums` 8.9 s.

Earlier runs of the same day found harness defects, fixed before the runs above: a Tokio
timer created outside the runtime context, a missing scratch folder, and a review that
compared popup controls with the main window beneath them (2,728 flags, now 16).
`tree_expand_all` exposed that inline-tree expansion waits for the visible-row sampling
production's thumbnail pulse performs; the harness now does it every frame.

### End to end: `./build.sh --acceptance` with the Docker backend

1. `./build.sh --acceptance` on this host (no GUI development packages, so `auto`
   selected Docker) created the build image for the current setup scripts
   (`midnight-explorer-build:b04c4862ae9f…`, 1.69 GB; the image other sessions used,
   `9049146550…`, predates the 2026-11-17 setup changes and lacks `gpg`, `cmake` and
   `ninja`) and started the in-container chain. To share the target directory this run
   had linked the worktree's `_build` to the common one; the HTML viewer step then
   refused the linked output path ("Generated HTML output must not traverse links", a
   deliberate check), so the run stopped with status 2 after the app, filesystem,
   viewer, image and PDF packages. Not a product fault: a checkout's own `_build`
   (the normal case) is a plain directory.
2. The identical in-container command that `scripts/build-docker.sh` composes
   (`sh /workspace/build.sh debug --acceptance` with `FM_BUILD_BACKEND=native`, in that
   image, the shared `_build` mounted directly, plus `CARGO_INCREMENTAL=0`): every build
   step passed (native and network filesystems, basic/source/Markdown/image/PDF/HTML
   viewers, thumbnailers including FFmpeg, archive filesystem), then
   `scripts/acceptance.sh` built the suite (`Finished test profile … in 12m 15s`) and ran
   it at full scale: 8 passed, `ui_review` WARN with 16 flags in 186 screenshots,
   372.7 s, `Result: PASS`; `exit 0 after 2844 s` (most of it waiting for the shared
   Cargo lock). Run folder `_build/acceptance/20261120-083023`. That `build-docker.sh`
   forwards `--acceptance` to this command is covered by
   `test_acceptance_uses_the_docker_backend`.

### Changed application code

`crates/midnight-ui/src/operation_windows.rs` (finding 1):
`cargo test -p midnight-ui --locked --test integration -- mutation_controls
native_operation_windows operations_native_ui` — 31 passed, 0 failed, 767 filtered out
(the refused-send path still shows "The operation queue is busy"); `create_rename`
passes against the real Engine. `rustfmt --check` is clean for the file.

### Static checks

`cargo clippy --workspace --locked --features midnight-acceptance/acceptance --test
acceptance -- -D warnings`: clean (it lints `midnight-ui` too). `cargo fmt -p
midnight-acceptance -- --check`: clean.

## Exclusion from normal runs

- CI and `--validate` gates: `scripts/ci-workspace-tests.py`'s own `load_metadata` and
  `plan` on this checkout (host Cargo, offline) produced 34 gates; the `--lib` and
  `--doc` gates both pass `--exclude midnight-acceptance`, no gate selects a test
  target named `acceptance`, and the package's active feature set in the workspace
  resolve is empty. Cargo metadata shows the `acceptance` test target with
  `required-features = ["acceptance"]` and the library with `test = false`,
  `doctest = false`.
- Build helper: `python3 -m unittest test_build_setup_linux test_build_docker_linux
  test_build_bootstrap_linux test_build_clean_linux` (in `scripts/tests`) ran 59
  tests, OK, including the new cases: `--acceptance` dry run prints the suite's Cargo
  command after the build plan, the suite runs after every build step, `--validate
  --acceptance` runs both with the first failure deciding, the Docker backend forwards
  `--acceptance` to the container's `build.sh`, and invalid combinations (`clean
  --acceptance`, a repeated flag, bad `scripts/acceptance.sh` options) exit 2 before
  any tool runs.

## Findings for the area sessions

Severity is a first estimate; owners decide. Screenshot names refer to the
`ui_review` and scenario captures of the runs above.

| # | Area | Finding | Status |
| --- | --- | --- | --- |
| 1 | Operations UI | Shift+F4 with the name of an existing file showed "The operation queue is busy. Try again." under the replace question, and the dialog kept the title and button "New empty file" although the presenter had switched to "Replace file" (dangerous). `operation_windows.rs` preset the busy error for every confirmation that left the dialog open, and copied title, button text and danger styling only when the dialog opened. | **Fixed here**: the busy error is set only when a send was refused; an open operation dialog refreshes its title, button and styling. Verified by `create_rename` (title and button read "Replace file", no busy text, the second Enter empties the file) and the `midnight-ui` tests below. |
| 2 | Main window, small sizes | At 960×600 the function-key captions are cut ("F2 Ren…", "F7 Crea…", "F9 User…" in Touch; "F9 User me…" in Desktop). KEYBOARD_SHORTCUTS.md says very narrow cells show the function key; these cells elide mid-word instead. | Open (review flags "text does not fit its button"). |
| 3 | Main window, small sizes | At 960×600 the middle-elided path breadcrumbs overlap: duplicate "Open /workspace" buttons on top of each other, an "Open /workspace/_build" button 2 px wide, segments under "Edit full folder path"; a glyph is visibly cut ("…\|d/acceptance"). | Open (review flags "overlapping controls", "label elided to nothing"). |
| 4 | Find window | At 672×450 with Touch density the "Copy details" caption needs 93 px and gets 64 px. | Open (review flag). |
| 5 | Main window | The function-key footer stays on the Ctrl layer behind a dialog opened with a Ctrl shortcut (Ctrl+F2): the Ctrl release goes to the dialog. It recovers when the dialog closes. Cosmetic. | Open (seen in `*-times.png`). |
| 6 | Panels | Right-to-left names display with the extension first (`עברית.txt` shows as `txt.עברית`): the paragraph direction follows the first strong character. File lists usually keep a left-to-right base direction for names. | Open, for a decision. |
| 7 | Docs | `docs/developer/CHECKSUMS.md` lines 143–146 ("Overwrite remains unavailable", "Picking a name does not publish it") and 216–217 ("Save manifest opens the destination form") describe an older flow: the application now saves as soon as the file chooser returns and asks before replacing (`checksums` scenario). | Open (documentation). |
| 8 | Docs | `docs/developer/DIALOG_ERGONOMICS.md` lines 41–46 list About among the native modeless hosts; About is an embedded popup on the main window (the review opens it, sees no new window and closes it with Escape on the main window). | Open (documentation). |
| 9 | Viewers, testability | Viewer and file-comparison windows cannot be driven from a test executable without copies of the plugin packages beside it: `viewers::install` discovers plugins only relative to the executable, and the comparator's image-viewer discovery has no override. The suite does not review them yet; a recipe is in the extension section of ACCEPTANCE.md. | Follow-up. |
| 10 | Operations, environment | F8 (Trash) is not exercised: the native home trash is unavailable in the build container and would write into the real user's Trash on a desktop. Scenarios delete with Shift+F8; the review only opens and cancels the Trash confirmation. | By design of the suite. |
| 11 | Build tooling | With one shared `_build` for several worktrees, Cargo linked another worktree's stale `midnight-ui-bindings` into this worktree's `midnight-ui` (22 compile errors). A per-crate profile override (`--config profile.dev.package.<crate>.codegen-units=255` for every workspace crate) gives each worktree its own workspace-crate artifacts in the same directory. | Workaround documented for parallel sessions; not in any script. |
| 12 | Environment | The Docker build image has only DejaVu fonts, so Japanese names and emoji render as blank boxes in its screenshots. | Environment note. |
