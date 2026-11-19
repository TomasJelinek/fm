# Test speed pass — 2026-11-18

## Scope

Follow-up to [test execution time](2026-11-18-test-execution-time.md) on branch
`claude/test-speed-pass`, based on main `2e6ee4ff`: cut the test execution time that
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

## Changes

| Change | Executables | Evidence |
| --- | --- | --- |
| `test = false` for the `midnight-elevated-listing` and `midnight-worker-probe` binaries and the `midnight-platform` library, which have no test on any platform; `library_has_no_unit_tests` in the platform integration executable fails when a unit test is added (cherry-picked `d43c1aaf`) | 3 fewer on both platforms | they listed no test; first start of a relinked copy on Windows 1.19, 1.01 and 1.16 s |
| The platform junction fixture runs `cmd /d /v:off /c mklink /J` instead of Windows PowerShell (cherry-picked `e7c60455`) | `midnight-platform::test:integration` | junction test 0.30 → 0.06 s (Windows) |
| `regex-automata` builds at `opt-level = 2` in the dev and test profiles | every executable that matches with `regex` | comparator budget test 5.05 → 0.51 s (Windows), 5.10 → 0.43 s (Linux) |
| The nine command kinds of the rejected-work test run as concurrent tasks sharing one 200 ms window | `midnight-engine::test:integration` | 1.93 → 0.23 s (Windows), 1.84 → 0.24 s (Linux) |
| `spool_linux`, `store_linux` and `worker_policy_linux` of `midnight-worker-platform` become the `linux` module of the `spool`, `store` and `worker_policy` executables (`tests/linux/*.rs`) | 3 fewer on both platforms | Windows built and started them without a test; first start of a relinked copy 1.14, 1.08 and 1.48 s |

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

Each of the nine command kinds already had its own `Engine`, channels and services; the
kinds now run as tasks on a four-thread runtime and share one 200 ms window, as
`network_rejections_apply_backpressure_when_event_consumer_stops` in the same file does
for its two engines. The window, the 2048 commands per kind and the assertion are
unchanged; a kind whose producer finishes within the window is still reported by name.

### Linux-only worker platform files

[Test execution](../developer/TEST_EXECUTION.md) keeps one binary per file in this crate
because its tests change or inspect process-wide containment. The three Linux-only files
do so only in a re-executed child: the spool owner test marks its child non-dumpable and
the worker policy test lowers its child's CPU limit; both children now select their test
by its module-qualified name with `--exact` and still run that one test alone. Linux
workers start through `std::process::Command`, whose pipes are close-on-exec, so tests
that start workers in the same process inherit nothing from them. The Windows-only files
stay separate executables: on Linux an empty executable costs about 0.01 s, and
`worker_policy_windows` starts its child with plain `std::process::Command` while Windows
worker creation briefly holds inheritable pipe ends (`lock_process_creation`), so it must
not share a process with `worker_policy`'s workers.

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

The scripts are in the session scratchpad (`speed-pass/`: `snap.py`, `inplace.py`,
`ab.py`, `probe2.py`, the drivers `win-ab.ps1` and `linux-ab.sh`, and report scripts);
the results are in the worktree's `_build/qa/speed-pass/` (`win-final`, `linux-final`,
`win-probe`, `regex-probe`, and the earlier `win-ab`, `linux-ab`, `win-lists` and
`linux-lists`, see [Not changed](#not-changed)).

## Environment

Windows 11 Enterprise 10.0.26200, i7-13800H (20 logical CPUs), 31.6 GB, NTFS on D:,
Microsoft Defender real-time protection on; WSL 2 Ubuntu 24.04 (kernel
6.6.87.2-microsoft-standard-WSL2, 20 vCPUs, 15.4 GiB) on the same laptop, ext4. Rust
1.99.1, debug builds. A shared machine: other applications ran throughout, the UI test
executables keep all 20 CPUs busy, and the laptop slows down under sustained load.

## Results

Wall times of the executables this pass changed, as median of the three runs per build
(in brackets: the three runs); every executable started moments before:

| Executable | Windows before s | Windows after s | Linux before s | Linux after s |
| --- | ---: | ---: | ---: | ---: |
| `midnight-platform-common::test:integration` | 5.21 (5.48, 5.15, 5.21) | 0.69 (0.69, 0.73, 0.62) | 5.19 (5.22, 5.14, 5.19) | 0.48 (0.48, 0.48, 0.51) |
| `midnight-platform-common::lib:midnight_platform_common` | 0.59 | 0.39 | 0.50 | 0.29 |
| `midnight-engine::test:integration` | 6.28 (6.28, 5.82, 6.39) | 5.48 (5.47, 5.48, 6.32) | 1.97 (1.97, 1.96, 1.98) | 1.62 (1.62, 1.65, 1.59) |
| `midnight-platform::test:integration` | 0.32 | 0.07 | 0.00 | 0.00 |
| `midnight-worker-platform::test:spool`, `store`, `worker_policy` | 0.04, 0.08, 0.30 | 0.04, 0.07, 0.28 | 0.01, 0.01, 0.00 | 0.01, 0.01, 0.00 |
| The six removed executables, together | 0.06 | — | 0.03 | — |

On Windows the engine integration executable is bounded by the corpus test
(`operations_real_windows::real_directory_corpus_matches_independent_sha256_for_both_policies`,
about 5 s), on Linux now by `selected_folder_sizes` (1.3 s); its summed test time fell
from 32.0 to 26.8 s on Windows and from 18.2 to 16.3 s on Linux. The other regex users
gained a little (`midnight-platform-common`'s library 0.2 s on both platforms).

All test executables, run by run (sum of wall times over the 63 executables that both
builds have; the six removed ones add 0.06 s on Windows and 0.03 s on Linux before):

| | Run 1 s | Run 2 s | Run 3 s | Passed / failed / ignored |
| --- | ---: | ---: | ---: | --- |
| Windows `2e6ee4ff` (69 executables) | 48.93 | 49.30 | 49.56 | 6,780 / 0 / 143 |
| Windows this branch (63) | 42.09 | 42.38 | 42.45 | 6,781 / 0 / 143 |
| Linux `2e6ee4ff` (69) | 35.86 | 37.41 | 37.15 | 6,689 / 3 / 133 |
| Linux this branch (63) | 32.40 | 32.66 | 32.29 | 6,690 / 3 / 133 |

So a test run of executables that started moments before is 6.9 to 7.2 s (14 %) shorter
on Windows and 3.5 to 4.9 s (12 %) on Linux. The three Linux failures are the runner's,
in both builds: `midnight-plugin-host`'s native file tests require their temporary
directory below a `_build` directory, which the runner's private one was not; with one
there (`_build/qa/speed-pass/linux-recheck`) the library passed 73 / 0 / 6 in both builds.
The `midnight-ui` integration executable and the engine library varied by up to 2.9 s
between runs of the same build, so their differences here are noise.

Windows first starts after a relink, from the new-hash copies: the 69 executables of
`2e6ee4ff` (688.6 MB) took 224.4 s for their first `--list` and 1.4 s for the second, so
222.9 s of start overhead; the 63 of this branch (678.1 MB) took 210.3 s and 1.4 s, so
208.9 s, 14.0 s less, of which the six removed executables account for 7.1 s. The
probed engine integration executable was an earlier build of this branch (28.4 MB, with
the exclusion withdrawn below); the final one (30.4 MB) first started in 8.55 and 8.87 s
against 9.05 and 9.13 s for the `2e6ee4ff` one in a separate probe. Together, the first
run after a full relink is about 21 s (8 %) shorter on Windows.

## Coverage

On both platforms the listings of all executables of both builds were compared (a test
listed by `--list` and, for ignored tests, by `--list --ignored`):

- Windows: 6,923 tests (143 ignored) in 69 executables before, 6,924 (143 ignored) in 63
  after. 62 executables list exactly the same tests; the platform integration executable
  adds `library_has_no_unit_tests`; the six removed executables listed no test.
- Linux: 6,825 tests (133 ignored) before, 6,826 (133 ignored) after. 59 executables
  list the same tests; the platform integration executable adds the guard; `spool`,
  `store` and `worker_policy` list the six tests of the removed `spool_linux` (3),
  `store_linux` (2) and `worker_policy_linux` (1) under `linux::`; the three other removed
  executables listed no test.

Every one of these tests passed in all three runs of both builds, apart from the three
Linux runner failures above. The rejected-work and comparator budget tests keep their
assertions; the worker policy and spool owner tests' children ran their test alone.

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
  empty executable starts in about 0.01 s.
- **`midnight-ui`'s integration executable** (13 to 17 s here, bounded by its total work
  on 20 threads): its UI stack is already optimized in debug builds, and no profiler was
  available (no `perf` in the WSL image, no elevation for Windows Performance Recorder).
- **The comparator font scan** that the compare-ui tests wait for when it is cold: product
  code.

## Validation

On this branch's final sources, after the timing:

- Windows: `cargo test --workspace --locked --offline -j 4 --no-fail-fast` with a
  private `HOST_TEST_INSTALL_ROOT`: 86 test result lines, 6,781 passed, 0 failed, 143
  ignored (`_build/qa/speed-win-test.log`).
- Linux, through the WSL mirror: `cargo test --workspace --locked --no-fail-fast -j 4`:
  86 test result lines, 6,693 passed, 0 failed, 133 ignored
  (`_build/qa/speed-linux-test.log`).
- Windows: `cargo clippy --workspace --all-targets --locked --offline -j 4 -- -D warnings`:
  no warning (`_build/qa/speed-win-clippy.log`). Linux, for the Linux-only test modules:
  `cargo clippy -p midnight-worker-platform -p midnight-engine -p midnight-platform
  --all-targets --locked -j 4 -- -D warnings`: no warning.
- `cargo fmt --all -- --check` and `git diff --check`: clean.

## Limitations

- One session per platform with three alternating runs, on a shared laptop that slows
  down under sustained load. The totals of the same build varied by up to 0.6 s on
  Windows and 1.6 s on Linux between runs, single large executables by up to 2.9 s. In
  an earlier session, one Windows run of `midnight-ui`'s integration executable spent
  15 s idle before its tests started.
- Windows first starts were measured on copies with a new content hash, not after real
  relinks, and Defender's cost varies with its state; whether CI runners pay a first-start
  cost at all is unknown, and CI was not measured.
- Build and link time were not measured, apart from `regex-automata`'s compile time; the
  six removed executables also save six links per build.
- In the earlier Windows session, `midnight-platform-windows`' library test
  `transfer::move_source::tests::whole_root_capture_retains_every_child_and_survives_all_pin_drops`
  failed once in six runs, in a `2e6ee4ff` executable: `copy file` of a just-written file
  below `_build/tmp` returned "Access is denied. (os error 5)". It passed in the other
  runs, in all six of the final session and in the validation run; not attributed.
- The branch is based on `2e6ee4ff`. Main has since moved to `a2f3f68b` (the Slint
  compile time, native icon and text fitting merge), which was neither built nor timed
  here.
