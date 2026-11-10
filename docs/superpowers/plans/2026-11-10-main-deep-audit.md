# Main branch correctness, concurrency and performance audit

**Goal:** deeply review all repository code, fix demonstrated defects, measure
data structures and test runtime, remove proven redundancy without losing useful
coverage, and reconcile documentation with implemented and tested behavior.

**Baseline:** fetched `origin/main`, commit
`5e04723319e341abcbd33adfad15b569b2f1c9b0`, 10 November 2026. There are 395
tracked files. Work proceeds on `codex/main-deep-audit` in an isolated checkout.
The newer `codex/viewer-plugins` branch and its uncommitted work remain untouched
and are outside this requested main-branch audit.

**Architecture:** preserve the existing Rust/Slint/Tokio separation. Backend,
platform and UI audits run independently; integration, timing and documentation
have one coordinator. A file inventory records actual coverage, concrete
findings, reproductions and remaining verification gaps.

## Constraints

- Rustfmt width 100; handwritten source at most 100,000 bytes.
- Keep platform implementations separate and all non-UI work in backend services.
- Keep source identities, cancellation, error reporting and security boundaries.
- No mandatory-fee dependencies or silent licensing changes. Check pinned
  dependency metadata and distribution obligations; prefer no new dependencies.
- Generated output belongs under `_build`; shared toolchain caches under `.tools`.
- Use only owned test fixtures. Never empty user Trash, exhaust a real disk,
  weaken ACLs/antivirus, or delete unrelated data to simulate failures.
- Benchmark before/after with identical input, profile and measurement method.
  Report startup/process overhead separately from test body time where relevant.
- Do not delete tests just because their names or implementations look similar;
  demonstrate equivalent observable coverage first.

## Review focus

1. Saturated channels, stalled consumers and cancellation must not create an
   unbounded number of workers, retained buffers or pending responses.
2. Stale generations, concurrent namespace replacement and late callbacks must
   not redirect destructive operations or act on newer UI state.
3. FFI resources must have exact ownership and thread affinity, including errors,
   panics and shutdown; OS guarantees must not be overstated.
4. Very large directories, Unicode paths and slow storage must remain bounded
   and responsive; structure changes require workload evidence.
5. Tests must exercise behavior, isolate mutable state and synchronize on events;
   timeouts are failure limits rather than substitutes for synchronization.

## Execution and evidence

- [ ] Inventory every tracked source/test/build/documentation file and assign ownership.
- [ ] Record clean baseline build, normal test results and per-test runtime.
- [ ] Audit all core/engine sources, contracts, structures and tests.
- [ ] Audit platform common/facade/Windows/Linux sources and native test boundaries.
- [ ] Audit UI/application sources, model costs, callback lifetime and tests.
- [ ] Reproduce concrete findings; apply focused fixes and meaningful regressions.
- [ ] Measure data-structure alternatives and test-runtime improvements.
- [ ] Review dependencies, scripts, CI and all maintained documentation claims.
- [ ] Run final Windows and available Linux checks, lint, formatting and source limits.
- [ ] Independently review fixes and publish evidence with explicit platform gaps.

The current user request authorizes investigation, fixes and verification. Routine
implementation choices and baseline failures do not require a new approval round.
