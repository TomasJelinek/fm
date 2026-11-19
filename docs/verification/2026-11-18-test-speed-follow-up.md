# Test speed follow-up — 2026-11-18

## Scope

Follow-up to [test execution time](2026-11-18-test-execution-time.md), on main
`d3817ea5`: why the twelve test executables that pass no test on Windows cost about
a second each, why some executables are slow for their test count, and the two load
timeouts of backlog item TEST-03. Windows only; build and link time are not included.

## Method

1. In this worktree's `_build`, the test executables of the affected packages were
   built with `cargo test --locked --offline -j 6 --no-run --message-format=json-render-diagnostics`
   and `-p midnight-worker-platform -p midnight-platform -p midnight-platform-linux
   -p midnight-elevation -p midnight-image-viewer-host -p midnight-native-filesystem-host
   -p midnight-filesystem-host -p midnight-compare-ui -p midnight-ui-input -p midnight-viewer-ui
   -p midnight-engine` (36 executables at `d3817ea5`, 33 after the change), and those of
   `midnight-ui` separately with `-p midnight-ui -j 4`. Debug profile, debug info off
   (`CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`).
2. The runner of the earlier record ran each executable alone, one after another, from
   its package directory with libtest's default thread count, JSON output and
   `--report-time`, three times in a row right after the build. Run 1 is therefore the
   first start of every newly linked executable. `HOST_TEST_INSTALL_ROOT` pointed at a
   private, owner-only directory. A libtest filter limited some runs to one module.
3. A start-up probe ran `EXE --list` twice for every executable (process start, test
   listing and exit; no test code runs), then copied each executable to a new file and
   listed the copy twice.
4. Where a test changed, both builds were copied aside, started twice, and then run
   alternately, so that both met the same machine load.
5. `EXE --list` and `EXE --list --ignored` of every executable were saved before and
   after the change and compared.

The runner and probe are orchestrator scripts, not repository tools; the results are
in this follow-up's worktree under `_build/qa/speed/` (`before-*`, `after-*`,
`probe-before`, `ab`, `list-before`, `list-after`).

## Environment

Windows 11 Enterprise 10.0.26200, Rust 1.99.1, NTFS on D:, 20 logical CPUs, 32 GB.
Microsoft Defender real-time protection was on with cloud protection at the High block
level, a 50 s extended cloud check and block at first sight enabled
(`Get-MpComputerStatus`, `Get-MpPreference`); its signatures (1.463.357.0) were last
updated at 07:26, before every run. The machine was shared: another workflow compiled
throughout, and CPU load sampled during the later runs was 71 to 100 %.

## First start of a new executable

Each executable costs about 1 s or more the first time it starts after it is linked,
and almost nothing afterwards. The cost follows the file, not the code: a
byte-for-byte copy pays again on its first start, and its second start is fast.
Static initialization, an embedded manifest or anything else in the program would cost
at every start, but a later `--list` of the same file took 0.01 to 0.15 s. Large
executables pay most: fresh copies of the 25 to 42 MB test executables took 2.8 to
8.2 s to list once. With Defender's on-access scanning and cloud checks as configured
above, this is consistent with scanning on first execution. The attribution is by
elimination: without administrator rights, Defender's exclusions and logs could not be
read, and its CPU use was not conclusive because it was busy with the other workflow's
builds throughout.

The twelve executables that pass no test on Windows, at `d3817ea5`:

| Executable | MB | Run 1 s | Run 2 s | Run 3 s | Copy, 1st list s | Copy, 2nd list s | Outcome |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `midnight-elevation::bin:midnight-elevated-listing` | 0.7 | 1.20 | 0.07 | 0.05 | 0.25 | 0.02 | removed |
| `midnight-worker-platform::bin:midnight-worker-probe` | 0.7 | 1.37 | 0.06 | 0.05 | 0.25 | 0.02 | removed |
| `midnight-platform::lib:midnight_platform` | 0.7 | 1.44 | 0.06 | 0.03 | 0.24 | 0.02 | removed |
| `midnight-platform-linux::lib:midnight_platform_linux` | 0.7 | 1.27 | 0.05 | 0.03 | 0.17 | 0.01 | kept: Linux unit tests |
| `midnight-worker-platform::test:spool_linux` | 0.7 | 1.30 | 0.06 | 0.03 | 0.10 | 0.01 | kept: isolation rule |
| `midnight-worker-platform::test:store_linux` | 0.7 | 3.02 | 0.11 | 0.03 | 0.11 | 0.01 | kept: isolation rule |
| `midnight-worker-platform::test:worker_policy_linux` | 0.7 | 1.83 | 0.11 | 0.06 | 0.14 | 0.01 | kept: isolation rule |
| `midnight-filesystem-host::test:discovery_kinds` | 2.1 | 1.65 | 0.05 | 0.05 | 0.45 | 0.02 | kept: opt-in, named in CI |
| `midnight-filesystem-host::test:streaming_worker` | 2.2 | 1.68 | 0.09 | 0.04 | 0.49 | 0.01 | kept: opt-in, named in CI |
| `midnight-image-viewer-host::test:native` | 1.6 | 1.68 | 0.10 | 0.03 | 0.41 | 0.02 | kept: opt-in, named in CI |
| `midnight-image-viewer-host::test:pdf_native` | 1.6 | 1.49 | 0.08 | 0.03 | 0.44 | 0.01 | kept: opt-in, named in CI |
| `midnight-native-filesystem-host::test:native` | 2.5 | 1.90 | 0.05 | 0.03 | 0.58 | 0.01 | kept: opt-in, named in CI |
| **Total** | | **19.83** | **0.89** | **0.46** | | | |

The executables that are slow for their test count, at `d3817ea5`:

| Executable | MB | Tests | Run 1 s | Run 2 s | Run 3 s | Copy, 1st list s | Slowest test in runs 2 and 3 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `midnight-compare-ui::test:integration` | 37.3 | 10 | 10.51 | 1.05 | 0.98 | 3.54 | 0.83 to 0.88 s, `fonts::` |
| `midnight-compare-ui::lib:midnight_compare_ui` | 36.0 | 6 | 7.61 | 1.11 | 0.75 | 4.32 | 0.61 to 0.84 s, `wake_tests::` |
| `midnight-viewer-ui::test:integration` | 35.4 | 20 | 13.46 | 0.26 | 0.22 | 8.17 | 0.12 s |
| `midnight-ui-input::test:placement` | 24.7 | 3 | 7.01 | 0.15 | 0.11 | 2.82 | 0.004 s |
| `midnight-platform::test:integration` | 1.4 | 18 | 2.07 | 0.63 | 0.53 | 0.39 | 0.44 to 0.54 s, the junction test |
| `midnight-worker-platform`, 13 executables | 0.7 to 1.1 each | 43 | 22.40 | 2.16 | 1.80 | 0.10 to 0.34 each | 0.34 to 0.39 s, `worker_policy` |

Once started, the UI executables take 0.1 to 1.1 s. The comparator's font test spent
0.65 s of its 0.71 s waiting for the comparison engine's scan of the installed
fixed-pitch fonts (measured with temporary timing output, not committed), and the
library's wake test waits for the same catalog; that scan is product code. In
`midnight-platform` the junction test spent nearly all its time starting Windows
PowerShell.

## Changes

| Change | Before | After |
| --- | --- | --- |
| `test = false` for the three targets that have no test on any platform: the `midnight-elevated-listing` and `midnight-worker-probe` binaries and the `midnight-platform` library, whose integration binary gains the guard `library_has_no_unit_tests` | three executables: 4.01 s in run 1, 0.19 and 0.13 s in runs 2 and 3 | no executable |
| The `midnight-platform` junction fixture runs `cmd /d /v:off /c mklink /J` instead of Windows PowerShell with `create-junction.ps1` | 10 alternating runs: wall median 0.704 s (0.618 to 0.883), junction test median 0.659 s | wall median 0.130 s (0.098 to 0.164), junction test median 0.104 s |
| TEST-03: `UI_WAIT` of the real-Engine UI harnesses 15 s → 60 s; every 3 s progress wait of the engine's `tests/navigation.rs` → `PROGRESS` = 30 s | see below | see below |

Every wait with a lengthened deadline returns as soon as its condition holds; the
deadline only bounds a hang. `navigation.rs` keeps its 100 ms check that a third
uncancellable read does not start. Passing runs did not change:

- `navigation::` (8 tests, filtered runs of `midnight-engine::test:integration`): wall
  0.46, 0.15 and 0.15 s before and 0.15 s three times after; each test took 0.010 to
  0.051 s (0.12 s for the test with the 100 ms check) before and after.
- `operations_native_ui::` (13 tests, `midnight-ui::test:integration`): three runs
  before took 0.81 to 0.87 s and three runs after 1.39 to 1.61 s, but the machine load
  had risen in between. Six alternating runs of both builds under 100 % load gave wall
  medians of 3.20 s (1.67 to 3.58) before and 2.57 s (1.89 to 3.34) after, a
  difference within the noise.

With a unit test added to `crates/midnight-platform/src/lib.rs`,
`library_has_no_unit_tests` failed and named the line; without it the test passed.
Cargo still builds the library harness when `cargo test -p midnight-platform` is given
a test-name filter, but not in unfiltered runs, `--no-run` builds of the package set or
CI's `--lib` gate, which excludes libraries with `test = false`.

The test lists of every executable are unchanged, apart from the three removed
executables, which listed no test, and the new guard test. In run 1 after the change
the `midnight-worker-platform` executables took 8.28 s in total, because only the
relinked ones paid their first start; runs 2 and 3 took 1.84 and 3.08 s under the
higher load.

## Not changed

- The five executables that hold only opt-in (ignored) tests: CI runs them by name
  with `--ignored`.
- `midnight-platform-linux`'s library harness is empty on Windows, but Cargo cannot
  disable a target on one platform only, and it holds the crate's Linux unit tests.
- `spool_linux`, `store_linux` and `worker_policy_linux` of `midnight-worker-platform`
  pass no test on Windows (three first starts, 1.3 to 3.0 s each here), but
  [test execution](../developer/TEST_EXECUTION.md) keeps one binary per file in this
  crate for isolation. For these three files the rule appears stricter than needed:
  `store_linux.rs` only creates private directories and links below its own
  temporary directory; `spool_linux.rs` and `worker_policy_linux.rs` change
  process-wide state (the dumpable attribute, the CPU rlimit) only in a re-executed
  child, so as `cfg(target_os = "linux")` modules of the `store`, `spool` and
  `worker_policy` binaries they would need module-qualified child names
  (`spool_linux::…`, `worker_policy_linux::…`). That is left to the reviewer.
- The large UI executables: their first-start cost comes with the size of the file,
  not with their tests. Merging a crate's library and integration executables would change the
  documented layout, and the comparator's `wake_tests` must stay the only test with an
  event-loop proxy in its binary.
- The comparator's font scan is product code. Its integration tests still poll with
  short real sleeps (`support::wait_for`, `ui_polish::draw`), but they run beside the
  font test and do not extend the binary's run.

## Validation

- `cargo test -p midnight-platform -p midnight-elevation -p midnight-worker-platform
  -p midnight-engine --locked --offline -j 6`: engine library 1629 passed / 16 ignored,
  engine integration 421 / 8, platform integration 19 / 0, elevation library 24 / 0,
  worker-platform library 20 / 1 and its eleven test executables 23 / 0 (the three
  Linux files pass none on Windows), doc tests 0; no failure.
- `cargo test -p midnight-ui --locked --offline -j 4`: library 1191 passed / 5 ignored,
  integration 747 / 1, doc tests 0; no failure.
- `cargo clippy -p midnight-platform -p midnight-engine -p midnight-ui -p midnight-elevation
  -p midnight-worker-platform --all-targets --locked --offline -j 4 -- -D warnings`: no
  warning.
- `cargo fmt -p <package> -- --check` for the five changed packages: clean.

## Limitations

- Windows only. The Linux tests were not run: the changed Linux-relevant code is
  platform-neutral or Windows-only, and on Linux the `midnight-platform-windows`
  library and its twelve test executables and the three `*_windows` files of
  `midnight-worker-platform` likewise pass no test.
- A shared machine under changing load, so differences of a few tenths of a second are
  within noise; the before/after comparisons of changed tests alternate both builds.
- The first-start cost is not paid only once per link: in run 1 after the change,
  executables that had not been relinked were slow again (for example 8.15 s for
  `midnight-viewer-ui::lib`, 7.54 s for `midnight-compare-ui::lib`), after the probe had
  started byte-identical copies of them. Run 1 totals are therefore not comparable
  before and after, and whether CI runners scan executables at all is unknown.
