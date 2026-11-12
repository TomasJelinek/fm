# F5 copy destination workflow

F5 and the Copy function button now open a destination form and submit an
immutable panel selection to the existing verified-copy controller. The system
clipboard is untouched. The backend expands captured identities and uses the
same admission budget, cancellation, progress and checksum readback as paste
and drag/drop. This increment does not implement F6 or replacement/merge.

Validation on Windows x64:

- Core: 24 tests; engine: 178 tests; UI library: 364 tests, all passed.
- Native selected-copy fixtures verify exact selected contents, exclusion of
  filtered entries without fallback, relative-target rejection, shared queue
  capacity and cancellation. All 11 copy-controller tests passed.
- Presenter tests preserve the original selection revision, source and other
  panel destination across selection/navigation changes; reject stale/repeated
  confirmations; retain the draft on admission failure; preserve unpaired UTF-16
  destination names when the displayed text is unchanged.
- Real Slint tests cover F5, editing, Enter/Escape, Tab/Shift+Tab containment,
  failed admission and unrelated notices without losing the edited target.
  Drag bridge tests exclude panels for the whole lifetime of the modal form,
  including repeated polling, and restore targets after cancellation.
- Copy, drag, error-notice and mutation-control integration tests all passed
  (1 + 4 + 3 + 3). The common operation dialog remains usable for rename,
  create-file, create-folder and deletion.
- Strict Clippy passed for core, engine and UI, including all test targets.
- Independent review found and resolved popup lifetime, keyboard focus and native
  drop exclusion issues; final review reported no further actionable findings.

Native Linux qualification (Ubuntu 24.04 through WSL): all 11 copy-controller
tests passed against Linux filesystem adapters, including selected-copy admission,
independent readback, conflicts and cancellation. All 361 Linux UI library tests
and the Slint copy-form integration passed, including Unix raw-path preservation.
The optimized Windows release rebuilt successfully after these changes.

Current bounds: the shared controller admits at most 32 jobs and executes serial
native I/O; each file uses a 256 KiB buffer and independent SHA-256 readback.
The existing selection expansion budget is 4096 roots / 4 MiB of total native
path data per request, with explicit rejection rather than truncation. This is
a request resource budget, not a per-path MAX_PATH limit. Recursive copies may
contain more descendants. Future streaming root admission is separate work.
