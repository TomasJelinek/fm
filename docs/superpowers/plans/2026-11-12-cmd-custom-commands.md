# Cmd Custom Commands Implementation Plan

> Use the executing-plans skill for this coordinated implementation. Do not commit
> independently; the parent owns shared qualification and integration.

**Goal:** Qualify explicit Windows Cmd Custom Commands without interpolating
captured filenames into shell source.

**Architecture:** Keep literal script and exact raw `ME_*` environment values.
For each declared scalar add a fixed `ME_*_ARG` companion containing Windows
C-runtime argument quoting. The picker inserts `!ME_FILE_PATH_ARG!` without
additional quotes. The Custom Cmd profile uses `/D /U /E:ON /V:ON /S /K` and the
existing native process tracker. Ordinary command-bar Cmd remains `/V:OFF`.

**Tech stack:** Existing Rust/Win32 process/environment APIs, native Cmd and the
owned UTF-16 argv/cwd/environment helper. No dependency or plugin ABI change.

**Spec:** `docs/developer/USER_COMMANDS.md` and the existing Custom Commands spec;
this plan records the explicitly approved language-specific extension.

## Contract and limits

- Companion references are complete arguments for directly invoked external
  programs using Windows C-runtime argv parsing. They are not generic Cmd syntax
  quoting, executable-name quoting, or a protection for arbitrary script authors.
- Builtins use raw `ME_*` references with their own syntax. `echo`, `type`, and
  `cd` receive separate native qualification. Never recommend the argv encoder
  as universal builtin quoting.
- `CALL`, nested shells, batch re-expansion, and pipe/block contexts can parse text
  again. Qualification of a direct invocation does not cover arbitrary reparsing.
  Native probes must retain both successful and counterexample evidence.
- Preserve raw UTF-16, empty values, exact cwd, no NUL, fixed key sanitation and
  the existing aggregate environment bound. Refuse initial Cmd command lines and
  declared/encoded environment records exceeding Cmd's native 8191-unit limit;
  do not truncate. User script can still construct an oversized expanded line.
- Cmd command-string mode ignores subsequent physical lines. Reject CR/LF in
  script source before spawn; preserve control characters in captured argument data.
- Verbatim DOS cwd converts only after both live directory identities match.
  UNC/device and changed DOS meanings fail; native long-cwd errors are retained.
- No parent environment mutation, automatic elevation, forced child termination,
  additional thread/queue, or changes to cancellation and process-quota ownership.

## Tasks

- [x] Inspect core/platform/UI fences; probe actual Cmd parser behavior before edits.
- [x] Demonstrate plain references lose empty/trailing-slash arguments, and the
  companion form round-trips Unicode, percent, bang, ampersand, caret, parentheses,
  quotes, empty values and unpaired UTF-16 in direct native argv.
- [x] Add native platform regression and observe current Unsupported RED.
- [x] Reuse one Windows argv encoder; add bounded Cmd companion environment and
  dedicated launch-profile selection without changing ordinary Cmd behavior.
- [x] Remove qualified core fences; enable exact picker reference and shell label.
- [x] Qualify native direct argv, builtin behavior, explicit reparse limits,
  cancellation and size refusal; retain PowerShell/environment regression coverage.
- [x] Run targeted core and native checks; record the exact results.
- [ ] Parent combined gate: the new UI form test and final all-target Clippy.

## Ownership

Platform desktop Custom Command preparation, terminal process/environment and
new dedicated Cmd tests are owned by this task. Core `user_command_process.rs`,
UI `user_command_settings/form.rs`, `form_tests.rs` and the Custom Command shell
label/hint are explicitly delegated by the parent. Shared engine startup and
configuration remain parent-owned. All generated artifacts stay under `_build`.
