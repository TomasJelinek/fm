# Modeless Find integration

Approved by the implement-all goal and recursive Find design. Reuse the backend
contract, existing theme/widgets, mask syntax and configurable action catalogue.

1. Add pure presenter session state and tests before production integration:
   bounded four windows, monotonically increasing session/generation/token IDs,
   rejection under full queues, stale result retirement, cancellation and close.
   Keep captured native roots separate from editable/display strings.
2. Add `ui/find-window.slint`, exported through the existing bindings. A native
   movable modeless window has editable masks, recursion and content options,
   root add/remove, virtualized result rows, progress/error details, Stop and Close.
   Results remain independently usable while the main panels navigate. At 320x200
   the query form scrolls and primary actions stay accessible.
3. Add `find_windows.rs` manager and lazy row model; show/refresh only outside
   mutable presenter borrows. Batches and old sessions are reclaimed on workers.
   Escape closes that session; closing the host closes every Find window.
4. Add configurable `files.find` (Alt+F7), panel toolbar entry and backend service
   wiring. Result activation requests backend identity/revision validation and
   only navigates the originating panel to the parent with a wanted cursor. It
   never executes a path received from an earlier validation event.
5. Verify presenter races/queue pressure, actual Slint controls at small/normal
   sizes, Unicode paths, independent windows, keyboard routing, Windows/Linux
   backend suites and strict Clippy. Update design, feature ledger and hotkeys.

Explicit limits: native Windows/Linux locations only; content is strict UTF-8
(optional BOM). Backend summaries disclose exclusions, encoding/access errors and
budgets. Result opening/editing/deletion needs a later guarded execution handoff;
Go to file is a read-only navigation action and does not confer mutation authority.
