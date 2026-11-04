# Compact tooltips implementation plan

Goal: reclaim persistent UI space and expose shell/drive details contextually.
Architecture: pure cached UI presentation, bounded backend metadata, separate
Windows native label implementation. Rust 1.99.1, Slint 1.18.1, Tokio; no new
dependencies. Spec: `../specs/2026-11-04-compact-tooltips-design.md`.

- [x] Backend: add VolumeDetails/service/event, native Windows label lookup and
  independent two-slot background refresh. RED/GREEN isolation, stale and error tests.
- [x] UI: add tooltips and compact accessible keyboard-operable selectors;
  remove permanent help text. Actual Slint pointer/keyboard coverage.
- [x] Presenter/bridge: validate cached details by ID/raw root, retire replaced
  data to backend, format dropdown/tooltip/loading/error states with Unicode.
  Tests verify stale results and literal names without changing navigation.
- [x] Update main design, controls and README; fresh code review.
- [x] Follow-up: center file/folder icons, restore bounded cwd prompt, compact
  Desktop spacing/location buttons and fix Options close policy. Actual pointer
  RED/GREEN covers retained Touch targets and repeated theme/density changes.
- [x] Final Windows/Linux tests, strict Clippy, formatting/99-file size check,
  builds/smoke; inspect native screenshot and reopen updated Windows window.

No Git repository exists; no initialization/worktree/commit is requested.
Evidence goes under `.superpowers/sdd/compact-tooltips`.

Final results: Windows 240 passed (two existing subprocess-only fixture ignores
covered by passing parents); Linux 222 passed. Formatting, strict Clippy,
builds/smoke and 100000-byte guards passed. Native compact desktop screenshot
inspected; updated Windows app opened visibly. The test-only Tokio timer-boundary
fix passed 50 repetitions plus a fresh five-test Linux volume suite.
