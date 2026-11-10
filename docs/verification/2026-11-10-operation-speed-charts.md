# Copy, Move and Delete speed charts

Scope: shared backend-measured operation activity and selected-job chart. No new
dependency, platform API, plugin ABI, queue authority or checksum-policy change.

## Checks and review

- The initial real Slint regression failed because the progress dialog had no
  accessible Transfer speed chart. After implementation, all three Jobs tests
  passed, including operation switching, cancellation/close behavior and 15
  combinations of five themes and 1000×700, 480×360 and 320×200 windows.
- Render inspection caught a separate visual defect: Slint's default path aspect
  preservation narrowed the history into the middle of short graphs. The chart
  now stretches both axes independently. A renderer-endpoint regression compiled
  against the exact earlier Cargo artifacts failed on a curve starting 58.5 px
  inside the chart, guarding full-width time-axis coverage at every tested size.
- Independent source review caught lost activity when a previously blocked
  native call completes after cancellation. The backend now accounts for those
  real counters while keeping the current inactive labels at zero.
- Review also caught invisible history for positive sub-millisecond work whose
  timestamp rounds to 0 ms. A visible point preserves that sample without
  inventing an elapsed interval.
- A final lifecycle regression covers native resource cleanup: progress must
  not publish a terminal event before the blocking worker and its owned cleanup
  actually return. Heartbeats must still show stalls during that interval.

## Backend qualification

Windows debug: 51 core unit tests passed; 714 engine unit tests passed, 7 ignored.
All 21 throughput regressions passed, including real native Copy/Move and
permanent/Trash deletion routes, blocked calls, cancellation, final cleanup,
waiting, reset, stalls, short jobs and bounded sampling. Whole-history checks
cover three hours, independent early peaks for all three series, growing uniform
bucket resolution, partial buckets, irregular real timestamps and terminal peak
preservation. Strict core/engine all-target Clippy passed. The remaining 18
engine integration binaries passed 89 tests before the whole-history follow-up.

The initial broader core/engine all-target invocation exposed the existing
`folder_sizes::full_scan_supersedes_a_blocked_individual_scan_before_replacing_its_cache`
test rejecting a valid final partial report from a superseded scan. The test now
validates that unfinished cancelled report, waits for the replacement scan's
completed 42-byte result, and checks a later sort uses the replacement cache
without rescanning. Folder-size production behavior is unchanged. The complete
folder-size integration suite now passes **20/20**. This qualification still does
not claim a fresh, entirely passing workspace-wide suite.

Integrated UI checks passed: the three Jobs tests include fifteen theme/size
combinations, and the full UI library passed 786 tests. Render inspection covered
the ordinary and compact/high-contrast charts. Final Windows debug build,
protected deployment and native startup/UI smoke tests passed; see the exact
[artifact and final shared checks](2026-11-10-staging-cleanup.md#debug-executable-and-startup).

## Artifacts

Local outputs are under `_build/qa/operation-speed-charts/` (UI logs and rendered
BMPs) and `_build/qa/operation-speed/` (backend regression logs). They are not
source-controlled. Tests use temporary owned fixtures, not user files.

These charts measure reported operation activity. Delete/Trash progress depends
on the provider's reporting granularity; neither physical disk bandwidth nor
reclaimed storage is inferred. The entire operation is represented by at most
240 intervals with progressively combined historical peaks per existing session
job record. This change does not qualify every other feature in
the broader project or replace platform-native integration tests.
