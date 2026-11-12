# Progressive transfer deselection

Requirement: deselect each successfully copied or moved selected root while the
job is still running. Leave skipped, failed and cancelled work selected. A folder
is successful only after its entire subtree, including empty directories.

Reference inspected: Salamander `src/fileswn8.cpp`, the `StartProgressDialog`
branch around lines 1104–1125, clears selection after successful admission;
`src/fileswn0.cpp::UnselectItemWithName` updates one item and selection status.
Midnight Explorer follows the requested stricter completion timing rather than
clearing all marks at admission. No reference code is copied.

The backend records each requested root's initial failed/skipped counters and
reports `CopyEvent::RootCompleted` only after traversal and current-file work
finish. Copy has already passed SHA-256 destination readback and publication;
Move must also finish its source-removal proof/finalization. Notifications precede
subsequent conflict prompts and terminal progress. Native paths retain raw OS
names; archive notifications use mount/entry IDs, never display names.

The presenter captures pane selection contexts at admission. Completion queues
idempotent removal in batches of at most 64 through the existing backend selection
workers. It leaves the caret unchanged and updates normal selection totals. Full
queues retry, including backend refusals; same-folder refresh retries raw paths
against the new index. New navigation contexts discard obsolete feedback. Terminal
job progress does not discard already received completion edits. Later manual
selection is not repeatedly cleared after acknowledgement.

Initial regression `_build/qa/transfer-selection/red.log` failed: two successful
roots produced no completion notifications before the next overwrite conflict.
The first source-mutation fixture failed because Windows correctly denied writing
to the pinned source (sharing violation). It was replaced with a native
cancellation case and a changed archive source case; no source locking policy was
weakened. An initial UI build using full debug information was stopped and rerun
with the existing low-artifact debug verification profile.

## Review and final verification

Independent read-only review found and corrected:

- Backend refusal coalescing could discard the Remove acknowledgement. The UI
  now retries when a later refusal resolves past that revision. The new regression
  failed before correction (`ui-review-red.log`, seven passed / one failed).
- Archive navigation and an archive/native round trip could revive the old
  selection context. Each distinct location transition now obtains a new context.
- Locally cancelled admitted drops could retain completion origins. History
  maintenance now removes origins for every terminal job, including local cancel.

Final review confirmed all findings resolved. The regression suite also checks
later manual reselection, clipboard/drag admission, native raw paths after ID
recycling, terminal progress with pending selection edits, full subtrees, skipped
children, archive identities, failed archive validation and actual source removal.

- Windows: 29 core + 389 engine + 518 UI library tests passed (**936** total).
- Linux/WSL: 29 core + 381 engine + 514 UI library tests passed (**924** total).
- One opt-in selection CPU benchmark was intentionally ignored on each OS.
- Strict core/UI Clippy with all targets and no dependencies passed. Engine
  Clippy and the app build passed with 34 existing warnings from separate
  unfinished inline-tree work; no warning was suppressed for this fix.
- Scoped Rustfmt and diff whitespace checks passed; all 1,254 checked source
  files meet the 100,000-byte limit. The final test-module ordering adjustment
  was formatting only. No dependency or platform-specific source changed.

Logs are under `_build/qa/transfer-selection/`: `windows-tests.log`,
`linux-tests.log`, `clippy.log`, `engine-clippy.log`, and `app-build.log`.
The debug executable `_build/debug/me.exe` is 83,524,096 bytes, SHA-256
`1a1825df6fa18d393e59c3939ba3b448a138d9387e3c9d80da95657acd7527f0`.
Release was not rebuilt. The existing verified plugin packages were deployed
with readback checks and registered for this executable in
`C:\Users\tjelinek\MidnightExplorer-20261108-125803-b94ba8f6`.

Native Windows startup with isolated Unicode fixtures passed, exited zero with
empty stderr, and its screenshot was inspected. Saved preferences were unchanged
(SHA-256 `bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`).
This smoke test verifies startup, not manual transfer interaction; transfer
behavior is covered by real filesystem engine tests and presenter regressions.
