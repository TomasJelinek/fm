# Panel warning details, 2026-11-10

Native/archive/tree size reports and complete tree failures are connected to
panel diagnostics. The focused gates below qualify this change; the deployment
checkpoint is recorded separately after the final integrated build.

## Behavior

- Yellow size warnings show the first actual issue and counts in their tooltip.
  Clicking opens the immutable report with paths, operations and reasons.
  Running/cancelled estimates without real failures have no warning icon.
- Red tree-listing errors and panel error icons expose details independently of
  file selection, activation, drag and retry. Failed branches keep separate
  Retry/Collapse controls. All three panel modes and both panels are covered.
- Gestures retain row/context/report revision. Rebinding a row or replacing an
  error with an identical summary cannot retarget a held click.
- Full errors use the basic generated-text viewer; short errors use the error
  dialog. Error Copy retains the complete source rather than copying a bounded
  summary. Native reads, snapshotting, clipboard encoding and final report
  destruction remain on backend workers.
- Invalid native units and control characters remain distinguishable in error
  paths. An unavailable report or a storage/queue/viewer failure is explicit.

The [developer contract](../developer/PANEL_DIAGNOSTICS.md) describes ownership,
concurrency, cancellation and backing-file lifetimes. No dependency or plugin ABI
change is introduced.

## Regression evidence

Logs are under `_build/qa/size-warnings/`.

- `ui-red.log` reproduces warning icons without an accessible detail action.
- `ui-green-3.log` revealed a lost Retry control after replacing the failed
  disclosure with a detail icon. `ui-green-4.log` passes all nine warning,
  tooltip and disclosure tests after separating those actions.
- `final-warning-review-red.log`: nine presenter tests pass; four fail on
  lossy fallback paths, copying only the summary, stale copy and queue refusal.
  Those failures drive the final copy/path corrections.
- `viewer-title-red-2.log` reproduces diagnostics receiving the hardcoded Make
  File List title. The generated-source label now follows its captured title;
  it is never opened as a filesystem path.
- `viewer-ui-retirement-red.log` reproduces final source destruction on the UI
  thread for rejected viewer openings. The fixed paths retire the launch through
  the backend.
- `generated-clipboard-retirement-green-linux.log`: 17 clipboard tests pass,
  including rejected generated-source destruction on a blocking worker.
- `error-path-green-windows.log` and `error-path-green-linux.log`: four actual-
  source formatter checks pass per platform, including unpaired UTF-16/invalid
  UTF-8 and control characters. These narrow harnesses do not replace the full
  UI library gate.
- [Backend diagnostics evidence](2026-11-10-tree-diagnostic-details.md) records
  the Linux core/scanner/spool/Engine route gates and their explicit opt-ins.

## Final focused Windows gates

- `windows-libs-green.log`: core **51**, UI **764**, viewer-engine **111**
  (four installed-plugin opt-ins ignored) and viewer-UI **43** tests pass. This
  initial command did not pass overall: four engine tests exposed a real
  concurrent first-use private-directory creation race.
- `archive-tree/windows-spool-first-use-red.log` reproduces eleven rejected
  writers out of twelve. The native fix accepts only the AlreadyExists race,
  then reopens and applies the existing no-reparse/ownership/DACL checks.
  `archive-tree/windows-full-first-use-green.log` passes **693 engine** tests
  (seven installed-catalog opt-ins ignored) and **10 worker** tests against a
  fresh private temporary root. `windows-store-safety-green.log` in that same
  subfolder passes **3** storage security cases.
- `windows-engine-routing-green.log`: **2** real Engine report/cancellation
  boundary tests pass.
- `windows-ui-regressions.log`: **67** of 70 tests across 21 integration targets
  pass initially; three old fixtures expected text roles or bypassed presenter
  authority. Updated fixtures exercise the real error event and new accessible
  buttons. `windows-ui-final-corrections.log` passes all **9** tests in the two
  corrected targets. Together these qualify all 70 integration cases without
  weakening geometry, full-error copy or operation-authority assertions.
- The later JSON renderer/engine qualification supersedes the viewer counts
  where those shared components changed; see its separate evidence.
- `clippy-backend.log` passes strict Clippy for the changed core, engine, UI and
  worker libraries. `format-final.log` passes workspace formatting after formatting
  the updated error fixture; `source-size-final.log` checks 1,706 source files,
  all at most 100,000 bytes. Later additions are rechecked before deployment.

## Integrated debug deployment checkpoint

The Windows debug executable was rebuilt, installed in the protected runtime and
registered for launch from `_build/debug/me.exe`. At this checkpoint its SHA-256
is `5FD645D5650C359497E1E22A7129FEA420C86B95D7B9ACADE90C0FB84E1CA8F2`.
Both the actual build-path startup smoke and isolated native UI startup passed.
The visually inspected snapshot is
`_build/qa/size-warnings/native-startup-e54a7456e4ce447e96eee5319268f269.png`.
Both panels, the main/bottom toolbars, aligned columns and corrected IEC units
are present. The snapshot uses the diagnostic software renderer; focused UI
tests above establish warning actions, rather than this ordinary-directory image.

The first snapshot rerun safely refused to overwrite an existing PNG (`os error
80`). The harness now chooses a unique filename; no production change was needed.
Release optimization, Linux desktop interactions and the broader feature backlog
are separate work. Any later cold-start optimization is recorded with its own
artifact and follow-up smoke results.

The subsequent bounded-viewer/startup optimization is deployed as
`C05ADAF9BABD742FA00E95069D9F72D840AEE26F3646C28844DED4F103DBC66A` with both
startup checks passing again. The final inspected native startup snapshot is
`_build/qa/size-warnings/native-startup-682ea3acca1a42ca9dae6a2b44379019.png`.
Final workspace formatting passes and the source-size check covers 1,714 files.
See [large-file qualification](2026-11-10-large-text-viewers.md) for the new
artifact, permission-cache regression evidence and real 10 GiB measurements.
