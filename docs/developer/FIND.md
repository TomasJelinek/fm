# Recursive Find service and UI

`midnight-core::FindService` is an internal Rust backend capability, not a stable
external plugin ABI. Windows and Linux implementations live in separate native
modules. The engine accepts `StartFind`, `CancelFind`, `CloseFind` and
`ValidateFindResult`; the UI only captures input and displays shared results.

Each of at most four modeless windows owns a monotonic session ID. Every accepted
search increments its generation. Up to two workers stream batches of at most
64 stable-ID results through bounded channels. Presenter and controller both
reject late generations. Stop retains results; Close releases the session, and
host shutdown closes the windows. Queue rejection never pretends cancellation
or close succeeded. Large result allocations are reclaimed on backend workers.

Roots retain raw native `PathBuf`s independently of their display text. Add/remove
edits a list of at most 16 roots; unchanged paths never round-trip through a lossy
text editor. Masks reuse the panel mask compiler. Content is literal UTF-8 with
optional BOM, streamed using a 64 KiB buffer, strict incremental decoding and KMP.
See the [backend design](../superpowers/specs/2026-11-07-recursive-find-design.md)
for budgets, skipped mount/link policy, revisions and sampled diagnostics.

`files.find` defaults to Alt+F7 and appears in the configurable command catalogue.
An existing explicit Alt+F7 assignment takes precedence over this new default;
the magnifier and configurable action remain available without resetting settings.
The magnifier opens a movable native window; both panels remain usable. Lazy
Slint row models format only requested rows. Form drafts stay in their own window
and are not reset when progress or another panel changes. Presentation revisions
avoid repeatedly formatting unchanged diagnostics during ordinary panel input.

Go to file sends retained session/generation/result IDs, not authority reconstructed
from displayed paths. The backend reopens and validates identity/revision. The UI
checks the pending token/generation again and navigates the originating panel to
the containing directory with a wanted cursor. This is read-only navigation; it
does not execute or edit the found file. A future Open/Edit/Delete needs validation
integrated into its backend execution, not a bare path forwarded after this event.

Current scope is native Windows/Linux. Archives and other providers, UTF-16 and
legacy content encodings, binary/regex content search and automatic file execution
are not enabled. Unsupported content, access errors and reached budgets produce
an explicit incomplete result, counts and Details; no silent complete claim.
