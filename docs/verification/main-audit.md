# Main-branch audit — 10–11 November 2026

## Scope and provenance

The audit starts at `origin/main`, commit
`5e04723319e341abcbd33adfad15b569b2f1c9b0`, in the isolated
`codex/main-deep-audit` branch at
`C:/Users/tjelinek/.codex/worktrees/main-deep-audit/FM`. Documentation/inventory
commit `1dbe4c761104120876dbab596e2970bf36cf4a69` precedes the audit fixes.
Verified source commit: **962ebb5049f51d7b2e268c213fb4436d5308b621**. The newer dirty source checkout at
`D:/experiments/dev/FM` was outside scope and was not used as the source of fixes.

This main snapshot implements browsing, panel prefix search, selection,
preferences, native
activation/terminal/clipboard services, and confirmed rename/create/delete.
Copy/Move, internal viewers and dynamic plugins are planned at this baseline;
their designs are not evidence of implemented behavior. The audit does not
qualify functionality present only in the newer dirty checkout.

The baseline contains 395 tracked files. The accompanying
[reviewed-file ledger](main-audit-coverage.csv) records every baseline path,
its baseline byte count, reviewer area and review depth:

| Area | Files | Review |
| --- | ---: | --- |
| Core and engine | 64 | Full source, manifest and test reads |
| Four platform crates | 73 | Full source, native ownership, manifest and test reads |
| UI and explorer | 178 | Full Rust/Slint, build helper, test and asset reads |
| Root, build/CI and documentation | 80 | 70 text/configuration/document reads; 10 historical PNG references checked |

These are reviewed-file counts, **not executable, branch or line coverage
percentages**. Historical screenshots were not requalified as current native
rendering evidence. The 58-file baseline Markdown review overlaps these areas;
it is not an additional 58 files. New audit files are outside the baseline count.

Detailed local evidence is retained under `_build/qa/main-audit/` in the original
workspace. Generated binaries, raw logs, benchmark fixtures and caches remain
untracked. This report retains the conclusions and measurement limits needed to
understand the result without treating those local artifacts as shipped files.

## Confirmed findings and changes

| Finding | Evidence and change |
| --- | --- |
| Rejected commands bypassed bounded output through one waiting task per rejection | All eight rejection families accepted a 2,048-command burst behind a stopped consumer. Rejection delivery now awaits the existing bounded dispatcher. Closed output releases that wait; no unbounded response mailbox replaces it. |
| Native/scan capacity could be released before result delivery | A stopped consumer allowed two Properties calls where the outstanding budget was one. Delivery and real native execution now share ownership of the permit; entry-size accounting remains live through terminal delivery. |
| Dropping the engine did not cooperatively cancel all worker classes | Listing, full/individual scans and clipboard regressions failed on engine abort. Request/controller Drop guards now cancel their tokens. Panic-safe fixture guards make intentional failing regressions terminate. This does not forcibly stop an uninterruptible OS call. |
| Completed/cancelled task records accumulated during sustained navigation | A standalone production-source probe grew to 137,722 JoinSet entries for 100,000 commands. Reaping completed entries during dispatch reduced that probe to 280; a further causal run observed at most 154; the dedicated regression went from failure at 10,000 commands to passing through 100,000. These observed maxima are not formal capacity limits. |
| Meta-modified double click could activate a file | The real pointer regression failed before the guard included Meta. The fix preserves the unmodified-left-double-click activation contract. |
| Large desktop fonts clipped the detail strip | At the supported 32 px font size, measured text required 42 px inside a 22 px viewport. The strip height now follows font metrics; the software-rendered regression covers 11/15/32 px. |
| Stable backend events and refreshes copied unrelated histories | Full preferences are cloned only after tab preferences actually differ. Lazy history models keep compact revision metadata and format requested rows, while stale clicks still resolve through captured identities. Discarded forward histories and complete shutdown preferences use backend retirement. |
| Empty Slint images invalidated otherwise unchanged history rows | A new regression caught this during the interim run: pinned Slint 1.18.1 does not consider two empty Images equal. Absent artwork is represented by `None`; loaded-image comparisons and original notification assertions remain. The corrected regression passes in the final suite. |
| Linux mount deduplication was quadratic | Membership now uses a temporary HashSet while the result Vec preserves first-seen order and raw native bytes. The measured change introduces no cap or silent omission of valid mounts. |
| Selection enumeration repeated a raw-path hash lookup | The iterator uses the ID already yielded by the path map, consults the validated bit position and stops at the cached selected count. Existing identity/index validation and raw-path references remain; nonempty worst-case enumeration remains linear. |
| Native settings size tests failed for the wrong reason | Former 64 KiB whitespace fixtures tested JSON failure after the bound had become 4 MiB. Tests now use valid boundary JSON, size-specific oversized errors and observable read budgets through a shared platform-neutral read-policy helper. |
| Cross-filesystem Trash test assumed TMPDIR differed from `/dev/shm` | The original test failed with `TMPDIR=/dev/shm` because both fixtures were on the same device. Fixture selection now checks fresh owned directories on bounded candidate parents. Local absence of a second device is an explicit prerequisite skip; required CI demands the `QUALIFIED cross_filesystem_trash:` marker. Both native and tmpfs temporary-root reruns passed and emitted the required qualification marker. |

The five new engine causal regressions and the two UI activation/layout
regressions first failed against the corresponding unchanged behavior. The
interim integrated run passed those regressions but was **not fully green** because
of the separate empty-image history-model failure above. The selection change
is a measured performance improvement: its semantic compatibility tests pass
both before and after, rather than manufacturing a failing correctness claim.

Test consolidation preserves every original UI integration assertion: 29 source
modules and 39 definitions, including one ignored benchmark, now share one Cargo
test target. Existing presenter, callback, pixel/layout and native tests retain
their distinct boundaries. PowerShell argument tests share process startup across
33 original invocations while preserving per-case markers, native argument
context, counts, exact bytes and sentinels. CMD tests retain their separate
process boundary. No production dependency or lockfile change is required.

## Native safety, cancellation and collection conclusions

Platform code remains separated behind backend services. Core, engine and app
forbid handwritten unsafe code. The native review found no confirmed P0/P1
ownership or data-loss defect in this snapshot; that result is not a guarantee
against unknown defects.

Windows guards acquire COM/GDI/process/file ownership only after successful
initialization, with scoped release and a meaningful conflicting-COM-apartment
test. Mutation traversal pins source/parent identities, treats reparse points as
leaves and uses handle-relative operations for rename/permanent deletion. Linux
retains descriptors, uses private staging and no-replace publication/restore,
and rejects symlink/mount traversal through `openat2`. Cancellation retains
completed changes and reports residual failures. Windows Shell recycling remains
path-based inside the OS call; Linux staging does not isolate same-UID/root or
previously opened descendant handles. No universal race-freedom claim is made.

Settings use bounded opened-file reads, raw native path encodings, exclusive
temporary publication and sync. Parent-entry power-loss durability remains
outside the promise. Activation remains an explicit service and is not an
immutable-inode guarantee against external replacement. No test empties the
user's Trash or changes system locale/global settings. Mutation/ACL fixtures
are freshly owned, narrowly scoped and restored/cleaned by their guards.

Awaited error delivery intentionally stops command consumption when a live
consumer stops draining a full result queue. Closing that receiver or aborting
the engine releases dispatch. Cooperative Drop cancellation does not declare a
running native call finished; actual native workers retain their permits until
return. The existing shutdown path must continue draining results.

The listing Vec, random-identity HashMaps and immutable selection bitsets fit
their actual access patterns. Small bounded FIFOs/Vectors (32 mutation jobs,
64 entry-size requests per pane, limited child processes and four scan slots)
do not justify more complex concurrent structures. Search preserves immutable
source/revision checks and full identity validation. Reclamation must retain
the closed receiver to handle reserved sends arriving after closure; a one-time
drain is insufficient.

Directory listings, identity maps, watch baselines, histories and concurrent
retained snapshots still consume memory proportional to their contents. Queue
bounds are not a constant-memory promise. Large listing/resort overlap and
streaming UI capacity growth remain profiling targets.

## Measured costs and limits

Windows measurements used Rust 1.99.1 on Windows 11 Enterprise build 26200,
Intel i7-13800H (14 cores / 20 logical processors), approximately 31.6 GiB visible
RAM. Workspace tests used debug profiles, with the existing UI optimization level
1 and renderer-dependency level 2 overrides; backend code remains unoptimized.
Debug symbols and incremental compilation were disabled, with two build jobs and
serial per-binary execution. Concurrent audit work,
process startup and host scheduling affect wall times; no clean-build, idle-host
or release-application speedup is inferred.

| Execution | Passed / failed / ignored | Binaries | Sum of binary wall time | Sum of reported test bodies |
| --- | --- | ---: | ---: | ---: |
| Unchanged Windows baseline | 634 / 0 / 8 | 64 | 344.376 s | 32.839 s |
| Interim Windows after changes, before empty-image correction | 647 / 1 / 9 | 37 | 133.829 s | 40.171 s |
| Final Windows production corrections | 649 / 0 / 9 | 37 | 108.222 s | 41.349 s |
| Final Linux workspace | 625 / 0 / 3 | 37 | 22.798 s | 20.042 s |
| Earlier Linux platform/common/facade run, native ext4 fixtures | 135 / 0 / 0 | 13 | 1.872 s | 1.327 s |


Individual test outcomes and elapsed body times are retained in the
[Windows baseline](main-audit-windows-baseline-tests.csv),
[Windows corrected sweep](main-audit-windows-tests.csv),
[final shared-helper UI rerun](main-audit-ui-helper-tests.csv), and
[Linux corrected sweep](main-audit-linux-tests.csv). Empty timing cells identify
ignored tests, not zero-cost execution. UI integration names gained their source
module prefix after consolidation; all original definitions remain. The final
helper-only rerun supplements the full Windows sweep rather than inflating the
649 distinct passed-test count.
These are libtest executable results, not a claim that doctests, all ignored
tests, release builds or every OS path executed. Test-body sums exclude process
startup. The changed target count and added regressions prevent treating the
interim totals as a controlled application performance result. The merged UI
target separately passed all 38 active tests with default parallelism in 2.23 s
and passed serial execution; the ignored benchmark remained ignored. After strict
Clippy identified duplicate helper module loading, all seven helper sources were
registered once and 22 consumers changed to imports. Test bodies stayed unchanged;
the resulting UI target passed again serially (38 active tests) and in parallel
(1.63 s). No lint suppression was added.

PowerShell's measured test body fell from 7.917 s to 0.888 s in the final run,
with the same 33 native argument cases. This is one before/after observation,
not a stable latency budget.

Linux qualification ran as the ordinary `tjelinek` user (uid 1000), not root,
on Ubuntu 24.04 / WSL2 kernel `6.6.87.2-microsoft-standard-WSL2`. Main fixtures
used `/tmp/_build/midnight-main-audit/tmp` on the distro's native ext4 filesystem;
the second-device fixture area was `/dev/shm/_build/midnight-main-audit`.


The Linux build investigation independently reproduced a WSL cache problem on
this host's `/mnt/d` (DrvFs/v9fs) target. Explicit timestamp updates lost subsecond
precision. Cargo observed libm's build-script executable 198.6 ms newer than its
rewound output and reported `StaleDependency`, causing an unchanged dependency
to rebuild. A native Linux timestamp-only control preserved precision; a later
isolated Cargo run became fully fresh. This proves that specific invalidation,
not that every rebuilt unit had the same cause. Use a native Linux checkout and
its `_build/` directory for future WSL work; do not manipulate fingerprint times
or weaken Cargo checks. Existing targets and registry source were not altered.
Five tiny offline fixture builds and the timestamp controls are recorded locally
in `cache-diagnosis/report.md` and its raw logs.

UI performance uses actual event application and refresh with eight stable saved
tabs per pane. Rasterization, setup and deferred backend destruction are excluded.
Values below are median microseconds; the event sample has 1,000 observations
and refresh has 100. These are debug software-renderer measurements.

| Previous paths / command-history bytes | Event before | Event after | Refresh before | Refresh after |
| --- | ---: | ---: | ---: | ---: |
| 0 / 0 | 2.5 | 2.5 | 87.6 | 86.1 |
| 0 / 1,048,576 | 229.0 | 2.6 | 9,403.4 | 86.4 |
| 10,000 / 0 | 2.7 | 2.6 | 15,653.4 | 88.5 |
| 10,000 / 1,048,576 | 273.6 | 2.6 | 30,292.6 | 88.4 |

The final row's refresh p95 changed from 48,923 to 93.9 us. The after run executed
with compilation idle; the pre/post host load is not identical. Actual backend
selection toggles remained about 2.6 us at 10k and 12.2 us at 100k rows (before:
2.6 and 12.3 us), separate from the selection-enumeration optimization below.
Do not infer native GPU frame latency from these numbers. The separate 10k-row
interaction diagnostic used an explicit 25 ms acknowledgement delay and software
rasterization. Its first-frame median increased from 28.033 to 31.909 ms, and
resize raster median increased from 27.590 to 38.151 ms across the recorded
24-sample runs. Immediate selection remained 24/24. These adverse observations
are retained; no renderer improvement or causal attribution is established by
these runs. The injected delay also prevents interpreting first-frame time as
uncontaminated backend/renderer latency.


Three subsequent alternating before/after process pairs retained 24 samples per
run. The before first-frame medians ranged 30.903–32.211 ms and after medians
30.145–35.726 ms. Before resize-raster medians were 31.235–34.360 ms; after were
31.929–47.631 ms. The first two pairs were comparable; the last after run was
slower. Thus these software-raster measurements remain variable and do not
establish a renderer improvement or a consistent regression attributable to the
model changes. All six runs retained immediate selection in 24/24 interactions.

The isolated mount parser benchmark used the exact old/new module at `-O`, one
warmup and seven measured repetitions. At 10k distinct mounts, median parser
time changed from 127.46 to 2.02 ms (ordered) and 173.65 to 2.01 ms (shuffled).
At 50k ordered unique roots, process peak working set was about 12.60 MB before
and 16.04 MB with the temporary set; that includes fixture/runtime memory, not
allocator-exact usage. At 33 roots it changed from about 5.0 to 6.9 us, and 50k
identical roots changed from 4.02 to 5.16 ms. The tradeoff removes quadratic
growth while preserving order. Slow original 50k-unique cases have only one
measured sample and do not support a p95 claim.

Collection experiments covered 10k/100k/1M entries with non-dense IDs, 25-byte
short paths and 109-byte Unicode paths. The initial core artifact was debug,
while generic comparisons used an optimized harness. At 1M Unicode entries,
fixture plus index used 822.71 MiB process private bytes; this excludes other
application snapshots/panes. The clone experiment was stopped after a sampled
1.007 GiB private-byte peak and is not a completed timing. Later harness runs
skip that case and use a 900 MiB early-stop guard. These are experiment limits,
not application limits or actual allocation totals.

The focused selection comparison compiled the exact before/after methods at the
same optimization level. With 1M Unicode paths, optimized sparse-six enumeration
changed from 699.304 to 166.305 ms median; all-selected changed from 617.266 to
143.822 ms. At 10k short paths, sparse-six changed from 0.747 to 0.129 ms. The
seven-observation p95 is only the sample maximum. The benchmark excludes path
cloning, the Properties consumer's visible-index validation and native UI;
these are not end-to-end Properties speedups. Empty enumeration becomes
constant-time, but Properties already bypasses it for empty selection.

HashMaps remain justified for raw-path lookup and large random ID lookup.
Sorted ID tables won at 10k/100k in the measured fixture, while hashing won at
1M; sorted raw-path lookup lost at every size. This supports keeping the present
structures rather than claiming one representation is universally fastest.

## Dependencies and documentation

The new [dependency policy](../DEPENDENCY_POLICY.md),
[pinned inventory](../THIRD_PARTY.md) and
[feature/license CSV](../third-party-inventory.csv) record 561 registry packages,
including 26 direct workspace dependencies. Eight local crates are excluded
from the third-party count. Windows/Linux build-and-test graphs contain 344/449
nodes including local crates; graph membership does not prove release shipment.

No mandatory-fee dependency was introduced and no project license was selected.
The inventory records Slint 1.18.1's free desktop Royalty-free 2.0 alternative,
its attribution/API/platform restrictions and the existing AboutSlint surface;
it does not call those terms permissive open source. Actual release license and
NOTICE texts still need to accompany the distributed artifacts. The inventory
is not a completed distribution notice bundle.

Cargo-audit 0.22.2 against RustSec revision
`28d73fc95e5972af3403225a217def57dcaba3c4` reported zero known matching
vulnerabilities and six unmaintained-package advisories: bincode 2.0.1,
pin-utils 0.1.1, ttf-parser 0.25.1, zbus_names 4.3.5, zvariant 5.15.1 and
zvariant_derive 5.15.1. These maintenance risks are retained in the inventory;
the scan does not cover unknown defects or all OS/native build artifacts.

The new [documentation index](../README.md) distinguishes current contracts,
rolling architecture, accepted future plans and historical verification.
Corrections cover implemented mutations, preference schema 6 and optional exact
command-history persistence, viewer-plan precedence, tab persistence, shutdown
grace and software-renderer limits. Earlier screenshots, commands, test counts
and code-generation sizes remain dated records. The final documentation pass
checked 181 relative link paths in 63 Markdown files with zero missing targets;
external URLs, anchors and private artifact references were outside that check.

## Reproduction and outstanding qualification

From the audit checkout, with the pinned toolchain and caches already populated,
keep Cargo output and temporary fixtures under `_build/`. The audit used
`CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0` and
`CARGO_PROFILE_TEST_DEBUG=0`. Use a native Linux filesystem for Linux permission,
symlink and Trash fixtures; WSL's DrvFS fixture failures are not valid native
Linux qualification.

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo test -p midnight-ui --test integration --locked --offline -- --test-threads=1
cargo test -p midnight-ui --test integration --locked --offline
cargo test -p midnight-ui --lib --locked --offline model_tests::performance::stable_tabs_and_history_presentation_performance -- --ignored --exact --nocapture
cargo test -p midnight-engine --lib --locked --offline selection::tests::interaction_selection_cpu -- --ignored --exact --nocapture
```

Run `pwsh scripts/check-source-size.ps1` on Windows or
`bash scripts/check-source-size.sh` on Linux. The build/test helper environment
and raw executable-timing method are recorded in local `windows-env.ps1`,
`run-timings.ps1`, `linux-platform-build.sh` and `run-linux-timings.py`. Timing
runners enable libtest JSON reporting only in already compiled child processes;
they do not use `RUSTC_BOOTSTRAP` to build the application. The collection/mount
reports and harnesses under `collections/` and `platform/` identify exact source
versions, compiler modes, fixture construction and independent assertions.

The dedicated Linux cross-filesystem test is
`mutation::tests::cross_filesystem_trash_returns_unsupported_and_keeps_source`.
Run the same built executable with its exact name and `--exact --nocapture`,
once with native ordinary TMPDIR and once with `TMPDIR=/dev/shm`; require both
successful exit and `QUALIFIED cross_filesystem_trash:` in each log. A local
prerequisite skip reported by libtest as `ok` is not cross-device evidence.

Final verification passed on both Windows and Linux: normal parallel workspace
execution, doctest targets and strict all-target Clippy. The full serial timing
sweeps are listed above. Windows also reran the consolidated UI target after the
helper import correction, both serially and with default parallelism. Rustfmt,
patch whitespace and the source-size check passed; all 273 checked source files
were at most 100,000 bytes.

Both debug application builds passed a filesystem smoke run against the same
checkout and its `crates` directory: each reported 18 and 8 directory entries,
respectively, with 3 Windows volumes and 15 Linux mounts. These are filesystem
service checks, not interactive native-window qualification. No optimized release
build was produced during this audit.

Explicit qualification limits:

- The first Linux platform run used DrvFS fixtures and had 24 failures; repeating
  on native ext4 passed 135 tests. The subsequent full native Linux suite passed
  625 tests. Native GPU visual qualification is separate.
- Two explicitly invoked optional Windows recycle/restore tests failed because
  this host reports Recycle Bin unavailable for the fixture location
  (`0x80270036`). The long-path no-permanent-fallback test passed. No global
  Recycle Bin configuration was changed; successful recycle/restore remains
  unqualified here.
- Historical multilingual screenshots and software layout tests do not qualify
  current native Unicode shaping, DPI, accessibility, GPU presentation or
  native scroll/resize latency. No new native visual conclusion is implied.
- Linux bind mounts, Windows case-sensitive directories/UNC edge cases,
  wide-directory cancellation under resource pressure, and persistent native
  volume-buffer retry errors remain useful targeted qualification work. Static
  hypotheses without a reproducer were not promoted to confirmed defects.
- Large streaming listings, full application release memory/latency, cold clean
  build/link costs, a new release artifact and distribution notices require
  separate measurement or packaging evidence. The central architecture document
  is already lengthy; future large additions should be split by responsibility.

Local evidence index: `agents/{engine,platforms,ui}.md`,
`proposals/docs/REPORT.md`, `baseline-tests/{tests,binaries}.csv`,
`engine-red/`, `ui-red/`, `green-tests/`, `linux-platform-native-tests/`,
`proposals/fix-review/joinset-*.log`, `collections/report.md`,
`collections/selected-paths-report.md`, `ui-performance-{before,after}.log`,
`ui-parallel.log`, `windows-trash-tests.log`, `cargo-audit.json`,
`docs-final-links/links.json`, and the corresponding final rerun artifacts.

## Windows CI follow-up — 11 November 2026

The Windows CI run for `54dd660` timed out after 15 seconds in the batched
PowerShell argument test. Investigation found that the shell-test helper waited
for process exit before draining its stdout/stderr pipes. A fresh owned fixture
writing 128 KiB to each stream reproduced a pipe-backpressure deadlock: the old
helper failed after 15.03 seconds. The original 33 PowerShell cases emit only
3,010 ASCII bytes with CRLF, so this reproduction proves a helper defect but
does not establish the precise cause of that historical hosted-runner timeout.

The helper now drains both streams concurrently, retains at most 1 MiB per
stream plus one overflow-detection byte, and drains/discards excess output before
reporting its capture-limit error. The unchanged 15-second deadline still kills
and reaps the direct child. Timeout diagnostics now include bounded stdout and
stderr excerpts. These are trusted test fixtures, not a general process-tree
supervisor or an application execution change. All 33 argument cases, markers,
byte comparisons and sentinels remain unchanged.

The affected Windows target passed **6 active tests and 2 ignored subprocess
entry points** with normal parallelism (2.48 s) and serial timing (2.769 s).
The new pipe regression took 0.047 s, capture-limit/drain regression 0.001 s,
PowerShell round trip 0.847 s and CMD round trips 1.872 s in that serial run.
The ignored child entry points are invoked explicitly by their parent tests.
Strict core all-target Clippy, workspace formatting and `git diff --check`
passed. Earlier full-suite counts and CSV measurements above remain unchanged;
this is a focused follow-up, and hosted CI must independently qualify the fix.
Local evidence is under this audit checkout's
`_build/qa/thumbnailer-settings/audit-ci-{failure,shell-red,shell-green,shell-clippy}.log`
and `audit-ci-shell-timings.jsonl`. No dependency or production code changed.