# Modeless checksum window and Ctrl+F8

Status: implementation plan. Backend/actor are verified separately in
[checksum backend verification](../../verification/2026-11-07-checksum-backend.md).
This implements the accepted [checksum design](../specs/2026-11-06-checksums-design.md),
without treating generated digests as authentication or weakening Copy verification.

## Capture and lifecycle

- Add configurable `files.checksums`, default Ctrl+F8, preserving existing explicit
  bindings and disabled overrides. Ctrl+F7 stays case conversion. The Ctrl footer
  shows Checksums and dispatches the same semantic action.
- Reuse the selection-action broker: wait for the captured selection revision;
  hold `SelectedFiles` and the original native parent in a window session. Never
  enumerate selected files on the UI thread or substitute a later active panel.
  Reject archive/provider selections explicitly. Parent rows are never fallback
  targets. A nonempty folder selection is authoritative; recursion starts off.
- Use independent native movable Slint windows, initially at most four visible
  result sessions. Start the default calculation after capture. Recalculate uses
  the same captured selection and a new generation. Closing sends the actor's
  Release command, which cancels calculation and associated saves. A full command
  queue leaves the window open with an error. Host shutdown prevents resurrection
  and the existing actor barrier owns actual native cleanup.
- Backend event IDs and generations select the owner. Old events, old rows and
  stale save callbacks never operate on another generation. Retire large shared
  rows/runs/captured views through the existing backend reclamation path.

## Presentation and actions

- A compact algorithm row offers SHA-256 (default), SHA-512, MD5 and SHA-1;
  Include subfolders is explicit. Calculation uses the backend's one-pass stream.
  Persist the chosen nonempty algorithm set through versioned preferences; an
  absent legacy choice means SHA-256. Keep local edits made during settings load.
- Virtualized results expose name/path, state, bytes read and requested digest
  values. Format visible rows lazily, use native locale for counts and byte sizes,
  and do not reconstruct authority from labels. Failed/changed/skipped/cancelled
  rows never show or copy a calculated digest. Error icon, multiline Details,
  complete/excluded/incomplete counts and cancellation remain visible.
- Copy the selected completed hash or labelled row using the existing backend
  clipboard command. Captured row token and chosen algorithm are validated again
  in the presenter; neither a stale row index nor a displayed hash is authority.
- Save presents an algorithm, absolute output path and exact inclusion/exclusion
  counts. Explicit acknowledgement is required for partial runs, including unknown
  undiscovered work. Unchanged suggested native paths retain their raw PathBuf;
  edited text is validated as a newly entered path. Send only job/generation,
  algorithm, destination and acknowledgement, never UI-supplied hashes.
- Manifest save is no-replace. The UI explains an existing target and lets the
  user choose a new name. Keep save ownership through queued/saving/finished,
  distinguish admission rejection from completion, and support cancellation.
  Save completion reports actual receipt bytes/path and refreshes a matching
  visible native destination parent. A save error cannot claim an exported file.
- Reuse current themed controls, offset/clamped tooltip placement, selection
  capture, preference persistence and clipboard/error paths. Extract small shared
  window helpers where Find and Checksums have identical lifecycle/presentation
  mechanics; avoid duplicating a second independent infrastructure.
- Minimum 320x200: scroll algorithm/details/save areas, keep Stop/Close reachable,
  and keep Escape effective from inputs, results and alternate views. Preserve
  native focus and IME/text editing. Do not add filesystem work to UI callbacks.

## Verification and integration

1. RED/GREEN shortcut default/override/conflict tests and preferences round trips,
   legacy defaults, invalid algorithm sets, null/unknown/duplicate fields.
2. Presenter tests for pending captured selection, parent/folder rules, navigation
   independence, queue refusal, stale generations/row tokens, all terminal states,
   immutable partial acknowledgement, save IDs and close/shutdown ownership.
3. Actual Slint software-window tests for multiple modeless sessions, rows/progress,
   keyboard/clipboard/save callbacks, Escape, 320x200 and every theme/density.
4. Windows/Linux focused backend and UI regressions, strict Clippy, main app check,
   then native app smoke with owned fixtures and memory preferences. Native
   manifest test verifies bytes independently and refuses an existing output.
5. Update main design, hotkeys, developer checksum API, feature status/backlog and
   verification evidence. Do not mark the UI or platform qualification complete
   from backend-only tests.
