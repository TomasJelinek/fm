# Custom Command Preview Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep Options usable when previewing a Custom Command, with resolved
values displayed beside the draft and no process launch.

**Architecture:** Preserve the existing backend preview/captured-context path.
Carry the originating settings draft identity through asynchronous selection
capture and preview replies. Render a bounded read-only preview inside the
Custom Commands page; only previews requested outside settings may open Jobs.

**Tech Stack:** Existing Rust, Slint and backend command services; no dependencies.

**Spec:** `../specs/2026-11-07-user-commands-design.md`, plus CMD-03 in
`2026-11-09-provider-performance-usability.md`.

## Constraints and evidence

- Settings use OK/Cancel; Preview neither saves nor dismisses the settings draft.
- Template expansion, native validation and process work remain on backend workers.
- Preview sends only the existing preview command, never ExecuteUserCommand.
- Context and draft identity must survive delayed selection and backend replies.
- Source/license rules, Unicode, 100-column Rustfmt and 100 KB source limits apply.
- Existing build/performance source freeze must finish before source edits.

The observed route in `user_command_presentation.rs` sets `command-preview-token`;
`main.slint` responds by opening Jobs while Options remains open. This can put
the interaction behind the wrong popup. Salamander's `src/lang/lang.rc`,
`IDD_CFGPAGE_USERMENU` (lines 246–274), keeps the command list and Command,
Arguments, Initial directory and icon controls together. This implementation uses
that grouping without copying its source/assets or weakening separate argv values.

## Review focus

- Preview reply after editing/switching/cancelling the originating draft must not
  open Jobs or replace a different draft's output.
- Two preview replies arriving out of order must retain only the latest result.
- Queue rejection and backend validation failure must remain visible in Options.
- Program arguments containing spaces/Unicode remain separate native values.
- Narrow/touch layouts must retain scrolling and access to Preview, OK and Cancel.

## Task 1: Preserve preview destination and draft identity

Files: `presenter/user_commands.rs`, `presenter/user_command_settings.rs`,
`presenter/user_command_settings_tests.rs`, `presenter/tests/user_commands.rs`.

- [x] Add a regression showing that a settings preview currently becomes a Jobs
  auto-open request; cover a changed draft and delayed reply too.
- [x] Carry a typed settings origin through `CommandCapture` and `LaunchJob`.
  Only non-settings previews set `preview_ready` for automatic Jobs presentation.
- [x] Keep preview errors tied to that form, without hiding Options behind a
  different modal surface. Preserve queue/backpressure and deferred cleanup.
- [x] Run relevant presenter tests; verify no Execute command is sent.

## Task 2: Present resolved values within the settings page

Files: `user_command_settings_presentation.rs`, `user_command_presentation.rs`,
`ui/user-command-models.slint`, `ui/user-command-form.slint`,
`ui/user-command-options.slint`, `ui/options-dialog.slint`.

- [x] Reuse the existing bounded preview formatting, with separate argument rows,
  explicit omitted-content markers and a clear read-only result.
- [x] Bind only the current originating draft's result. Clear stale output after
  editing or selecting another command; keep the normal form controls usable.
- [x] Put draft actions together rather than placing Update command in the
  working-directory row. Keep program, argument and folder pickers available.
- [x] Remove the unused preview-started callback chain and stale popup comments.

## Task 3: Verify real interaction and document the result

Files: `tests/user_menu_forms_ui.rs`, a focused verification report, existing
Custom Commands design and the provider/performance completion ledger.

- [x] Exercise actual Options controls, asynchronous preview delivery, continued
  form editing, Cancel/OK and the absence of Jobs opening. Render compact and
  regular layouts and inspect them.
- [x] Run the affected UI tests and strict target Clippy; run installed native
  window checks after the combined application build.
- [ ] Review the final patch and record verified behavior and remaining limits.
  Commit/push a coherent reviewed slice, preserving other pending work.
