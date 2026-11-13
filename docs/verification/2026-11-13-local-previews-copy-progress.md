# Local previews and Copy progress — 13 November 2026

Scope: `codex/filter-linux-drive-list`, based on `c5688c0`, with these two later
corrections. No dependency pins/features, public enums/protocols or preview
preferences changed. All compilation used Debug; existing user operations and
processes were not stopped or restarted.

## Screenshots folder admission

The reported `/home/tjelinek/Pictures/Screenshots` contained 36 regular PNG files.
Saved previews were enabled, network previews disabled, and the left pane used
Thumbnails. The running process loaded the current Debug executable. Its actual
cache and source mounts were writable; cache ownership/private modes were valid.
A read-only cache observation inside Codex's separate sandbox did not describe
the running desktop process and was not treated as its cause.

The actual mount was ext4 on device 252:1, behind `dm-1 → dm-0 → sda3 → sda`.
The physical leaf is a non-removable ATA Samsung SSD. The native provider stopped
at any device with backing slaves and returned `Unknown`. With network previews
disabled, the existing UI admission gate correctly withheld Unknown storage, so
no PNG targets reached the decoder despite both installed thumbnailers.

Both Linux storage implementations now traverse recognized dm/md backing stacks
and normalize partitions. Every leaf must independently satisfy the existing
physical ATA/PCIe NVMe proof. All SSD leaves yield LocalSsd; known mixed SSD/HDD
leaves yield Rotational. Missing, remote, virtual, removable or unknown leaves
remain Unknown. Active-path cycle detection, shared-leaf memoization, cancellation,
depth 32, 128 total nodes and bounded slave iteration prevent unbounded probing.
This work stays in backend services. The UI network/Unknown guard is unchanged.

The new explicit, ignored native regression uses the installed provider, real
storage query, application thumbnail factory, worker service and private test cache.
It only reads the selected source PNG. Before installation it failed on Unknown;
after the Debug provider correction the same source decoded and persisted in a
fresh cache (final run: one pass, 0.13 s). This tests one representative real PNG,
not every file in the directory or the user's live panel repaint.

The existing native build helper publishes completed new inodes with rename.
The old provider and hash were retained before replacement, and the running
application's mapped library was not truncated. Restart is required to use the
new provider in that application session.

Receipts: `_build/qa/local-lvm-thumbnails-20261113/`, including
`native-red.log`, `native-final.log`, `storage-test-results.json`,
`plugin-tests-build.jsonl`, `platform-tests-build.jsonl`,
`native-plugin-debug-build.log` and `native-provider-before-fix.*`.
The native regression requires explicit `MIDNIGHT_NATIVE_TEST_INSTALLATION`,
`MIDNIGHT_NATIVE_THUMBNAIL_PNG` and `MIDNIGHT_NATIVE_THUMBNAIL_CACHE` (under `_build/`).

## Copy window refresh

Copy already had an independent 250 ms timer. Native I/O also samples throughput
at safe points between chunks. The timer sent progress only when its own sampling
call produced a new throughput Arc. If the worker had just sampled that interval,
the timer saw no new Arc and discarded the already-fresh status. Repeated worker
sampling could therefore suppress window updates for seconds. Source review found
no separate five-second UI throttle.

The timer now publishes the latest status even when the worker sampled first.
It retains the existing cadence, nonblocking bounded outbox and wakeup only after
successful delivery. A full queue retries the latest state on the next tick;
no per-chunk UI publication or extra timer was added. Native I/O stalls still
leave byte counters unchanged, with truthful zero-rate/elapsed updates.

Two deterministic regressions failed before correction: pre-sampled 250/500/750 ms
ticks delivered no snapshot, and a full queue lost the next pending update. Both
now pass alongside closed-queue handling, blocked I/O, pause and cleanup ordering.
The tests use the real sampler and production publication boundary; they do not
claim an input-to-paint latency measurement for the user's original live copy.

## Final verification

**75 targeted tests passed, 0 failed, 0 ignored** in the final selected scope:

| Boundary | Passed |
| --- | ---: |
| Production native provider storage graph, physical proof and network mount guard | 9 |
| Legacy Linux storage graph and existing storage guards | 10 |
| Actual installed application/native capture/worker/persistent PNG path | 1 |
| Periodic publication, throughput, pause, blocked I/O and native cleanup ordering | 22 |
| Thumbnail admission, chart rates, ETA and row cache | 24 |
| Progress form rendering and controls | 3 |
| Application visible-sync, wake gate and thumbnail delivery | 6 |

`_build/qa/copy-progress-timer-20261113/` retains `red-tests.log`,
`green-engine-build.jsonl`, `engine-green-tests/summary.json`,
`ui-app-build.jsonl`, `final-ui-app-tests/summary.json` and
`final-debug-build.jsonl`. The initial UI runner used the filename rather than the
Rust ETA module name; it stopped on an unmatched selector before running that
library target. The corrected selection passed and is the final evidence above.

The final Debug app/elevation build passed. A separate native startup/snapshot and
automatic shutdown passed with in-memory settings and owned empty directories;
the snapshot was visually reviewed. The known WGPU discrete-GPU warning persists.
Formatting for both workspaces, whitespace and source-size checks passed.
The post-build source identity, artifact hashes and exact selected commands are in
`build-result.json`; this is not a pre-build snapshot or full-workspace result.
Windows native behavior, arbitrary storage topology and original-copy physical
latency remain outside this Linux verification scope.
