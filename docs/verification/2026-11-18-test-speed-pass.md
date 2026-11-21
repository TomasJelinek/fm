# Test speed pass — 2026-11-18

## Scope

Follow-up to [test execution time](2026-11-18-test-execution-time.md) on a branch
based on main `2e6ee4ff`: cut the test execution time that
the timing of `2e6ee4ff` showed to be fixable on Windows and Linux, without losing
coverage or isolation. Measured on 19 November. Build and link time are not included,
except the compile cost of the one profile change.

The timing of `2e6ee4ff` (the orchestrator's runner, as in the earlier record; results
in the measured worktree's `_build/qa/timing-main` and `_build/qa/timing-main-linux`)
ran each of the 69 test executables alone. On Windows they took 44.8 s when each had
started moments before, and about 254 s in the first run after a full relink: a probe
started a copy of every executable with a new content hash and found 209.5 s of start
overhead, about 1.3 s plus 0.17 s per MB (0.8 to 2.2 s for the executables below 5 MB,
17.6 s for the 108 MB `me` test executable). Byte-identical copies cost about a fifth of
that (44.4 s), a later start of the same file 0.01 to 0.1 s, and after some minutes
without a start an executable could pay much of it again. On Linux (WSL 2) they took
31.4 s, with no first-start cost. Its findings that this pass acts on:

- `file_comparators::matching::comparator_aggregate_budget_does_not_reduce_panel_or_editor_defaults`
  took 4.9 s on both platforms, almost all of `midnight-platform-common`'s integration
  executable.
- `desktop::rejected_work_cannot_accumulate_unbounded_responses_behind_a_stopped_consumer`
  waited out nine 200 ms windows one after another (1.9 s), the longest test of the
  engine integration executable on Linux.
- Twelve executables pass no test on Windows; six of them (three targets without any
  test, and three Linux-only files of `midnight-worker-platform`) cost a first start
  after every relink for nothing.
- `midnight-engine`'s integration executable runs 162 of the library's unit tests again
  (kept, see [Not changed](#not-changed)).

An earlier, unmerged attempt (branch `worktree-wf_1a6e26e5-02d-1` on `d3817ea5`)
contributed two reviewed commits, cherry-picked here without conflicts and re-verified
on `2e6ee4ff`; its lengthened TEST-03 deadlines were not taken, because the root-cause
fixes on main replace them.

An independent review of the first version of this record found that it overstated the
Windows first-start saving and counted noise as gains, and that running the rejected-work
kinds together could narrow what that test detects. This version corrects the first two
from the measurements (and a second session, see [Method](#method)); the rejected-work
test now gives each kind a thread of its own and a 400 ms window, from a probe of its
detection margin; and the two binaries without a test harness gained guard tests.

## Changes

| Change | Executables | Evidence |
| --- | --- | --- |
| `test = false` for the `midnight-elevated-listing` and `midnight-worker-probe` binaries and the `midnight-platform` library, which have no test on any platform (cherry-picked `d43c1aaf`). A guard test fails when a unit test is added to one of them: `library_has_no_unit_tests` in the platform integration executable, `listing_binary_has_no_unit_tests` and `probe_binary_has_no_unit_tests` in the `midnight-elevation` and `midnight-worker-platform` library tests | 3 fewer on both platforms | they listed no test; first start of a relinked copy on Windows about 1 s each ([Results](#results)); each guard failed on a planted unit test |
| The platform junction fixture runs `cmd /d /v:off /c mklink /J` instead of Windows PowerShell (cherry-picked `e7c60455`) | `midnight-platform::test:integration` | junction test 0.30 → 0.06 s (Windows) |
| `regex-automata` builds at `opt-level = 2` in the dev and test profiles | every executable that matches with `regex` | comparator budget test 5.05 → 0.51 s (Windows), 5.10 → 0.43 s (Linux) |
| The nine command kinds of the rejected-work test run at the same time, each on a thread and current-thread runtime of its own, in one 400 ms window | `midnight-engine::test:integration` | 1.95 → 0.43 s (Windows), 1.83 → 0.42 s (Linux) |
| `spool_linux`, `store_linux` and `worker_policy_linux` of `midnight-worker-platform` become the `linux` module of the `spool`, `store` and `worker_policy` executables (`tests/linux/*.rs`) | 3 fewer on both platforms | Windows built and started them without a test; first start of a relinked copy 1.14, 1.08 and 1.48 s |

The binary guards read only the binary's entry point (`src/main.rs`,
`src/bin/midnight-worker-probe.rs`), so they also fail on a module file declared there.
The listing guard failed on a planted `#[cfg(test)]` module and a planted
`#[tokio::test]` function, the probe guard on a planted `#[test]` function and a planted
`mod planted;`.

### Comparator budget test

The test compiles `a{10000}` as a panel filter, matches 10,000 `a` with it, validates it
as an editor rule, expects 64 copies to exceed the comparator budget and compiles the
panel filter again. A standalone probe of these steps with `regex` 1.13.1 (built like the
workspace tests; source and builds in `_build/qa/speed-pass/regex-probe*`) timed each
step:

| Build | 4 compilations | `is_match` of 10,000 bytes | All steps |
| --- | ---: | ---: | ---: |
| All at `opt-level` 0 (before) | 17–21 ms | 4.37–4.79 s | 4.39–4.81 s |
| `regex-automata` at 2 | 6 ms | 0.28–0.29 s | 0.29–0.30 s |
| `regex-automata` at 3 | 6–7 ms | 0.29–0.30 s | 0.30 s |
| `regex`, `regex-automata`, `regex-syntax`, `aho-corasick` and `memchr` at 3 | 5 ms | 0.29–0.30 s | 0.30 s |

The unanchored search builds a lazy DFA state for each byte, each holding every partial
match so far, so its cost grows with the square of the repetition: unoptimized, 0.23 s
for `a{2000}` and 1.78 s for `a{5000}`. The expression stays: the comparator's 128 KiB
per-engine budget for 64 rules refuses `a{n}` from n = 1636, and the 2 MiB default
accepts the case-insensitive editor pattern below n = 26212 (the case-sensitive panel
pattern below 65530), so 10,000 keeps margins of 6 and 2.6; a smaller n would narrow what
the test checks rather than its cost. `regex-automata` compiles in 9.6 s instead of 2.3 s
at level 2 (10.3 s at 3; one job, `cargo --timings`), and the profile change rebuilds its
dependents once. Panel, tree and Find name filters and comparator and editor rules all
match with `regex`, so debug builds of the application match faster as well.

### Rejected work

Each of the nine command kinds already had its own `Engine`, channels and services. They
now run at the same time, each on a thread and current-thread runtime of its own, as each
had its own runtime when they ran one after another, and share one window, as
`network_rejections_apply_backpressure_when_event_consumer_stops` in the same file shares
one for its two engines. The 2048 commands per kind and the assertion are unchanged; a
kind whose producer sends all of them within the window is still reported by name.

The test notices an unbounded kind only if that kind's producer sends its 2048 commands
within the window, so running the kinds together could narrow what it detects. A
temporary probe test (not committed) measured that time: it drained one kind's events, so
that its engine accepted every command as an unbounded engine would, while the other
eight ran blocked as in the test, and compared four designs: the kind alone on a
current-thread runtime (the former sequential test), all kinds on a multi-thread runtime
with four or nine workers, and a thread and current-thread runtime per kind. "Busy" means
that the probe and the rest of the engine integration executable's tests, run in a loop,
shared 4 or 2 CPUs (process affinity on Windows, `taskset` on Linux), roughly a CI
runner's load. One session with all four designs per platform, six rounds per kind,
design and scenario, median / slowest of the 54 producer times in ms
(`_build/qa/speed-pass/fix-margin-win4`, `fix-margin-linux3`):

| Producer of an unbounded kind | Windows idle | Windows, 4 busy CPUs | Windows, 2 busy CPUs | Linux idle | Linux, 4 busy CPUs | Linux, 2 busy CPUs |
| --- | --- | --- | --- | --- | --- | --- |
| Alone (former sequential test) | 13 / 25 | 18 / 74 | 19 / 124 | 19 / 78 | 16 / 182 | 19 / 72 |
| Multi-thread runtime, 4 workers | 16 / 63 | 24 / 272 | 31 / 203 | 69 / 158 | 27 / 194 | 21 / 87 |
| Multi-thread runtime, 9 workers | 16 / 41 | 23 / 194 | 31 / 110 | 71 / 137 | 33 / 214 | 21 / 94 |
| A thread and current-thread runtime per kind (this test) | 13 / 28 | 24 / 181 | 33 / 231 | 18 / 72 | 20 / 139 | 22 / 89 |

- On idle CPUs an unbounded kind sends its commands in 9 to 72 ms with a thread per kind,
  as alone. A multi-thread runtime hands every command between worker threads; on Linux
  (WSL 2) that took about four times as long even on idle CPUs, with nine workers as
  with four.
- On busy CPUs the nine kinds share the CPUs while their engines start. Over all seven
  probe sessions (`fix-margin-*`; 216 to 594 samples per design and platform), producers
  needed more than 200 ms in 1 sample of the former sequential test (205 ms, Windows),
  5 with four workers (up to 272 ms), 2 with nine workers (up to 238 ms) and 2 with a
  thread per kind (231 and 220 ms, Windows, 2 busy CPUs). Three of the worker samples
  come from the first Linux session, which ran just before another session stopped the
  VM.
- The window is therefore 400 ms, 1.7 times the slowest producer of this design and 1.5
  times that of any design. A bounded kind never sends all its commands, so a longer
  window cannot fail it: the test passed 20 of 20 runs on 2 busy CPUs on each platform
  (`fix-stress-win`, `fix-stress-linux`; 0.41 to 0.64 s each).

### Linux-only worker platform files

[Test execution](../developer/TEST_EXECUTION.md) keeps one binary per file in this crate
because its tests change or inspect process-wide containment. The three Linux-only files
do so only in a re-executed child: the spool owner test marks its child non-dumpable and
the worker policy test lowers its child's CPU limit; both children now select their test
by its module-qualified name with `--exact` and still run that one test alone. Linux
workers start through `std::process::Command`, whose pipes are close-on-exec, so tests
that start workers in the same process inherit nothing from them. The Windows-only files
stay separate executables: on Linux an empty executable starts in 1 to 2 ms (0.7 to
2.2 ms per run here), and `worker_policy_windows` starts its child with plain
`std::process::Command` while Windows worker creation briefly holds inheritable pipe ends
(`lock_process_creation`), so it must not share a process with `worker_policy`'s workers.

## Method

1. The `2e6ee4ff` executables were kept for the comparison. The profile change gives every
   executable that links `regex-automata` a new file name, so those old ones stayed in
   place; 15 executables were also copied aside before the first build of the branch,
   which overwrote three of them (`spool`, `store` and `worker_policy` of
   `midnight-worker-platform`), whose copies were used. The branch was built in the same
   worktree and WSL mirror with the settings of the `2e6ee4ff` timing:
   `cargo test --workspace --locked --offline -j 4 --no-run
   --message-format=json-render-diagnostics`, debug information off, on Windows
   `CARGO_INCREMENTAL=0`.
2. Per executable, both builds' executables first started twice with `--list` and once
   with `--list --ignored` (the listings are compared under [Coverage](#coverage); this
   also pays a new file's first start outside the timed runs). Then three rounds ran the
   whole executable once per build, alternating which build went first, from the package
   directory, with libtest JSON output and `--report-time`, a private temporary directory,
   and on Windows a private, protected `HOST_TEST_INSTALL_ROOT`. Windows and Linux ran
   one after the other with no build running; the Linux run held the WSL build slot.
3. Windows first starts: for every executable of both builds, alternating, a copy with 16
   random bytes appended (a new file and content hash, as after a relink) listed its tests
   twice.
4. After the review, a second session: the six removed executables' new-hash copies first
   started in four rounds (`fix-win-measure/first-starts.out`); the engine integration
   executable with the final rejected-work test and `midnight-platform-common`'s library
   ran five alternating rounds against the `2e6ee4ff` executables (`fix-win-measure`,
   `fix-linux-measure`), the rejected-work test alone five more. On Windows the final
   build was the same workspace build; on Linux it was `cargo test -p` of the five
   changed packages in a separate WSL mirror (`spfix`), whose dependencies lack some
   workspace features (for example `regex-automata` without its DFA features).

The scripts were kept in a private scratch folder outside the repository
(`speed-pass/`: `snap.py`, `inplace.py`, `ab.py`, `probe2.py`, the drivers
`win-ab.ps1` and `linux-ab.sh`, and report scripts;
`speed-fix/` for the second session and the probe); the results are in the worktree's
`_build/qa/speed-pass/` (`win-final`, `linux-final`, `win-probe`, `regex-probe`, the
second session's `fix-*`, and the earlier `win-ab`, `linux-ab`, `win-lists` and
`linux-lists`, see [Not changed](#not-changed)).

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on D:,
Microsoft Defender real-time protection on; WSL 2 Ubuntu 24.04 (kernel
6.6.87.2-microsoft-standard-WSL2, 20 vCPUs, 15.4 GiB) on the same laptop, ext4. Rust
1.99.1, debug builds. A shared machine: other applications and other sessions' builds
ran throughout, the UI test executables keep all 20 CPUs busy, and the laptop slows down
under sustained load.

## Results

Wall times of the executables this pass changed, as median of the runs per build (in
brackets: the runs); every executable started moments before. The engine integration
executable is from the second session, with the final rejected-work test; the other rows
are from the first session:

| Executable | Windows before s | Windows after s | Linux before s | Linux after s |
| --- | ---: | ---: | ---: | ---: |
| `midnight-platform-common::test:integration` | 5.21 (5.48, 5.15, 5.21) | 0.69 (0.69, 0.73, 0.62) | 5.19 (5.22, 5.14, 5.19) | 0.48 (0.48, 0.48, 0.51) |
| `midnight-platform-common::lib:midnight_platform_common` | 0.59 (0.56, 0.79, 0.59) | 0.39 (0.39, 0.41, 0.39) | 0.50 (0.49, 0.51, 0.50) | 0.29 (0.29, 0.29, 0.30) |
| `midnight-platform::test:integration` | 0.32 (0.41, 0.32, 0.26) | 0.07 (0.10, 0.07, 0.06) | 0.00 | 0.00 |
| `midnight-engine::test:integration` | 6.59 (6.68, 5.54, 5.98, 6.59, 6.65) | 6.37 (6.37, 6.06, 6.19, 6.41, 7.53), noise | 1.95 (1.92, 1.93, 1.95, 1.95, 1.95) | 1.67 (1.67, 1.70, 1.61, 1.60, 1.67) |
| `midnight-worker-platform::test:spool`, `store`, `worker_policy` | 0.04, 0.08, 0.30 | 0.04, 0.07, 0.28, noise | 0.01, 0.01, 0.00 | 0.01, 0.01, 0.00 |
| The six removed executables, together | 0.06 | — | 0.03 | — |

On Windows the engine integration executable is bounded by the corpus test
(`operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies`,
5 to 6 s), so its difference is noise: the runs overlap, the first session gave 6.28 →
5.48 s (overlapping as well) and the review 6.77 → 7.15 s. On Linux it is now bounded by
`selected_folder_sizes` (1.3 s). There the second session's after build was a
`cargo test -p` build (see [Method](#method)); the first session, with workspace builds
on both sides and an earlier version of the rejected-work test, gave 1.97 → 1.62 s. Its
summed test time fell from 32.0 to 26.8 s on Windows and from 18.2 to 16.3 s on Linux in
the first session.

`midnight-platform-common`'s library is about 0.2 s faster on both platforms, not noise:
its slowest test, `settings::tests::versioned_json_roundtrips_every_theme_and_density`,
which bounds the executable, decodes preferences and so compiles their panel filters and
comparator rules with `regex`. It took 0.32 to 0.35 s instead of 0.49 to 0.71 s on
Windows and 0.26 to 0.27 s instead of 0.44 to 0.47 s on Linux in every run, and five
more alternating Windows runs in the second session gave 0.35 to 0.38 s against 0.55 to
0.91 s (`fix-win-measure`); the review's two runs, 0.50 and 0.52 → 0.89 and 0.35 s,
include one slow outlier.

All test executables, run by run in the first session (sum of wall times over the 63
executables that both builds have; the six removed ones add 0.06 s on Windows and
0.03 s on Linux before):

| | Run 1 s | Run 2 s | Run 3 s | Passed / failed / ignored |
| --- | ---: | ---: | ---: | --- |
| Windows `2e6ee4ff` (69 executables) | 48.93 | 49.30 | 49.56 | 6,780 / 0 / 143 |
| Windows this branch (63) | 42.09 | 42.38 | 42.45 | 6,781 / 0 / 143 |
| Linux `2e6ee4ff` (69) | 35.86 | 37.41 | 37.15 | 6,689 / 3 / 133 |
| Linux this branch (63) | 32.40 | 32.66 | 32.29 | 6,690 / 3 / 133 |

The totals differ by 6.9 to 7.2 s on Windows and 3.5 to 4.9 s on Linux, but they include
executables that this pass did not change: on Windows the engine library alone was 1.4 s
faster in the median (8.32 → 6.94 s; its runs, 5.77 to 8.69 s, overlap) and the engine
integration executable 0.8 s; on Linux `midnight-ui`'s library and `me` were slower.
The `midnight-ui` integration executable and the engine library varied by up to 2.9 s
between runs of the same build. Counting only the executables that this pass changed, a
run of executables that started moments before is about 5.0 s (10 %) shorter on Windows
(platform-common integration 4.5 s, its library 0.2 s, platform integration 0.25 s, the
six removed executables 0.06 s) and about 5.2 s (14 %) on Linux (4.7 s, 0.2 s, the engine
integration executable 0.3 s and 0.03 s). The three Linux failures are the runner's, in
both builds: `midnight-plugin-host`'s native file tests require their temporary
directory below a `_build` directory, which the runner's private one was not; with one
there (`_build/qa/speed-pass/linux-recheck`) the library passed 73 / 0 / 6 in both builds.

Windows first starts after a relink: only the six removed executables save one. In the
second session their new-hash copies first started in 0.7 to 2.2 s each, apart from one
of 10.2 s; the start overhead of all six was 5.9, 6.4, 7.8 and 16.6 s in the four
rounds. The first session's probe gave 7.0 s and the review's 9.3 s. That probe also
summed one first start of every executable of each build: 222.9 s of start overhead for
the 69 executables of `2e6ee4ff` (688.6 MB) and 208.9 s for the 63 of this branch
(678.1 MB), 14.0 s less. Only 7.0 s of that are the six removed executables; the other
7.0 s fell on the 63 executables that both builds have and are single-sample noise of
Defender's scanning. The unchanged 1.1 MB `midnight-worker-platform` library test
executable took 9.45 s in the `2e6ee4ff` probe and 1.20 s in this branch's,
`midnight-filesystem-host`'s library test 4.44 and 1.86 s, and without those two the
other 61 were 3.8 s slower after. One sum of first starts cannot resolve a difference of
a few seconds. Together with the warm saving, the first run after a full relink is about
11 to 14 s (4 to 5 %) shorter on Windows: 6 to 9 s of first starts and about 5 s of test
time.

## Coverage

On both platforms the listings of all executables of both builds were compared (a test
listed by `--list` and, for ignored tests, by `--list --ignored`); the final build was
listed again in the second session:

- Windows: 6,923 tests (143 ignored) in 69 executables before, 6,926 (143 ignored) in 63
  after (`fix-win-lists`). 60 executables list exactly the same tests; the platform
  integration executable adds `library_has_no_unit_tests`, the `midnight-elevation` and
  `midnight-worker-platform` library tests add `tests::listing_binary_has_no_unit_tests`
  and `tests::probe_binary_has_no_unit_tests`; the six removed executables listed no test.
- Linux: 6,825 tests (133 ignored) before, 6,828 (133 ignored) after: the first session's
  6,826 and the two binary guards. The first session compared every executable; the
  second listed the 15 executables of the five changed packages (2,505 tests, 26 ignored,
  in 21 executables before; 2,508 after; `fix-linux-lists`). The three guards are added
  as on Windows; `spool`, `store` and `worker_policy` list the six tests of the removed
  `spool_linux` (3), `store_linux` (2) and `worker_policy_linux` (1) under `linux::`; the
  three other removed executables listed no test; every other list is unchanged.

Every one of these tests passed in all three runs of both builds of the first session,
apart from the three Linux runner failures above, and in the validation runs. The
rejected-work and comparator budget tests keep their assertions; the worker policy and
spool owner tests' children ran their test alone.

## Not changed

- **The 162 duplicated library unit tests** in `midnight-engine`'s integration executable.
  `checksums_backend.rs` and `checksum_manifest_verification.rs` include `src/checksum.rs`
  and `src/retirement.rs`, and the two user-command preparation files
  `src/file_targets.rs`, with `#[path]` for their `#[cfg(test)]` internals; `cfg(test)`
  holds there too, so 46 checksum and 32 retirement tests run again per checksum file and
  3 file-target tests per preparation file. A first build of this branch declared those
  test modules inside a macro that the including files defined as empty: the integration
  executable then listed exactly these 162 tests fewer, each still listed by the library
  (both platforms, `_build/qa/speed-pass/win-lists` and `linux-lists`), and its summed
  test time fell by a further 2 to 3.5 s; its wall time did not fall on Windows (5.52
  against 5.41 s, the corpus test bounds it) and fell to 1.48 instead of 1.62 s on Linux
  (`win-ab`, `linux-ab`). It was withdrawn: `cargo fmt` follows no `mod` declared inside
  a macro call, so 13 test files would have dropped out of `cargo fmt --check`, and no
  `cfg` tells the library's test build from an integration test that includes the same
  source.
- **Fixed waits set by engine timing.**
  `engine_comparator_policy_tests::settings_return_after_shutdown_grace_and_runtime_exit_retires_on_cleanup_worker`
  (2.0 s) waits out the engine's fixed two-second settings grace at shutdown (`lib.rs`),
  and the throughput timer test waits for real heartbeat ticks (1.0 s). Shortening them
  needs an injectable interval in engine code (Backend), and the engine library
  executable is bounded by its total work (89 to 114 s of summed test time on Windows,
  49 to 60 s on Linux, on 20 threads), so the gain would be small.
- **`native_operation_layout`** (4.3 s on Windows): it lays out every theme on purpose
  ("every theme is laid out and checked"); replacing the other themes' layouts with a
  geometry check is the UI owner's trade-off.
- **Large fixtures**: the Windows corpus test (about 5 s, 271 files), the metadata
  traversal tests (1,000 entries), the enumeration-page move test and the 100,000
  sibling roots test check exactly these sizes; shrinking them is their owner's call.
- **`midnight-platform-linux`'s watch test** (0.55 s of fixed windows on Linux): its 80 ms
  quiet windows attribute each notification to its mutation through an API that reports
  only that something changed.
- **The five executables that hold only opt-in ignored tests**: CI starts them by name
  with `--ignored`, and `test = false` would also drop them from `cargo clippy
  --all-targets` and from unfiltered `--no-run` builds.
- **`midnight-ui-input`'s `placement`** (24.7 MB, three tests, about 10 s first start on
  Windows): the layout rule keeps per-file binaries in crates with at most three
  integration files, and its tests link a Slint software-rendered window; folding them
  into the library's 1.4 MB test executable would make that one about as large, so only
  the fixed part of one first start (about 1.3 s) would be saved.
- **`midnight-platform-linux`'s library on Windows and `midnight-platform-windows`' 13
  executables on Linux**: Cargo cannot leave a target out on one platform; on Linux an
  empty executable starts in 1 to 2 ms.
- **`midnight-ui`'s integration executable** (13 to 17 s here, bounded by its total work
  on 20 threads): its UI stack is already optimized in debug builds, and no profiler was
  available (no `perf` in the WSL image, no elevation for Windows Performance Recorder).
- **The comparator font scan** that the compare-ui tests wait for when it is cold: product
  code.

## Validation

On this branch's final sources (the second session):

- Windows: `cargo test --workspace --locked --offline -j 4 --no-fail-fast` with a
  private, protected `HOST_TEST_INSTALL_ROOT`: 86 test result lines, 6,783 passed,
  0 failed, 143 ignored (`_build/qa/fix-win-test.log`).
- Linux, in the `spfix` WSL mirror: `cargo test -p midnight-engine -p midnight-elevation
  -p midnight-worker-platform -p midnight-platform -p midnight-platform-common --locked
  --no-fail-fast -j 4`: 20 test result lines, 2,482 passed, 0 failed, 26 ignored
  (`_build/qa/fix-linux-test2.out`). The other packages are unchanged except that they
  are rebuilt against `midnight-worker-platform`, whose only change is a unit test; the
  first session's workspace run passed 6,693 / 0 / 133 (`_build/qa/speed-linux-test.log`).
  A first run in that mirror failed the three source-scanning tests of the platform
  crates (`every_test_file_is_a_module` twice, `library_has_no_unit_tests`): its target,
  copied from another mirror to reuse the dependencies, still held their executables,
  whose `CARGO_MANIFEST_DIR` pointed into that mirror, since deleted. With the workspace
  packages' artifacts removed, every test executable was rebuilt there and the run above
  passed.
- Windows: `cargo clippy --workspace --all-targets --locked --offline -j 4 -- -D warnings`:
  no warning (`_build/qa/fix-win-clippy.log`; a first attempt stopped once on "only
  metadata stub found for `rlib` dependency `test`" while checking `midnight-ui`'s
  integration tests, and the rerun passed). Linux: `cargo clippy -p
  midnight-worker-platform -p midnight-engine -p midnight-elevation -p midnight-platform
  --all-targets --locked -j 4 -- -D warnings`: no warning (`_build/qa/fix-linux-clippy.log`).
- `cargo fmt --all -- --check` and `git diff --check`: clean.

## Limitations

- One session per platform with three alternating runs for the totals, on a shared laptop
  that slows down under sustained load. The totals of the same build varied by up to
  0.6 s on Windows and 1.6 s on Linux between runs, single large executables by up to
  2.9 s. In an earlier session, one Windows run of `midnight-ui`'s integration executable
  spent 15 s idle before its tests started. The totals were measured with an earlier
  version of the rejected-work test (four workers, one 200 ms window); only the engine
  integration executable and the test were timed again with the final one.
- Windows first starts were measured on copies with a new content hash, not after real
  relinks, and Defender's cost varies with its state: single first starts of the same
  executable ranged from 1 to 10 s. Whether CI runners pay a first-start cost at all is
  unknown, and CI was not measured.
- The detection-margin probe drains the unbounded kind's events instead of reproducing a
  real regression, and its "busy CPU" scenarios only approximate a CI runner. The Linux
  VM was shut down by another session during one probe scenario, which was repeated.
- Build and link time were not measured, apart from `regex-automata`'s compile time; the
  six removed executables also save six links per build.
- In the earlier Windows session, `midnight-platform-windows`' library test
  `transfer::move_source::tests::whole_root_capture_retains_every_child_and_survives_all_pin_drops`
  failed once in six runs, in a `2e6ee4ff` executable: `copy file` of a just-written file
  below `_build/tmp` returned "Access is denied. (os error 5)". It passed in the other
  runs, in all six of the final session and in both validation runs; not attributed.
- The branch is based on `2e6ee4ff`. Main has since moved to `e7e0f390` (the Slint
  compile time, native icon and text fitting merge `a2f3f68b` and two UI commits), which
  was neither built nor timed here.
