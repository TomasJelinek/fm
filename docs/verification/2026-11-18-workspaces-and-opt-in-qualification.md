# Separate workspaces and opt-in qualification — 18 November 2026

Scope: integration tip `93d36411` (batches 8 and 9) plus the batch 10b build and
test changes on top of it: UTF-8 `cargo metadata` and caller-relative paths in the
Windows HTML packager, the CI X11 and installed archive worker steps,
`scripts/archive-test-zip.py`, the AVIF decoder tests, the volume precondition of
`copy_planning_tests` and documentation. The installed archive worker tests were
repeated on the merge of these changes with the batch 10a tip `05acd6a9`. Hosts:
Windows 11 x64 (Windows PowerShell 5.1, Python 3.15 with zlib-ng, MSVC Build Tools
18) and Ubuntu 24.04 in WSL2 (Python 3.12.3, PyYAML 6.0.1, Xvfb). Rust 1.99.1,
`--locked --offline` throughout (the lockfile-less tiny viewer example: `--offline`).
PowerShell 7 is installed on neither host. No GitHub Actions run took place: the
workflow changes are checked by parsing and by reproducing their commands locally.

## Separate workspaces

Unless the table says otherwise, each independent workspace was formatted
(`--check`), linted with `clippy --all-targets -D warnings` and tested as its CI job
does, against SDK bundles packaged from this commit. Counts are libtest results
(passed/ignored).

| Workspace | Windows | Linux (WSL) |
| --- | --- | --- |
| `api` (+ ABI check on Linux) | pass, 63 | pass, 63; `check-abi.sh` C/Rust layouts agree |
| `worker-api`, `thumbnail-api`, `image-viewer-api`, `web-viewer-api` | pass, 5/5/10/13 | pass, 5/5/10/13 |
| `filesystem-api` | pass, 102/1 | pass, 102/1 |
| viewers `basic`, `source`, `markdown` (`scripts/ci.*`, debug package) | pass, 44/2, 66/2, 100/3 | pass, 44/2, 66/2, 100/3 |
| `viewer-conformance` (+ release build, native C test) | pass; MSVC: 7 ABI scenarios | pass; `test-linux.sh` |
| `docs/developer/examples/tiny-viewer` (copied, no lockfile) | pass | pass |
| `midnight-viewer-pdf` (`scripts/ci.*`, debug package) | pass, 6 | pass, 6 |
| `midnight-fs-archive` (`build.* --test`, fixture export) | pass, 67/4, export 2 | pass, 67/4, export 2 |
| `image-decoder`, `midnight-viewer-image`, `midnight-thumbnail-image` | pass, 24, 16, 7 (dav1d 1.5.4 offline) | fmt only (see below) |
| `midnight-thumbnail-video` | pass, 5 (FFmpeg 9.0.2 MSVC prefix) | fmt only (see below) |
| `midnight-viewer-html` | fmt; `--no-default-features` clippy and 25/1 tests | same |
| `midnight-filesystem-native-windows` / `-linux` | pass, 60/1 | pass, 93/1 |
| `midnight-filesystem-network` (+ local-only GIO check on Linux) | pass, 58/1 | pass, 56/1; ignored check passes |
| `midnight-filesystem-native-macos` (portable parts) | pass, 6 | pass, 6 |

The Windows plugin SDK and worker SDK bundles match every checksum that the
plugin lockfiles pin. On Windows the only Python (3.15) uses zlib-ng, which
`web-viewer-api/scripts/package.py` refuses; the classic-zlib web SDK packaged in
WSL (Python 3.12) matches the HTML viewer's pin and was used on both hosts.

Environment notes, none of them a repository defect:

- The worktree's copy in WSL (rsync without `.git`) has no repository, so
  `cargo package` cannot record the provenance that the plugin SDK packager
  requires ("missing release member .../.cargo_vcs_info.json"). The Linux plugin
  runs used the Windows bundle, copied with exactly the files CI uploads; its
  configuration is relative.
- That copy inherits drvfs mode 0777 (0755 after `go-w`), and Cargo packages
  record execute bits, so the first WSL worker SDK differed from every pin.
  Without the execute bits (as in a Git checkout) all six crates match.
- `bootstrap-windows.ps1` requires PowerShell 7. On Windows its Python core
  (`bootstrap-tools.py`) and `build-windows.ps1` ran under 5.1 in an MSVC shell,
  with the pinned archives copied from the main checkout's download cache and
  verified by their pinned SHA-256. The first attempt failed once with
  `[WinError 5] Access is denied` while renaming the freshly extracted NASM
  directory; an identical retry succeeded.
- Linux AVIF and video builds need the pinned Linux NASM source and Ninja wheel,
  and FFmpeg's source signature, none of which is cached here; offline
  preparation stops before compiling, so those workspaces were only formatted.
  The HTML viewer's `native-cef` feature needs the CEF SDK, CMake and a short MSVC
  target directory; its Windows build was not run, and the Linux CEF archive is not
  cached. Its Linux metadata used a private copy of the HTML Linux Cargo cache,
  because the shared WSL Cargo home lacks the `cef` crates.

## Installed archive worker qualification

The ignored real-worker tests ran as the CI `check` job now runs them: the archive
package installed in a private root, the plugin's exported fixtures and a 69 MB ZIP
from `scripts/archive-test-zip.py`. Both hosts pass every case: `test:worker` 1,
`test:streaming_worker` 4 (including
`cooperative_installed_zip_and_solid_sevenzip_keep_mount_and_cached_member`),
`captured_source_tests::` 1 and `test:discovery_kinds` 3. The large ZIP test
reported `index_reads=33925 first_member_source_reads=1294336 member_size=1048576`
on both. The private roots were removed afterwards.

Batch 10a (tip `05acd6a9`) adds the archive spool free-space reserve, new spool roots
and a changed `discovery_kinds.rs`, so the same nine tests ran again on the merge of
this branch's first pass (`3d037912`) with that tip (`git merge-tree --write-tree
3d037912 05acd6a9`, tree `c67c11a9`, merge base `93d36411`, no conflict); the review
follow-up changes none of the code that these tests build. On Windows that tree,
exported to a plain directory, packaged the worker SDK, ran the archive plugin's tests
through `build.ps1 -Test` (67 passed, 4 ignored), exported the fixtures from that test
build (only the plugin crate itself was compiled again, in 0.65 s) and built the host
test executables; the spool volume (`TEMP` on D:) had 51.9 GiB free of 644.8 GiB. In
WSL the mirror of that tree rebuilt the host tests and reused the archive package and
fixtures built there at `3d037912`, whose plugin, worker SDK and filesystem API
sources the merge does not change; the spool volume had 912 GB free. Both hosts pass
all nine tests again, with the same large-ZIP read counts.

## Opt-in host tests

With `HOST_TEST_INSTALL_ROOT`, `HOST_TEST_PLUGIN` (basic viewer), `HOST_BAD_PLUGIN`
and the viewer packages installed as `private-test-installation` installs them:

| Run | Windows | Linux (WSL) |
| --- | --- | --- |
| `midnight-plugin-host` lib `--include-ignored` | 112 passed | 65 passed |
| `midnight-viewer-engine` lib `--ignored` | 4 passed | 3 passed, 1 failed |

The Linux failure is `image_worker_routes_fits_zooms_pans_cycles_and_preserves_basic_override`
("No expected frame"): no image viewer package exists there (see the AVIF note
above). The native viewer, source position and PDF cases pass on Linux.

## Isolated X11 tests

In WSL, on a private Xvfb: with `-noreset` the ten drag/drop tests passed in three
of three runs (0.67-0.80 s each) and the eight clipboard tests (the session test and
the seven `file_clipboard::x11` tests, `WAYLAND_DISPLAY` unset) in three of three.
Without `-noreset` the drag/drop tests failed in one of five runs, where
`x11_registers_verified_proxy_and_restores_previous_host_properties`, which opens 32
connections one after another, could not connect (`x11rb::connect` at
`file_drag_drop_tests.rs:954`), and each run took 2.2-3.0 s while the server reset
after every last disconnect.

## Engine stress

The integration binary ran with `TEMP` on D: under the machine's normal load from
other builds. Both formerly intermittent tests passed 50 of 50 runs with `--exact`:
`selected_folder_sizes::selected_status_reads_cached_totals_and_tracks_global_and_individual_scans`
(3.3-8.9 s, median 4.3 s) and
`tree_controller::rejected_refresh_keeps_calculated_sizes_and_frozen_size_order`
(2.6-7.8 s, median 3.7 s). In ten full runs (415 tests, 11-23 s each) both passed
every time. Run 8 failed another test once:
`operations_real_windows::real_fifo_queue_cancels_queued_job_behind_a_conflict_gate`
recorded the terminal Progress of operation 8 twice (terminal order
`[1, 2, 3, 4, 5, 6, 7, 8, 8, 9]`, `operations_real_windows.rs:359`). That test then
passed 30 of 30 runs with `--exact`. Linux was not stressed.

## Windows Copy/Move qualification volume

`copy_operations::preparation::planning_tests::` passes 10 of 10 with `TEMP` on D:
and fails 8 of 10 with `TEMP` on C:. The fixture message now names the NTFS 8.3
requirement and `docs/developer/test-execution.md`, but at first only the five tests
that check their sources through the fixture's `known()` showed it. The other three
failed without a hint: `endpoint_planning_opens_only_read_only_destination_and_uses_exact_children`
(`qualified native target required`),
`metadata_only_move_can_plan_an_opaque_destination_without_directory_read_rights`
(1 live owner instead of 0) and
`captured_native_roots_keep_deferred_owners_and_original_plan_on_retry` (128 instead
of 0). These three now first probe their temporary folder with an uncounted native
source (`require_qualified_volume`), so all eight failures show the message; with
`TEMP` on D: all ten still pass. `cargo clippy -p midnight-engine --all-targets -- -D warnings`
is clean.

## Scripts, workflow and packaging

- `py -3 -B -m unittest discover -s scripts/tests`: 324 tests OK, 137 skipped, 90.5 s.
  `python3 -B -m unittest discover -s scripts/tests` in WSL: 324 tests OK, 30 skipped,
  22.6 s. With the review follow-up (one more Windows HTML packaging test): 325 OK,
  137 skipped, 83.7 s on Windows; 325 OK, 31 skipped, 24.4 s in WSL. PowerShell 7 is
  absent, so its script paths and the `test-*.ps1` checks under PowerShell 7 were not
  run.
- PyYAML parsed `ci.yml` (10 jobs) and the three composite actions;
  `test_ci_workflow.py`'s structural test passed in WSL, also after the follow-up
  added the spool volume's free space to the archive step. Its new X11 and archive
  tests failed against the previous workflow (four failures).
- HTML packaging: the new non-ASCII metadata test failed before the UTF-8 change
  (`Cannot find path ... because it does not exist`) and passes now, as do the other
  Windows HTML build tests. A second new test calls the packager with relative paths
  after `Set-Location`. Before the packager resolved them against the PowerShell
  location, it failed with `Path outside cef-sdk`, and with only the CEF and output
  directories resolved, with a `DirectoryNotFoundException` from writing
  `dependency-metadata.json` below the process directory; it passes now.
- A no-op Windows build already keeps the installed HTML package when its stamp
  matches (`build-html-viewer.ps1`, test
  `test_short_target_builds_from_the_verified_sdk_and_unchanged_inputs_keep_the_package`).
  `validate-runtime` on the main checkout's complete debug runtime (read only), with
  other builds running: three CLI runs took 8.74 s (cold file cache), 1.04 s and
  0.99 s; in process (`_build/qa/time_validate.py`), 0.68-1.43 s of 2.08-2.93 s hashed
  the HTML viewer's 534 inventoried files (412.5 MB). The deployment that follows
  reads and hashes the same files anyway. The first pass measured 1.67-1.81 s, and
  0.59-0.64 s of 1.09-1.29 s in process, but kept no output.

## Dependency inventory

All 46 metadata/tree pairs were captured fresh: 22 on Windows, 22 on native Linux in
WSL, and the two macOS cross-target pairs of `native-macos`.
`scripts/dependency-inventory.py --check` passed against the committed tables
(643 records, 525 names, 65 direct records), so the inventory is unchanged.
Capture notes: `source` and `markdown` resolve only with the configuration their
`scripts/prepare-sdk.*` derives from the bundle (the bundle's own configuration
replaces crates.io with its vendored API dependencies). From a worktree nested in
another checkout, Cargo's workspace search from `vendor/femtovg` reaches the outer
checkout's manifest; its Windows pair was captured from an exact copy elsewhere.

## Logs

The logs stay in the qualifying worktree's `_build/qa/`. The first pass's logs are
copied unchanged into `_build/qa/first-pass/`; its build outputs, including the
dependency metadata captures, were removed afterwards. Each row names the log with
the passing run; superseded attempts are listed only to explain them.

| Evidence | Windows | Linux (WSL) |
| --- | --- | --- |
| API workspaces and SDK bundles | `first-pass/w2-api.log` | `first-pass/wsl-api.log`; worker SDK without execute bits: `first-pass/wsl-worker-sdk.log` |
| Viewers `basic`, `source`, `markdown` | `first-pass/w3b-viewers.log` | `first-pass/wsl-viewers.log` |
| `viewer-conformance`, tiny viewer | `first-pass/w3-viewers.log` | `first-pass/wsl-viewers.log` |
| PDF viewer, archive filesystem and fixture export | `first-pass/w4-workers.log` | `first-pass/wsl-workers2.log` |
| Video thumbnailer (Windows), filesystem plugins, formatting-only workspaces (Linux) | `first-pass/w4-workers.log` | `first-pass/wsl-workers.log` |
| Image decoder, image viewer, image thumbnailer | `first-pass/w5-images.log` | formatting only: `first-pass/wsl-workers.log` |
| HTML viewer, `--no-default-features` | `first-pass/w8-html.log` (formatting: `w4-workers.log`) | `first-pass/wsl-inventory2.log` (formatting: `wsl-workers.log`) |
| Bad viewer descriptor fixture | `first-pass/w6-fixtures.log` | `first-pass/wsl-hosts-build.log` |
| Installed archive worker at `93d36411` | `first-pass/archive-qual-windows.log` | `first-pass/wsl-archive-qual.log` |
| Installed archive worker at the 10a merge | `mt-archive-build.log`, `mt-archive-qual.log` | `wsl-mt-archive-qual.log` |
| Opt-in host tests | `first-pass/host-ignored-windows.log` | `first-pass/wsl-host-ignored.log` |
| Isolated X11 | — | `first-pass/wsl-x11.log` |
| Engine stress | `first-pass/stress.log`, `first-pass/stress/full-08.log`, `first-pass/stress-fifo.log` | — |
| Copy/Move volume | `q3-planning-{c,d}-{before,after}.log` | — |
| Engine Clippy | `q3-engine-clippy.log` | — |
| Script tests | `first-pass/scripts-windows.log`, `scripts-windows.log` | `first-pass/wsl-scripts-final.log`, `wsl-scripts.log` |
| `validate-runtime` timing | `validate-runtime-timing.log` | — |
| Dependency inventory | `first-pass/w7-inventory.log`, `first-pass/w9-inventory2.log` | `first-pass/wsl-inventory.log`, `first-pass/wsl-inventory2.log` |

Superseded attempts: in `first-pass/w3-viewers.log` the `basic`, `source` and
`markdown` steps stopped on a path error of the step script itself; in
`first-pass/wsl-workers.log` the archive and PDF steps failed (exit 101) before the
worker SDK was packaged again without execute bits; `first-pass/wsl-html.log` failed
(`no matching package named cef`) without the private copy of the HTML viewer's
Cargo cache; `first-pass/w7-inventory.log` and `first-pass/wsl-inventory.log` contain
failed `source`, `markdown`, `renderer-tests` (Windows) and `html-viewer` (Linux)
captures that `w9-inventory2.log` and `wsl-inventory2.log` repeat with the right
configuration. `first-pass/q3-copy-planning-{c,d}.log` and `w10-engine-clippy.log`
predate the volume precondition.
