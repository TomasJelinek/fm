# Operation speed charts implementation plan

**Goal:** Live, bounded speed charts for Copy, Move and Delete in their existing
progress/Jobs window.

**Architecture:** Shared core snapshot types, engine-owned monotonic sampling,
bounded presenter geometry and one reusable theme-aware Slint chart.

**Tech stack:** Existing Rust, Tokio, Slint; no dependency or platform API changes.

**Spec:** [Operation speed charts](../specs/2026-11-10-operation-speed-chart-design.md).

## Constraints and review focus

- Preserve unrelated working-tree changes; no automatic commit or release build.
- Non-UI sampling remains backend-owned; 250 ms cadence, 240 intervals maximum,
  covering the whole operation through hierarchical peak-preserving aggregation.
- Test stalled native calls, pause/resume, retry resets, cancellation and very fast
  completion. Never count verification as additional copied bytes.
- Test independent job histories and minimum-size UI; keep actions reachable.
- Outputs under `_build/`; use the existing debug toolchain/SDK configuration.

## Backend task

- [x] Add core throughput sample/snapshot types and default snapshot
  fields on CopyProgress and OperationProgress.
- [x] Add deterministic failing sampler tests, implement cumulative-counter/time
  deltas, bounded history, terminal/pause semantics and 250 ms sampling.
- [x] Integrate Copy/Move and Delete reporters, including stalled I/O updates;
  update progress construction sites and test actual emitted events.
- [x] Run relevant core/engine tests and strict Clippy.
- [x] Verify whole-operation aggregation, early peaks and current-rate isolation
  after the user's follow-up replaces the initial rolling-window design.

## UI task

- [x] Add presenter and failing actual Slint tests for series, labels, selecting
  different jobs, no-data states and compact layout.
- [x] Add bounded chart geometry/label presentation and a reusable Slint area
  chart in OperationsBody; non-transfer jobs keep their existing layout.
- [ ] Run UI regression tests and inspect rendered light/dark/contrast snapshots.

## Integration

- [x] Independently review sampling semantics and integration ownership.
- [ ] Update transfer/design documentation and record validation evidence.
- [ ] Check formatting/source limits; rebuild debug app and run startup smoke.
