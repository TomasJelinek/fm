# Acceptance suite: tools scenarios — verification and findings (2026-11-20)

Scope: the opt-in acceptance scenarios for the tools area (`tools_*` in
`crates/midnight-acceptance/tests/acceptance/scenarios/`, helpers in
`tests/acceptance/tools_support.rs`; see [the acceptance suite](../developer/acceptance.md)):
Compare directories, Find, Checksums, folder sizes, the Jobs window, Help, Drive
information and the main menu. Every result below names the command that produced
it; nothing is claimed from a test's existence alone.

## Environment

Linux host without the GUI development packages, so Cargo ran in the project's Docker
build image (`midnight-explorer-build:9049146550…`): worktree mounted read-only at
`/workspace`, the shared `_build` and `.tools` writable, `CARGO_INCREMENTAL=0`,
`MIDNIGHT_ACCEPTANCE_RUN_DIR=/workspace/_build/acceptance-acc-tools/<UTC run id>`.
Several sessions built into the same target directory concurrently; a per-session
`--config` file (`codegen-units` overrides) gave this session its own
`midnight-acceptance` artifacts. The test binary was compiled with `--no-run` and then
started directly, so the shared build lock was held only while compiling:

```text
cargo test --workspace --locked --config <acc-tools.toml> \
  --features midnight-acceptance/acceptance --test acceptance --no-run
_build/debug/deps/acceptance-<hash> --scale smoke tools_
```

The runner takes scenario filters as plain arguments (`scripts/acceptance.sh` turns
its `--only A,B` into those arguments; the test binary itself rejects `--only`).

## What the scenarios cover, and what they cannot

| Tool | Covered | Not offered by the application (recorded, not tested) |
| --- | --- | --- |
| Compare directories | Size, modification time with tolerance 0/1/2 s (equal-to-tolerance counts as equal), exact case and case folding, unmatched folders; identical, asymmetric and nested trees; unreadable metadata (mode 0444 folder) and a dangling link; invalid tolerances; dialog closed at the click while the comparison ran; no result notice or window. | Content comparison and recursion: the dialog compares displayed metadata only ("Metadata only · displayed files · no recursion"). An Engine failure notice ("Directory comparison failed") cannot be provoked through input here: it needs an archive panel or a capture that changes between the request and the worker. |
| Find | Masks (case, `?`, `;`, `|` exclusions, folder names), Subfolders off, Regex (with case), Expression (size, size range, kind, date, date range, name with size, attributes), Containing text (case, whole words, UTF-16), Modified within, Identical files; Search folders tick boxes, Add, Browse… (scripted chooser), Remove; syntax help for each mode, F1, examples; an invalid expression's status line (tooltip "Click for more details") opening the details page; Stop; 10,000-result limit; sorting; every column resized; Go to file. | Content regular expressions (Regex applies to names; content is literal text), archive search, feeding results to a panel, a depth limit other than Subfolders on/off. |
| Checksums | All five algorithms ticked at once and checked independently; a manifest saved per algorithm; Replace prompt Cancel and Replace; Include subfolders; Verify preselected for BSD, GNU binary, SFV, hash-only sidecars and UTF-16 manifests with mismatches and missing files; a mixed manifest; progress and Stop on the large file; nothing starting by itself. | — |
| Folder sizes | Marked `links/`, `readonly/`, `many/`; folder links inside `links/`; an unreadable subfolder; cancel while running; a complete rescan. | — |
| Jobs | Progress window of a long copy, Pause, the Jobs list while paused, Resume, Cancel operation with its confirmation, the cancellation notice, a completed copy, Jobs states, Clear all finished. | — |
| Help, Drive information, main menu | F1 and Ctrl+F1 content and closing; every main menu item by category, each checked and cancelled or undone; Exit. | Items whose service the harness does not install (View, Edit, clipboard, System properties, Compare files, network) are clicked and their visible effect is recorded only. |

In the harness, View, View as plain text and Compare files… had no visible effect
(viewers and comparators are not installed); Edit, Copy/Cut to clipboard and Paste
files showed "… is unavailable" notices; System properties showed a fallback notice;
Connect/Disconnect network drive… were disabled (no network service).

## Results

Smoke scale, after the last code change of each scenario
(`acceptance-<hash> --scale smoke tools_`, run `20261120-031857`):

| Scenario | Result | Time | What failed |
| --- | --- | --- | --- |
| `tools_compare_dirs` | PASS | 19.3 s | |
| `tools_find_criteria` | PASS | 15.7 s | |
| `tools_find_folders_help` | PASS | 7.8 s | |
| `tools_find_results` | PASS | 26.9 s | |
| `tools_checksum_algorithms` | FAIL | 13.7 s | finding ACC-TOOLS-05 only; every other check passed |
| `tools_checksum_manifests` | FAIL | 11.7 s | findings ACC-TOOLS-01 and ACC-TOOLS-06 only |
| `tools_checksum_large_stop` | PASS | 3.4 s | |
| `tools_folder_sizes` | PASS | 17.1 s | |
| `tools_jobs_copy` | FAIL | 16.3 s | finding ACC-TOOLS-04 only |
| `tools_help_drive_info` | FAIL | 3.3 s | finding ACC-TOOLS-03 only |
| `tools_main_menu` | PASS | 69.7 s | |

A scenario reports a finding after all its other checks have run, so a FAIL above
means "everything else verified, and this application bug was reproduced".

The suite's existing `find_by_mask` and `checksums` scenarios passed on the same
build (run `20261120-030056`), including the Clippy-requested change in `find.rs`.

Clippy on the acceptance target, on the final code:

```text
cargo clippy --workspace --locked --config <acc-tools.toml> \
  --features midnight-acceptance/acceptance --test acceptance -- -D warnings
```

exit 0, no warnings (earlier runs reported `chunks_exact` with constant sizes in the
digest helpers and one `iter().any()` comparison; both fixed). `rustfmt --edition 2024
--check` passes on every changed and new file.

Full scale, the whole suite, final code (`acceptance-<hash> --scale full`, run
`20261120-033127`, 693 s): 15 passed, 1 warned, 4 failed of 20.

- `startup`, `navigation`, `copy_move_delete`, `create_rename`, `tree_expand_all`,
  `folder_sizes`, `find_by_mask`, `checksums`: PASS.
- `ui_review`: WARN, 17 review flags in 186 screenshots (layout flags of the
  foundation's review; not part of this work).
- `tools_compare_dirs` (19.2 s), `tools_find_criteria` (15.8 s),
  `tools_find_folders_help` (7.9 s), `tools_find_results` (116.3 s: 3,010 results
  scrolled through and compared with the disk three times; 10,050 files gave 10,000
  results, Incomplete; Stop at 308 examined), `tools_checksum_large_stop` (3.8 s: 12.8 MB
  read when Stop was pressed, Cancelled at 61.6 MB of 2.2 GB), `tools_folder_sizes`
  (13.4 s: `links/` 0 bytes, a marked folder link measured as its 99-byte target,
  the unreadable folder partial with a warning, the 100,000-file scan cancelled) and
  `tools_main_menu` (69.9 s): PASS.
- `tools_checksum_algorithms` (ACC-TOOLS-05), `tools_checksum_manifests`
  (ACC-TOOLS-01, ACC-TOOLS-06), `tools_jobs_copy` (ACC-TOOLS-04) and
  `tools_help_drive_info` (ACC-TOOLS-03): FAIL on those findings only, as at smoke
  scale.

Nothing in these runs crashed, hung or lost data. No application source was changed.

## Findings

Severity is a first estimate; owners decide. Screenshots are in the run's
`shots/<scenario>/` folder.

| ID | Severity | Area | Finding | Steps | Expected | Actual | Suspected location |
| --- | --- | --- | --- | --- | --- | --- | --- |
| ACC-TOOLS-01 | Medium | Checksums | A manifest that cannot be parsed fails without a reason. | Ctrl+F8 on `mixed.sha256` (one SHA-256 and one MD5 line), Verify. | The window says why, e.g. "Checksum manifest mixes different algorithms.". | "Failed · 0 verified · 0 mismatched · 0 missing · 0 B read", the banner "Verification needs attention" and the Calculate page's "Choose the checksums in the list above, then press Calculate."; the reason exists only in `ChecksumRun.summary.diagnostics`, which nothing presents (`verify-mixed.png`). | `crates/midnight-ui/src/presenter/checksums/events.rs:27-33` (Finished ignores diagnostics); `checksum_windows.rs:268-277` (the error comes from `s.error` only). |
| ACC-TOOLS-02 | Low (decision) | Find | The Expression syntax help offers `attributes has hidden/readonly/…` ("Items with the hidden or system attribute") without saying that Linux listings report no attribute flags, so on Linux these predicates never match (`missing(attributes)` matches everything). The behaviour itself is documented in `docs/developer/metadata-filters.md`; the scenario asserts it. | Find, Expression `attributes has readonly or attributes has hidden` over a tree with a dot-file and a mode-0444 file. | The in-app help mentions the platform limit. | 0 results, no hint. | `crates/midnight-ui/ui/find-help.slint:67,81` |
| ACC-TOOLS-03 | Low | Help | The keyboard help says "Native size scans follow directory links and junctions with cycle detection". Links inside a measured folder are skipped (measured `links/` is 0 bytes); only a marked link row is measured through its target, as `docs/developer/tree-panel.md` says. | F1. | Help matches the behaviour. | The outdated sentence (`help.png`). | `crates/midnight-ui/ui/help-dialog.slint:38` |
| ACC-TOOLS-04 | Low | Jobs | The Jobs window lists a paused copy by its phase ("Copying" or "Verifying SHA-256"); only its progress window says "Paused". | F5 of a large file and `many/`, Pause, Jobs. | The Jobs row says Paused. | "Copying"/"Verifying SHA-256" (`jobs-paused.png`). | `crates/midnight-ui/src/job_presentation.rs:252-289` (row state from `copy_waiting_label`, not `copy_status_label`). |
| ACC-TOOLS-05 | Low | Checksums | After Replace is clicked in the save prompt, Escape no longer closes the Checksums window; Tab gives it focus again. | Save manifest… onto an existing file, Replace, Escape. | Escape closes the window. | The window stays open (`escape-after-replace.png`). | Suspected: the focused prompt button is removed with the prompt row (`checksum-window.slint:148-162`) and nothing takes focus. |
| ACC-TOOLS-06 | Low | Checksums | A Checksums window that opens on the Verify page ignores Escape until something else takes focus. | Ctrl+F8 on `all.md5`, Escape at once. | The window closes. | It stays open (`escape-on-verify-page.png`). | Suspected: `begin()` focuses the results list (`checksum-window.slint:20`, called from `checksum_windows.rs:314`), which the Verify page hides, so Slint drops the key. |

Harness and tooling notes (not application bugs):

- `App::notice()` keeps returning the last notice after it was dismissed: the main
  window's message properties are only written while a notice exists
  (`crates/midnight-ui/src/lib.rs:862-870`). The tools scenarios use
  `NoticePresentation.token` instead (`tools_support::shown_notice`); acceptance.md
  lists this and the other pitfalls.
- The test binary rejects `--only`; pass scenario names as plain arguments.
- Compare directories' Engine failure notice is not reachable through input in this
  harness (no archive panels); it is not covered.
- Known foundation finding 5 (function-key bar stuck on the Ctrl layer after a Ctrl
  shortcut opened a dialog) also shows after Ctrl+F10 and Ctrl+F1, and persists after
  the dialog has closed until the next key (`asymmetric-default.png`, `drive-info.png`).
