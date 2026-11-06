# System-Locale Sorting Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for core/engine work
> and superpowers:dispatching-parallel-agents for isolated native adapters.

**Goal:** File-panel names, extensions and metadata ties follow the system locale.

**Architecture:** Core stores immutable opaque keys on entries and compares them
without FFI. Engine prepares keys on its listing workers and reuses captured keys
for re-sorts. Separate native adapters capture one session locale.

**Tech Stack:** Existing Rust/Tokio, Windows NLS and Linux libc locale objects.

**Spec:** [System-locale sorting](../specs/2026-11-06-locale-sorting-design.md).

## Global constraints

- Native case and accent weights are retained; numeric/natural sorting is excluded.
- No process-global `setlocale`, lossy native names, ANSI conversion or name truncation.
- Generated keys are limited to 4 MiB each; a missing Linux locale falls back to C.
- OS tests stay in separate files; fixtures and build output stay under `_build`.
- One Cargo process per target platform. Final integration follows the user's
  existing instruction to commit and push the completed work.

## Review focus

- Czech `h/ch/i`, accent and case ordering agree with native locale APIs.
- Invalid UTF-8/UTF-16 names remain distinct through raw fallback ordering.
- Wide-key serialization preserves signedness and terminator/prefix ordering.
- Cancellation during native preparation cannot commit a stale listing.
- Cached re-sorts and size/date ties retain the captured locale and grouping rules.

## Task 1: Portable key preparation and comparison

Files: `midnight-core/src/localization.rs`, `entry.rs`, new `sorting.rs`,
`tests/locale_sorting.rs` and per-OS `tests/support/locale_*.rs` fixtures.

Interfaces: `LocaleService::collation_key(&OsStr) -> Result<Option<Vec<u8>>, FsError>`;
`prepare_sort_keys(&mut [Entry], &dyn LocaleService, &Cancellation) -> Result<(), FsError>`;
existing `sort_entries(&mut [Entry], SortSpec)` remains the comparator entry point.

- [x] Add six tests for key ordering, extension/metadata ties, caches, cancellation,
  raw equality/invalid-name ties and the generated-key resource budget.
- [x] Observe missing API RED, implement worker preparation and immutable caches.
- [x] Run `cargo test -p midnight-core --test locale_sorting --offline`: six pass.

## Task 2: Native collation adapters

Files: per-platform `src/sort_keys.rs`, `sort_keys_tests.rs`, locale capture and lib exports.

- [x] Add native ordering, case/accent, malformed-name and dynamic-buffer regressions.
- [x] Observe Windows missing-helper RED and shared engine behavioral RED; implement
  Windows `LCMapStringEx(LCMAP_SORTKEY)` and Linux `wcsxfrm_l` with checked lengths
  and independent locale capture. Linux implementation follows the shared behavioral
  RED; its missing-helper compiler failure was not separately run.
- [x] Verify Windows Czech contraction and direct-native comparison parity.
- [x] Generate private Linux `LOCPATH` under `_build`; explicitly run Czech tests,
  C fallback and collation independence from unavailable numeric/time locales.

## Task 3: Engine and smoke integration

Files: `midnight-engine/src/lib.rs`, `tests/locale_sorting.rs`, explorer `src/main.rs`.

- [x] Add engine tests proving worker-only native calls, reuse on re-sort and failure
  publication; observe both failing before wiring.
- [x] Pass locale through `ListingTask`, prepare before publication and check cancellation
  after sorting; inject the native locale service into the headless smoke path.
- [x] Run engine locale integration tests: both pass in the Windows all-target suite.

## Task 4: Integration verification and documentation

- [x] Link the spec from the main design and describe native weights, restart scope,
  fallback behavior and key budget in README.
- [x] Run core/common/engine/native all-target tests with `--offline --no-fail-fast`
  and strict Clippy on Windows and Linux.
- [x] Run Windows critical copy/mutation tests with `--release`.
- [x] Run Linux critical copy/mutation tests with `--release`, as required by
  `docs/RELEASE_BUILD.md` for native integrity and queue/cancellation behavior.
- [x] Review native adapters and engine changes; hand off source freeze for final UI
  verification, optimized application build and native screenshot.

## Verification evidence

- Windows core/common/engine/native all-target run: 393 passed, 8 intentional ignored
  across 34 suites; strict all-target Clippy passes. Logs under `_build/qa/panel-shell/`:
  `locale-backend-windows-green.log` and `locale-backend-clippy-windows.log`.
- Windows release copy/mutation checks: 54 pass (common checksum stepper 5,
  engine queues 16, native create/mutation/permissions/transfers 33). Logs
  `copy-common-release-windows.log`, `copy-mutation-engine-release-windows.log`,
  and `copy-mutation-native-release-windows.log`.
- Linux core/common/engine/native all-target rerun: 370 passed, 5 intentional ignored
  across 31 suites; strict all-target Clippy passes. Logs
  `locale-backend-linux-final-green.log` and `locale-backend-clippy-linux.log`.
- Linux release copy/mutation checks: 61 pass (common checksum stepper 5,
  engine queues 16, native mutations/create 32, native transfers 8). Logs
  `copy-common-release-linux.log`, `copy-mutation-engine-release-linux.log`,
  `mutation-native-release-linux.log`, and `copy-native-release-linux.log`.
- Windows native adapter run: 118 passed, 6 intentional ignored; native Clippy passes.
  `locale-sort-windows-all-green.log` and `locale-sort-windows-clippy.log`.
- Linux native sort tests: 5 ordinary tests, 1 explicitly executed private Czech
  locale test, 1 separate-process capture-independence test, and 4 existing locale
  tests pass; strict platform Clippy passes. Logs `locale-sort-linux-green.log`,
  `locale-sort-linux-czech-green.log`, `locale-sort-linux-independent-green.log`,
  and `locale-sort-linux-clippy.log`.
- A first combined rerun stopped during linking because the shared build volume was
  full. Its failure was environmental; the Windows counts above are from the completed
  rerun after regenerable incremental caches were removed.
- The first combined Linux run exposed a timing assumption in a native transfer
  test: a same-length source rewrite retained identical mtime/ctime values
  `1793963357.065742931`. The target contained the original, independently verified
  bytes; the source contained the later bytes. Diagnostic log
  `transfer-stamp-diagnostic-linux.log` preserves the reproduction. The test now
  explicitly advances mtime and asserts that metadata changed before expecting
  rejection, and a separate test covers real cancellation. Production copy logic
  remains unchanged; `SECURITY.md` explicitly describes this timestamp limitation.
  Independent review of the native locale implementation and this test/contract
  correction found no actionable issues.
