# Explicit diagnostic report factory integration

Implemented against baseline `280c49a`; scoped producer tests and strict lint passed. Native report
ownership reproduction and arena internals are recorded separately in the
[report storage evidence](2026-11-12-report-storage-retirement.md).

## Producer routes

Every native report creator now receives the startup `ReportArena` explicitly:

- `folder_sizes::compute_batched` receives a borrowed factory; each worker lane
  retains the existing independent lazy append log and sequential root ranges.
- `entry_sizes::Services`, full-folder work, and `tree_sizes::State` carry the same
  factory into native scans. Tree fallback diagnostics use it for exact failed rows.
- `archive_sizes::Controller` passes the factory through each native size worker.
  `scan::ScanScope` groups the existing address, ancestor and reserved-entry checks;
  it changes no revision, identity, cancellation, traversal or publication rule.
- Folder-presence probes carry the factory in their native worker context.
- Tree model workers and sessions retain explicit factory clones for listing,
  construction, refresh, watch and terminal action diagnostics.
- Public `DirectoryTree::new(location, limits, reports)` now requires the caller's
  arena. Standalone users obtain it from `EngineLifetime::report_arena()`. There is
  no per-tree worker creation, implicit default, process global or thread-local.

Selection-size aggregation and composed diagnostic sources only retain existing
reports; they do not allocate native report files and need no factory parameter.
No plugin ABI, warning text format, output truncation or authority contract changes.

## Ownership review

Independent review of `report_arena.rs` and `size_warnings.rs` found no actionable
issue. Registry ownership precedes writer exposure; raw writer and prefix references
release before their shared token. Actual native payloads contain no back-reference
to the factory. Cleanup removes payloads under the registry mutex and destroys them
after unlocking; the producer's State-to-registry/storage lock order has no inverse
cleanup path. Startup failure exposes no factory/native file. Unexpected cleanup
worker exit refuses new admission while existing tokens retain guarded fallback
cleanup. Nodes are reused only after destruction returns or unwinds.

This is admission proportional to retained or retiring report files, with reusable
metadata at their high-water mark. It does not impose an arbitrary report-count cap
or merge unrelated reports into one failure domain.

## Focused evidence

Existing tests retain complete Unicode branch failures through retry and tree
teardown, preserve cancelled native/archive scan reports and immutable ranges, and
keep exact cached watch-diagnostic identity. The refresh aggregate regression now
clones its published source, drops both terminal diagnostic and complete originating
fixture/session, then reads and checks both full branch failures.

The first Windows/Linux compile found one migrated test helper call using the new
`ScanScope` shape while its test-only `compute` helper kept the previous signature.
That fixture call was restored; production and the actual `compute_checked` callers
were unchanged. The initial compiler logs are preserved by the platform gate owners.
Final targeted test and lint results are recorded below.

The first Linux producer/integration pass completed 57 integration tests with one
existing ignored case. The later arena empty-reservation correction and CMD-01
shared changes were subsequently included in the final combined gate below.

Final Linux source passed 140 focused producer/tree cases: archive sizes 32, native
folder sizes 36, folder presence 6, provider tree 18, tree size capture/runtime 16,
refresh 22, controller 8, and terminal diagnostics 2. The shared arena/storage cases
passed 28 additional tests. Five affected integrations passed 57 tests with one
existing ignored case. Logs are
`_build/qa/report-arena-20261112/linux-final-lib-*.log` and
`linux-final-integration-*.log`; use the actual named tree actor filters rather than
the earlier empty `tree_sizes` filter. Strict engine and Linux platform all-target
Clippy passed with `-D warnings` in 1 m 20 s, recorded in `linux-final-clippy.log`.
The Windows focused report gate also passed 58 cases in 0.80 s (build 22.23 s),
recorded in `windows-report-tests.log`.
