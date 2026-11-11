# Settings and dialog follow-up — 11 November 2026

This change implements the screenshot follow-up in the existing Rust/Slint
application. It introduces no third-party dependency or plugin ABI change.

- Options uses a taller initial layout, a scrollable plugin catalog and one pinned
  Rescan all plugins action. Package details retain the full path without a
  redundant ID line. Theme labels are alphabetic; persisted theme IDs are unchanged.
- Previews defaults to a 96 logical pixel longest side. Existing saved sizes remain
  valid. The backend publishes the actual thumbnail cache path above Clear cache.
- Editors uses ordered, enabled wildcard/regex associations and retains the old
  editor as fallback. See [the editor contract](2026-11-11-editor-rules.md).
- Editors and Colors share compact, centered list actions. History/cache/extension
  removal uses the same themed red cross. Filename archive presets become yellow;
  only the exact old preset migrates, preserving user color rules.
- Confirmation forms have an internal icon/title/divider. Rename measures its
  summary at the intended initial width; its path is on a separate line. Native
  host modality, target capture, cancellation and overwrite decisions are unchanged.
- Find exposes Whole words beside Containing text without opening More options.
- Ctrl+Shift+P appends the focused native full path without added quotes or spaces.
  Parent/loading rows, relative row text and virtual backing paths are rejected.
  The action uses current shortcut settings and respects command-edit focus.
- Make File List removes DOS 8.3 choices; legacy templates intentionally resolve
  those tokens to full Unicode names/paths. See [Make File List](../MAKE_FILE_LIST.md).
- Raw slash-prefixed mount labels follow alphabetic names within their navigation
  section. Sorting does not modify paths.
- Panel metadata widths use bounded backend samples measured with the actual UI
  font; scrolling does not rescan all entries. See [column widths](2026-11-11-metadata-column-widths.md).
- Filter history retains 100 unique successfully applied mode/text/case combinations;
  typed, canceled, stale and invalid drafts do not enter history. See
  [the history contract](2026-11-11-filter-history.md).

## Validation

The initial rendered pass detected a collapsed Plugins scroll viewport and an
outdated theme-order keyboard fixture; both were corrected before deployment.
The compact Rename regression checks initial sizing, visible controls and the
internal title. Existing operation tests also cover large fonts, all themes,
small windows, long content and Escape/default actions. The final integrated
results and debug build identity are recorded below after qualification.

## Integrated test results

The completed affected Windows frontend batch contains 148 targets. Three stale
fixtures were corrected without weakening their behavior assertions: disclosure
artwork now checks the SVG, confirmation ownership distinguishes modeless progress,
and the Jobs scrolling fixture targets the ledger instead of removed details.
After focused rechecks the batch reconciles to **928 passed, 20 existing ignored,
no unresolved failures**. Six directly affected UI integration targets were run
again against the final corrections: **41 passed**, including native operation
windows, close/cancel, small-window reflow and row input.

The final UI library adds **889 passed, 4 existing ignored** in 8.81 seconds.
Windows backend coverage is **1,857 passed, 47 existing ignored**, and Linux
backend coverage is **1,675 passed, 26 existing ignored**; see the
[backend evidence](2026-11-11-diagnostic-preview-and-pause.md) for original failures,
focused corrections and environment details. Counts refer to those affected suites,
not every platform or every optional external-runtime integration.

Per-target durations, exit codes, artifacts and rendered images are retained under
`_build/qa/viewer-settings-20261111/`, including `final-frontend-reconciled.json`,
`final-corrections-ui-results.json` and `final-corrections-ui-unit-executable.json`.
The production cache was also measured independently; see the
[Jobs history audit](2026-11-11-job-history-audit.md) for its limited benchmark scope.

## Final debug runtime — 12 November 2026

Strict `cargo clippy --workspace --all-targets -- -D warnings`, formatting,
`git diff --check` and the 100,000-byte source limit pass. The source-size check
covered 1,992 handwritten files. Two existing warnings remain inside vendored
FemtoVG; no application/test lint error remains. The last equivalent UTF-16 helper
and test-fixture lint corrections were rechecked in three focused targets:
7 helper, 2 filter-history and 2 progress tests passed.

Fresh debug `me.exe` SHA-256:
`0CAD3979BC07E02DAF5BFECD72DF628ECEA4BE9114F7B97F1E838B29F58E05CE`.
Build log: `_build/qa/viewer-settings-20261111/final-debug-qualified-build.log`.
The companion `midnight-elevated-listing.exe` and independently qualified Markdown,
Source and HTML plugins were installed with the existing filesystem/thumbnail
packages in:

`C:/Users/tjelinek/MidnightExplorer-20261112-progress-final-0cad3979/`

The protected runtime is registered for the exact build hash, so
`D:/experiments/dev/FM/_build/debug/me.exe` locates these qualified packages too.
No source-directory or ancestor ACL was weakened. Backend startup, native-renderer
UI startup and explicit software-renderer UI startup each exited successfully with
empty stderr. User preferences were unchanged. Both main-window snapshots and
compact progress renders were visually inspected. The three smoke logs, snapshots,
and registration result are retained in the same QA directory.

This does not claim that image cold loading or HTML scrolling has reached its
final latency target. HTML wheel-to-draw improved by 21–24% in controlled native
measurements; image loading still has variable worker-startup/publication latency.
Cross-volume Move and sampled-verification Copy intentionally keep ETA at
Calculating until their work budgets can be measured reliably.

Obsolete isolated build trees were removed after validating their absolute paths
and excluding reparse-point escapes. The cleanup freed 32.91 GiB before the fresh
qualification builds; the current build, test evidence and pinned SDKs remain.

## Subsequent transfer ETA qualification

The [12 November accounting follow-up](2026-11-12-transfer-eta.md) adds approximate
ETA for sampled Copy and cross-volume Move using actual outstanding verification
and source-removal proof work. The earlier Calculating limitation above describes
the previous qualified build, not the subsequent implementation.
