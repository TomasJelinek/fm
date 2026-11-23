# Update check — 2026-11-23

## Scope

The update check ([design](../developer/UPDATES.md)): the `midnight-update` crate, the
toolbar button and dialog, About's Check for updates, Options › General › Updates
(settings schema 39), the release signing tool, and the installation swap with
rollback. All work was done in an isolated `git archive` export of `700df95f` with its
own target directory, the debug test profile and `CARGO_INCREMENTAL=0`. No test
contacted GitHub; the transport is injected or a local server on `127.0.0.1` answers.

## Automated tests

| Command | Platform | Result |
|---|---|---|
| `cargo test -p midnight-update --locked --offline` | Windows | 62 passed (WinHTTP loopback flow, three Windows swap tests) |
| `cargo test -p midnight-update -p midnight-platform-common --locked --offline` | Linux (WSL, Ubuntu 24.04) | 59 + 181 + 182 passed (ureq/rustls loopback flow) |
| `cargo test -p midnight-update --release --locked --offline -- client:: loopback::` | Linux | 13 passed: in a release build the loopback endpoint is refused and the loopback flow test returns at once |
| `cargo test -p midnight-core -p midnight-platform-common --locked --offline` | Windows | 157 + 179 + 165 passed |
| `cargo test -p midnight-ui --lib -- preferences options` | Windows | 172 passed |
| `cargo test -p midnight-ui --test integration -- update_dialog about_dialog shared_toolbar options small_window tool_reflow main_window_polish minimum_dialogs tooltips dialog_polish panel_toolbar` | Windows | 221 passed, 6 ignored (the suite's own `#[ignore]`s) |
| `python -B -m unittest scripts/tests/test_release_signing.py` | Windows | 9 passed (RFC 8032 test vectors 1 and 2) |
| `cargo clippy --workspace --all-targets --locked --offline --features midnight-acceptance/acceptance -- -D warnings` | Windows | clean |
| `cargo test -p midnight-explorer -p midnight-update --locked --offline` | Windows | 132 passed, 27 ignored (interactive-desktop tests); 62 passed |
| `cargo fmt --all -- --check` | Windows | clean |
| `cargo clippy -p midnight-update -p midnight-platform-common -p midnight-platform-linux --all-targets --locked --offline -- -D warnings` | Linux | clean |
| `cargo fmt` (changed packages), `scripts/check-source-size.ps1` | Windows | clean; 3,366 files at most 100,000 bytes; `main.slint` 99,278 bytes (was 99,280) |

Defects that tests showed red before their fix:

- `options::tab_walk::every_options_page_tab_cycle_keeps_to_shown_enabled_controls`
  failed while the two update check boxes were stacked at the end of the General page:
  the second was below the page's visible area when Tab reached it (the page has no
  focus-follow scrolling). Placing them side by side keeps the page's height; placing
  them above Command line instead broke
  `options_transaction::panel_history_changes_are_staged_until_ok_and_cancel_restores_them`,
  whose pointer click then missed the scrolled-away check box.
- `shared_toolbar::tiny_window_keeps_one_file_row_and_custom_commands_reachable_with_large_fonts`
  failed at 320 × 200 with touch spacing and 32 px fonts while the update button took
  its place at the right end: the action strip left beside it was too narrow to reveal
  Checksums. The button is now left out below that width (About keeps Check for
  updates).

- The updater left the used-up staged copy's `ready.json` in place until the new
  version confirmed, so the new version offered "Restart to update" again (found in the
  end-to-end run). `a_confirmed_update_replaces_the_installation` failed with
  `assertion failed: !*launcher.saw_ready.borrow()` with the fix reverted, and passes
  with it.

## Local end-to-end run (Windows)

A debug build (`--software`, since this machine's GPU device was lost for new WGPU
devices) in a fake installation folder with a `BUILD-INFO.json` of 0.5.0, the filesystem
and thumbnail packages, a local plain-HTTP stand-in for GitHub's release list and
downloads, a throwaway key made with `scripts/release-signing.py generate-key
--no-password` and releases 0.5.1 (the same executable) and 0.5.2 (an executable that
exits at once), each with `SHA256SUMS` and `SHA256SUMS.minisig` made by the tool. The
test endpoint, the test key and a test profile folder (`MIDNIGHT_TEST_DATA_ROOT`,
`LOCALAPPDATA`) were set for the run only. The steps were driven through UI Automation.

1. 5 s after the window showed, the startup check asked `/releases`
   (`User-Agent: Midnight-Explorer/0.5.0`) and the button became "Version 0.5.1 is
   available".
2. Download and install fetched `SHA256SUMS`, `SHA256SUMS.minisig` and then the archive;
   the dialog showed Restart now.
3. Restart now closed the window; the updater waited for the exit, installed 0.5.1
   in place of 0.5.0 and started it with the same `--left`, `--right` and `--software`
   arguments; it confirmed its start about 4 s later, the backup and staged copy were
   removed and `result.json` read `updated`.
4. 0.5.2 was then installed the same way, its executable exited during the startup
   check (status 1), the updater restored 0.5.1 and started it again, which showed
   "Version 0.5.2 did not start correctly, so the previous version was restored." and
   removed `failed/`.

The settings, the update state and the downloads of the run were in the test profile
folder only. The plugin trust policy refused the unregistered fake installation's
filesystem plugin ("Plugin path is writable or replaceable by another account"), so
its panels showed that notice; it does not concern the update.

`me --ui-smoke-test --software` failed with "Left toolbar action did not focus the left
list" both for this build and for an unmodified build of the same day on this machine.
