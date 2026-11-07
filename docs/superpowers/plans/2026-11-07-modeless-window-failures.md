# Modeless native window failure follow-up

The checksum creation-failure probe confirms that refused Release commands keep a
hidden session, and the old zero-time callback recreates a failing native window
on every timer turn (attempts2→8 in six immediate turns). Root authorized a shared
bounded failure coordinator for Find and Checksums and accessible Find rows.

- Add `tool_window_failure.rs` and a module declaration. A per-manager failed-ID
  set suppresses repeated creation immediately. Report once after the caller's
  Presenter borrow ends. Retry close/release only every250ms, with weak host,
  Presenter, registry and closing references. Keep retrying until admission works;
  host shutdown/drop stops retries and the engine shutdown barrier owns cleanup.
- Narrow `find_windows.rs`/`checksum_windows.rs` edits to coordinator wiring and
  failed-creation branches. Never hold registry or Presenter borrows across Slint
  callbacks, rendering or model notifications. Root retains checksum row/model,
  context menu and layout ownership.
- Add forced software platform failure tests: full release queue then acceptance,
  no repeated native attempts, one error report, delayed retries, reentrant
  Presenter borrow and host shutdown. Existing successful window behavior remains
  covered by Find and Checksum integration suites.
- Expose a generic role/label accessibility lookup from shared modeless test
  support. Add actual Find accessible list/row selection test first, observe RED,
  then add role/count/index/selected/label and a default selection action which
  focuses the results. Selection must not reveal a target until explicit action.
- Run focused Windows/Linux UI integration and strict Clippy with the parent
  batch, formatting/source-size checks. Evidence stays under `_build/goal/`.
