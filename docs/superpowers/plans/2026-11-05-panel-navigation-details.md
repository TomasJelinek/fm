# Panel navigation and details implementation plan

> **For agentic workers:** use superpowers:subagent-driven-development for the
> independent backend, Slint and build tasks; root integrates presenter behavior.

**Goal:** Implement all requested command, panel navigation, details and Options
changes while preserving responsiveness, Unicode and existing read-only scope.

**Architecture:** Existing Tokio engine owns I/O and persistence. Presenter sends
identity-bearing intents; shared Slint components render models. Independent task
owners avoid simultaneous edits to the same files.

**Tech stack:** Rust 1.99.1, Tokio 1.54.1, Slint 1.18.1, Windows/Linux native services.

**Spec:** [panel navigation/details](../specs/2026-11-05-panel-navigation-details-design.md).

## Global constraints

- 100,000 bytes per handwritten source, rustfmt max_width 100.
- All I/O/heavy work on backend; bounded queues and backend reclamation.
- Separate platform implementations/tests; no lossy native identity conversion.
- Preserve read-only internal file operations and existing search/IME behavior.
- Persistent history: 100 unique exact strings, no elevation replay.

## Review focus

- Held menu gestures after model replacement must reject stale identities.
- Space sizing must not cancel other scans or lose results during sorting.
- Failed/blocked selection must not silently change the selected target.
- History recall must not execute or change shell grammar under existing text.
- Cargo writes directly into `_build`; shared WSL builds use `_build/linux`.
  Remove compatibility junctions and rebuild cleanly as explicitly requested.

## Task 1 — Build output organization

Owner: build agent. Files: `.cargo/config.toml`, build helpers, `scripts`, CI,
README build paths. Keep installed toolchains and legacy evidence intact.

- [x] Audit actual native/Linux cache paths, size, free space and active processes.
- [x] Remove compatibility aliases without following them; verify the canonical
  workspace build root, delete it entirely as requested and recreate QA/temp roots.
- [x] Configure direct `_build` outputs/QA paths and verify helper cwd/exit behavior.
- [x] Preserve installed toolchains/download caches; document exceptions. The
  user's later clean-rebuild request supersedes the initial cache-reuse plan.

## Task 2 — Backend metadata, clipboard, preferences and history

Owner: backend agent. Files: `fm-core`, `fm-engine`, platform crates/tests;
`fm-ui/src/presenter/{preferences,terminal,desktop}.rs` and their tests.

Interfaces: `SizeDisplay::{Readable,Bytes}`, `UserPreferences.size_display`,
`remember_command_history`, `command_history`; `CommandHistoryEntry {command,shell}`;
optional native attributes/Unix mode; `Command::CopyText {request,text}` with
`ClipboardCopied`; `TerminalStarted` carries frozen executed history entry.

- [x] Add failing preference migration/unique history and metadata tests.
- [x] Implement schema 5, exact-text dedup, bounded history and late-load protection.
- [x] Add native clipboard ownership on backend with errors and tests.
- [x] Provide presenter methods for preference edits, history recall/clear and
  successful-launch recording; never persist/replay elevated state.

## Task 3 — Targeted size scans and selection advancement

Owner: backend implements `ComputeEntrySize {pane,navigation,scan,entry}`;
root owns presenter scan scopes and cursor actions.

- [x] Tests: Space and Insert advance exactly one row, clamp at last, exclude `..`.
- [x] Tests: Space only sizes real directories, uses frozen identity, preserves
  concurrent scans and same-location sort/refresh, rejects queue pressure safely.
- [x] Implement bounded targeted scans using current global storage budget and
  existing FolderSizeUpdated/FolderSizesFinished events.
- [x] Track per-entry scopes in presenter; retire state on true navigation and
  tab switch, keep raw-path mapping independent of new listing IDs.

## Task 4 — Presenter navigation models and details

Owner: root. Files: new `fm-ui/src/presenter/panel_navigation.rs`,
`panel_details.rs`, `fm-ui/src/panel_models.rs`, `navigation_models.rs`,
`presenter/choices.rs`, `native_shortcuts.rs`, integration in `lib.rs`/app.

Interfaces with Slint: `PathPartRow {identity,label,prefix}`, `DetailRow
{identity,label,value}`, existing NavigationChoiceRow for command/history/hot paths;
callbacks `path-part`, `history-choice`, `detail-copy`, `command-history-choice`,
`command-history-clear`, `sizes-format-changed`, `remember-command-history-changed`,
`tab-number`. Per-pane breadcrumb/details/back/forward models, shared hot-path and
command-history models, size/history options.

- [x] Failing tests for volume-only selected drive, cumulative breadcrumbs/stale
  tokens, exact history navigation, numbered tabs and clipboard field identity.
- [x] Implement raw identity models and callbacks; restore successful/failed paths.
- [x] Implement size formatting option, exact-byte details and typed attributes.
- [x] Route Ctrl+Shift+1…9 to tabs, retain AltGr/editor protection and bookmark UI.
- [x] Integrate backend clipboard and history services into app lifecycle.

## Task 5 — Shared Slint controls and Options topics

Owner: UI agent. Files: `fm-ui/ui` and UI integration tests.

- [x] Add behavior/render regression tests for prompt, Options topics, dropdown
  fill-without-submit, breadcrumb click, details copy and tab/drive placement.
- [x] Extract Options body; add Panel/Appearance/Command topics without duplicating
  existing controls or regressing font/search persistence.
- [x] Add shared detail strip, breadcrumb and history/Hot Paths pickers; vertically
  center icons and strengthen selected-row contrast across themes.
- [x] Keep paths authoritative across changes and separate Ctrl+G edit drafts.

## Task 6 — Integration, review and delivery

- [x] Run appropriate targeted tests, then Windows/Linux complete suites once
  integrated; strict Clippy, fmt, source size and normal app build.
- [x] Inspect actual native UI and screenshots, fix issues and rerun relevant tests.
- [x] Independent behavior/security review of completed diff; resolve findings.
- [x] Update main design, hotkeys, README and verification report with actual results.
- [x] Prepare intended source/docs only for delivery; exclude build products and QA captures.

Delivery: commit and push to `main` with account TomasJelinek as authorized.
The Git history and remote ref record delivery; verification details are in the
[final report](../../verification/panel-navigation-and-smooth-scrolling.md).

## Task 7 — Feedback from the rebuilt native window

- [x] Enlarge drive and file/folder icons 10% without increasing row/toolbar heights.
- [x] Make breadcrumbs visually contiguous while retaining real filename spaces.
- [x] Correct canonical drive captions/tooltips below Home and special locations.
- [x] Align Back/Forward split controls and center their dropdown chevrons.
- [x] Fix the details separator at its top edge and verify native/software pixels.
- [x] Run focused regressions and inspect the final native layout after these changes.
