# Compact Custom Commands settings implementation plan

> **For agentic workers:** Use the existing explicit file ownership. Root owns execution/toolbar integration; this plan covers settings UI and drafts.

**Goal:** Match the requested Salamander-style ordered command list: one line per item, small header actions, selected details below, explicit direct/terminal execution, inert separators.

**Architecture:** Keep stable command IDs, generation-checked drafts, Options save/cancel, native path preservation and structured argv. Reuse ListHeader and ArgumentEditor. Core adds the unit invocation Separator; settings renders it without executable or working-folder controls and refuses Preview.

**Tech stack:** Existing Rust presenter and Slint widgets; no dependencies.

**Reference:** `D:/experiments/salamander-main/src/dialogs4.cpp`, `CCfgPageUserMenu::EnableButtons`, `LoadControls`, `StoreControls`, and `WM_INITDIALOG` near lines 2080–2290. Its editable ordered list has header actions and selection-specific details; separators disable command controls. The user explicitly requested this arrangement and delegated routine implementation choices.

- [x] Inspect prior behavior and preserve identity/drag/cancel policy. Existing rows are two lines/42px minimum and the full-width New/Delete/Up/Down grid is below the list.
- [x] Add real software-window regression for one-line rows and compact actions above the list. Isolated RED attempt cannot link stale UI against rebuilt core; no behavior failure is claimed from that attempt.
- [x] Replace the button grid with ListHeader; use one-line rows, summary tooltips, bounded list height and unchanged drag coordinates.
- [x] Label execution choices Run directly / Run in terminal / Separator. Keep all mode fields in the draft when switching. Retain structured arguments and working directory controls for executable modes; collapse optional appearance and terminal variable declarations.
- [x] Convert Separator in form/presentation, preserve stable ID/order and refuse settings Preview without sending a command. Core/engine separately reject execution.
- [x] Verify actual Options layout, mode switching, separator apply/cancel and no execution. Run existing reorder/drag/native-path/settings tests and inspect software render artifacts after the coherent UI build.

The final software-window settings/toolbar gate passes all 20 tests, with 53
command and 7 picker library tests passing. Managed-artwork follow-up removes the
editable path bypass and routes both choices through typed native pickers; the
[verification note](../../verification/2026-11-12-compact-custom-commands.md)
records the causal guard test, fixture corrections and reviewed renders.

All outputs belong under `_build`; source UTF-8 LF and Rust width100. No commits from this agent.
