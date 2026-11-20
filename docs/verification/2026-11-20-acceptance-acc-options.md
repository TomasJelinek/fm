# Acceptance: Options and customisation (2026-11-20)

Scope: the Options and customisation scenarios of the opt-in acceptance suite
([Acceptance suite](../developer/ACCEPTANCE.md)). Each scenario starts the real
MainWindow, Presenter and Engine headless with Slint's software renderer, acts only
through synthetic keyboard and pointer input on folders it generates, verifies the
visible effect in the UI and what the application saved in the in-memory settings
store, and restarts the application on that store to verify persistence.

## Environment

- Source: `origin/main` 038e2eab with the coordinator's integration snapshot
  5ada8623 applied, uncommitted.
- Build: the project's Docker build image, `cargo test --workspace --locked` with the
  `midnight-acceptance/acceptance` feature, debug test profile, Slint software
  renderer, DejaVu fonts only.
- Fixture scale: `smoke` while iterating, `full` for the final run.

## Commands

All commands ran in the build image with the repository mounted read-only at
`/workspace`, `CARGO_TARGET_DIR=/workspace/_build`, `CARGO_INCREMENTAL=0` and
`MIDNIGHT_ACCEPTANCE_RUN_DIR=/workspace/_build/acceptance-acc-options/<run>`. The
`--config` file only sets per-package `codegen-units`, so that agents working in
parallel keep separate artifacts in the shared target directory.

```text
cargo test --workspace --locked --config <agent config> \
  --features midnight-acceptance/acceptance --test acceptance -- --scale smoke options_
cargo test --workspace --locked --config <agent config> \
  --features midnight-acceptance/acceptance --test acceptance -- --scale full
cargo clippy --workspace --locked --config <agent config> \
  --features midnight-acceptance/acceptance --test acceptance -- -D warnings
```

## Results

Final full-scale run of the whole suite (`--scale full`, every scenario of this
checkout): **16 passed, 2 warned, 0 skipped, 2 failed of 20 in 590.9 s**. Both failures
and the Options warning are the recorded findings below; every other check of those
scenarios ran and passed before the finding was reported.

| Scenario | Status | Time | Verified |
| --- | --- | --- | --- |
| `options_general` | PASS | 9.2 s | Search filter (typing hides non-matching rows; off: cursor jump, all rows), exact byte sizes in the panel, quick copy verification note, disk I/O "Manual: 2", history off, Clear command history; Cancel discarded a previewed change; OK saved all six; restart showed them; Reset to defaults restored them and kept the tabs. |
| `options_appearance` | PASS | 16.9 s | Light/Dark preview (panel luma), Cancel restored Blue exactly; desktop 16 px and touch spacing grew rows; DejaVu Sans Mono and 18 px touch text changed the rendered panel; smooth text rendering stored; restart showed theme, density, row height and every picker; Reset to defaults restored Blue/Desktop/12/15/Sharp. |
| `options_custom_commands` | FAIL (OPT-CMDERR) | 40.7 s | Grouped `/bin/sh` run listed both selected paths into the work folder; per-item `touch ${file.name}.stamp` stamped exactly the two selected files; drag reorder, rename with toolbar caption, separator between commands, Delete (Cancel kept it, OK removed it); Preview showed the plan and started nothing; Cancel discarded a staged command; `user-command.custom-1` bound to Ctrl+Alt+J ran the command before and after a restart; F9 opened the User menu. |
| `options_keyboard` | PASS | 16.2 s | F5 reported as used by "Copy selected files to folder — File list and filename search"; replacement declined, then accepted and cancelled with Options (F5 still opens Copy); Create folder F7 → Ctrl+Alt+K: the new key opens it, F7 does not, also after a restart; Restore command brought F7 back. |
| `options_colors` | PASS | 17.1 s | Added `*.dat;notes.txt` (#FF2050): .dat rows coloured at once, notes.txt only after moving the rule to the top; saved as the first of 13 rules; red pixels in the panel; Cancel kept the rule after a previewed removal; restart kept the colours. |
| `options_archives` | FAIL (OPT-ESC) | 16.4 s | `MyBak` stored as `.mybak` (ZIP), `pack7` (7-Zip), a temporary chip removed; spaces, wildcards, paths, empty parts and duplicates refused with their reasons; `zip` pointed to its built-in check box; `.jar` association enabled; Cancel kept a removed chip; restart showed both formats' chips. |
| `options_editors` | PASS | 10.8 s | Fallback and `*.md` rule as `/bin/cp {file} <work>`: F4 copied each file to its own destination before and after a restart; a rule without `{file}` kept Options open with the reason; Cancel saved nothing. |
| `options_comparators` | PASS | 13.8 s | `*.cmp` → Built-in text and `*.ext` → External `/bin/true {left} {right} --brief` (never run) saved and listed after a restart; `([` kept Options open with the parse error (OPT-CMPERR). |
| `options_previews_plugins` | PASS | 9.9 s | 192 px grew Thumbnails rows by 96 px; previews off disabled the dependent switches; OK saved all four values; Cancel restored a 64 px preview; restart kept them. Plugins: every section showed its empty state without staged plugins; Rescan finished; Archive associations… opened Archives. |
| `options_hot_paths` | WARN (OPT-HOTPATH) | 15.9 s | Add current saved three folders as 1–3; giving gamma 1 exchanged it with alpha (3); removing beta kept its folder; Ctrl+1/Ctrl+3 opened gamma/alpha, Ctrl+2 nothing, also after a restart. |
| `options_visual` | PASS | 58.6 s | 30 Options pages (Light and Dark at 1340×820, Dark touch at 720×560 with the topic picker): no layout flags. |

The suite's other scenarios passed at full scale (`startup`, `navigation`,
`copy_move_delete`, `create_rename`, `tree_expand_all`, `folder_sizes`,
`find_by_mask`, `checksums`); `ui_review` warned with 13 flags on the main window and
Find at 960×600, none on Options pages (outside this area).

Clippy on the acceptance target with `-D warnings`, run after the full suite on the
same sources: clean (exit status 0, no warnings).

Not covered:

- Browsing a file with a custom archive extension: the harness browses with the
  host `LocalFileSystem`, not the installed native filesystem plugin, so only the
  setting, its display and its storage are verified.
- Plugin enable/disable and priority: no plugin packages are staged beside the test
  executable, so the Plugins page shows only empty sections.
- The text rendering choice configures the FemtoVG renderer at startup; the software
  renderer ignores it, so it is verified by storage and display only.
- Options has no settings for columns, date formats, confirmations, hidden files,
  sort defaults or tooltips, so none were tested.
- Run in terminal Custom Commands and real editors are deliberately never started.

## Findings

Evidence paths are relative to the final run's folder
(`_build/acceptance-acc-options/full1/`).

| ID | Severity | Steps | Expected | Actual | Evidence | Suspected source |
| --- | --- | --- | --- | --- | --- | --- |
| OPT-ESC | Medium | Options › Archives › Add custom archive extension, type a suffix, press Escape (the field's hint: "Press Enter to add it, or Escape to cancel"). | The entry is cancelled; Options stays open with its other unsaved changes. | The whole Options dialog closes (Cancel), discarding every unsaved change of the session. `options_archives` fails with this message after all its other checks passed. | `options_archives` message; field holds the typed text before Escape | `crates/midnight-ui/ui/options-dialog.slint:187` (`capture-key-pressed` closes Options on Escape before the focused field sees it; the field's own Escape handler is `archive-options.slint:113`) |
| OPT-HOTPATH | Medium | Manage Hot Paths with three folders that share a long parent path. | Each row identifies its folder (for example the final folder name stays visible, or the middle is elided). | Every row shows the same truncated prefix ending in "…": the folders cannot be told apart without hovering for the tooltip. The UI review flags "text does not fit its button" three times. | `shots/options_hot_paths/hot-paths-reordered.png` | `crates/midnight-ui/ui/hot-paths-dialog.slint:97` uses `PathText`, which clips the end (`path-text.slint:28`) |
| OPT-CMDERR | Low | Configure a Run directly Custom Command whose program does not exist, run it from the toolbar. | The notice names the Custom Command operation. | "Custom Command needs attention … Operation: external editor"; it also reports "0 processes started · 0 not started" for the one planned process. `options_custom_commands` fails with this message after all its other checks passed. | `shots/options_custom_commands/commands-failure.png` | `crates/midnight-platform-linux/src/user_commands.rs:76` validates the program with `editor::validate_program`, whose errors are labelled "external editor" (`editor.rs:135`) |
| OPT-CMPERR | Low | Options › Comparators › Add comparator rule, Regular expression, pattern `([`, OK. | Options stays open and names the syntax error. | It does, but the message runs the regex parse error into an unrelated size-limit sentence: "error: unclosed character class Comparator matcher limits are 2097152 bytes each for the program and DFA …", and wraps it mid-word. | `shots/options_comparators/comparators-invalid.png` | `crates/midnight-platform-common/src/comparator_rules.rs:44` appends the limit sentence to every compile error |
| OPT-TIP | Low | Hover a control with a prose tooltip, for example Clear command history or Apply colors. | Tooltip lines wrap between words. | Words are split across lines ("co / mmand", "aft / er validation"). | `shots/options_general/general-changed.png`, `shots/options_colors/colors-rule.png` | `crates/midnight-ui/ui/controls.slint:27` (`TipBody` text uses `wrap: char-wrap`) |
| OPT-F9 | Low | Configure Custom Commands, press F9. | The User menu offers the configured commands (the settings page follows Salamander's User Menu). | F9 shows only the fixed built-in entries; Custom Commands run from the toolbar and assigned keys only. `docs/KEYBOARD_SHORTCUTS.md` still says that no custom command execution is configured in this milestone, which is no longer true. | `shots/options_custom_commands/commands-f9.png` | `crates/midnight-ui/ui/main.slint:1534` (`user-menu` rows are fixed); `docs/KEYBOARD_SHORTCUTS.md:183` |
