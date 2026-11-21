# Test execution

## Full validation

`scripts/validate.ps1` (Windows) and `scripts/validate.sh` (Linux) are the
full-validation entry point: they run every test that CI
(`.github/workflows/ci.yml`) runs for the host platform, with CI's own test commands,
and build only the SDK bundles, packages and fixtures that those tests need.
`build.bat --validate` runs `validate.ps1` after its build steps and before
the private deployment; `./build.sh --validate` runs `validate.sh` after its last
build step (native backend only). Run them on their own once the toolchain and
Python 3.12+ are installed:

```text
pwsh -NoProfile -File scripts\validate.ps1 [debug|release] [--strict] [--offline] [--dry-run] [--only ID[,ID...]]
bash scripts/validate.sh [debug|release] [--strict] [--offline] [--dry-run] [--only ID[,ID...]]
```

`--dry-run` prints every step with its commands, the variables they use and why a
step would be skipped, and runs nothing. Both scripts list the same 40 steps in CI's
job order (a test keeps them in step); a step that belongs to the other platform is
listed as skipped:

| CI job | Steps |
| --- | --- |
| `public-sdk` | `sdk-api` (tests), `sdk-abi` (C/Rust layouts; MSVC on Windows), `sdk-package` |
| `public-worker-sdk` | `sdk-protocols` (tests of the worker, thumbnail, filesystem, image-viewer and web-viewer APIs), `sdk-web-package`, `sdk-worker-package` |
| `viewers`, `viewer-conformance` | `viewer-basic`, `viewer-source`, `viewer-markdown` (the test command of their `scripts/ci.*`, then their package build), `viewer-conformance` (tests, fixture build and the native C loader test) |
| `image-viewer`, `thumbnail-image` | `viewer-image`, `viewer-pdf`, `thumbnail-image` (the test commands of their `scripts/ci.*`; the two viewers then build their packages) |
| not in CI | `thumbnail-video` (needs an FFmpeg 9.0.2 prefix), `html-viewer` (`--no-default-features`, without native CEF) |
| `archive-filesystem`, `filesystem-libraries` | `archive` (tests, release package, fixture export), `filesystem-api`, `filesystem-native` and `filesystem-network` (tests, release package) |
| `scripts-and-lint` | `test-install`, `script-tests`, `decoder-native-tests`, `powershell-tests-7`, `powershell-tests-5` |
| `check` | `workspace-tests`, `trash-qualification`, `bad-descriptor`, the nine ignored qualification steps, `x11-dnd`, `x11-clipboard` |
| `release` | `host-build` and `smoke-test` for the selected profile |

CI runs the `public-sdk` and `public-worker-sdk` jobs on ubuntu-24.04 only; its
Windows plugin jobs download the bundles that those jobs package. Windows validation
runs these steps in addition: it packages the bundles its plugin steps need, and
`sdk-api`, `sdk-abi` (`api/scripts/check-abi.ps1`, which CI never runs) and
`sdk-protocols` are extra Windows coverage.

`workspace-tests` runs CI's gates (below) with
`scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast`: the same Cargo
selections and features, including doc tests and example builds, but it retires no
test executable, needs no `CI=true` or `_build` target directory, adds Cargo's
`--no-fail-fast` and runs every gate after a failure (exiting with the first failing
status). The later qualification steps reuse those executables through its `run` and
`artifact` modes, as CI does.

Each step runs in its own process (a PowerShell 7 process, or Windows PowerShell 5.1
when PowerShell 7 is not installed; a `bash` subshell on Linux), with `TEMP`, `TMP`
and `TMPDIR` (Windows, as CI sets them) or `TMPDIR` (Linux) under `_build/tmp`,
`PYTHONUTF8=1` and CI's registry name variable. `rustc` must be on `PATH` (the
user-command probe tests compile with it); a checkout's `.tools/cargo` is used when
Cargo is not on `PATH`, as by the build helpers. The independent modules are built
below `_build/validate/` (the image viewer, PDF viewer and image thumbnailer share
`raster-ci`, so the pinned dav1d build is prepared once); the main workspace uses the
normal target directory. A few outputs lie outside `_build/validate/`: the plugin SDK
packager writes its bundle to `_build/plugin-sdk-<timestamp>` (removed when
validation ends), the web SDK packager compiles in `_build/html-viewer/api-target`,
`thumbnail-video` prepares FFmpeg in the build helpers' `_build/thumbnailers/ffmpeg`,
temporary files go to `_build/tmp`, and the Windows private test installation
(below) is created in `%USERPROFILE%`.
`test-install` mirrors CI's `private-test-installation` and `plugin-packages` actions
with the packages this run built: on Windows a new `%USERPROFILE%\MidnightViewerTests-<guid>` whose DACL
grants only the user, SYSTEM and Administrators; on Linux a mode-700
`_build/tmp/viewer-tests-*` directory. It sets `HOST_TEST_INSTALL_ROOT` and the other
installation variables of that action for every later step, and is removed when
validation ends, as are the SDK bundles the run packaged. Tests use the debug test
profile; the selected profile applies only to `host-build` and `smoke-test`.

A step whose prerequisite is missing is skipped with the reason: PowerShell 7 and
the x64 MSVC tools for the AV1 decoder bootstrap (`viewer-image`,
`thumbnail-image`), MSVC for the ABI tests on Windows, a Python with classic zlib
for the web SDK (Python 3.14+ on Windows and Fedora's Python use zlib-ng), the
pinned dav1d downloads of an earlier online run when `--offline`, the HTML viewer's
locked crates (its lockfile includes the optional CEF bindings) in Cargo's cache when
`--offline`, an FFmpeg prefix,
the GIO runtime for the Linux network plugin, and `xvfb-run` with `xauth`. A step
whose required step did not pass is skipped as well. `--strict` turns a skip for a
missing prerequisite into a failure; other-platform steps and SDKs given with
`--plugin-sdk`, `--worker-sdk` or `--web-sdk` never count. Without `--strict` such a
run still passes, but when a step that CI runs was skipped this way (or because a
step it needs did not pass), the summary names it before the result:
`WARNING: N CI step(s) SKIPPED for a missing prerequisite or needed step: IDS (use
--strict to fail)`; the two steps that CI does not run are not listed. `--only` runs
the named steps (at least one, comma-separated) and the steps they need, for example
`--only workspace-tests` reruns the workspace tests with their private installation.
A flag given a value, such as `--strict=no`, is rejected with exit status 2.

Validation continues after a failure. Each step's output is shown and saved as
`_build/validate/logs/<run>/NN-<step>.log`, next to the generated step script
(Windows) and `summary.txt`, the closing table of every step with its result,
duration and the passed, failed and ignored counts of its libtest and unittest
summaries. The exit status is 0 when no step failed, 1 when a step failed (or a
`--strict` skip), 2 for invalid arguments and 127 when cargo, rustc or Python 3.12+
is missing.

Differences from CI, all deliberate: validation runs tests only, so CI's
formatting, Clippy and source size checks are not steps, the plugins' `scripts/ci.*`
(which also run those checks) are not called but their test commands are (a test
compares the arguments of every `cargo test` run with each plugin's `ci.ps1` and
`ci.sh`), and the image thumbnailer's optimized build, which no test uses, is not
run: the build helpers compile the thumbnailer optimized only for a release build
(`build.bat release`, `./build.sh release`), so `--validate` with a debug build never
compiles it optimized; the steps run one after another on one machine instead of in
parallel jobs; test executables are kept; the developer's Cargo profile settings are
kept, while CI's `check` and `scripts-and-lint` jobs set `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0` and `CARGO_BUILD_JOBS=4` (set them
yourself to reproduce a CI build exactly; see [CI jobs and build time](#ci-jobs-and-build-time));
the filesystem API tests run once instead of in both filesystem jobs; and the two
modules above that CI does not build yet are included when their inputs exist.
Validation does not run inside `build.sh`'s Docker backend: the build image has no
Xvfb, mounts the source read-only and has not been qualified for the tests, so
`build.sh --validate` refuses Docker.

## CI jobs and build time

CI's `check` and `release` jobs start with the run instead of after the plugin jobs.
The workspace gates use no plugin package, only the empty protected root
(`HOST_TEST_INSTALL_ROOT`) that the `private-test-installation` action creates; every
test that reads an installed package is `#[ignore]`d and runs in a later qualification
step. So both jobs build and test first, and only then does the `plugin-packages`
action wait for this run's viewer and filesystem packages
(`scripts/ci-wait-for-artifacts.py`), download them and install them into that root.
The wait lists every artifact its pattern downloads select, so none starts with a
partial set; it fails at once when a plugin job leg of the same runner label has
finished without success, and after 60 minutes. Formatting, Clippy, the source size
check and the script, native helper and PowerShell tests run beside them in
`scripts-and-lint`.

`check` and `scripts-and-lint` build with four Cargo jobs (the hosted runners have four
CPUs and 16 GB of memory) and the default linkers. `check`, `scripts-and-lint` and
`release` restore the compiled third-party dependencies of the
root workspace from the Actions cache
([Swatinem/rust-cache](https://github.com/Swatinem/rust-cache), pinned by commit);
workspace crates are always compiled. Pushes to `main` save a new entry when its key
changes: the job, the installed rustc versions, any Cargo manifest, lockfile or Cargo
configuration, and the `CARGO_*` and `RUST*` environment. Other runs only restore; when
no key matches they take the newest entry of the same job, toolchains and environment.
The plugin jobs cache their downloads only. Measurements and the decisions behind
these settings: [CI time](../verification/2026-11-21-ci-time.md).

Locally, `.cargo/config.toml` keeps `jobs = 4`: several agents build on the shared
development machine at once, and one generated UI crate alone needs up to 7 GB. On a
machine of your own, set `CARGO_BUILD_JOBS` (or pass `-j`) to use more of it. On
Windows, `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld.exe` (the pinned
toolchain's LLVM linker) made the test executables of a CI-equivalent build about a
fifth faster to build on the development machine, and every test passed; on the hosted
runners it made no measurable difference, so CI does not use it. Set it only for test
builds: release builds keep the MSVC linker.

## Acceptance suite

The opt-in end-to-end acceptance suite is not part of validation, and CI does not run
it: CI's `scripts-and-lint` job lints it on Windows and Linux in its one workspace Clippy step,
which enables the suite's feature (`cargo clippy --workspace --all-targets --locked
--features midnight-acceptance/acceptance -- -D warnings`), so its platform-specific
code cannot stop compiling unnoticed and no dependency is checked twice.
`./build.sh --acceptance` (native or Docker backend) runs `scripts/acceptance.sh` after
its build steps, and on Windows `pwsh -File scripts\acceptance.ps1` builds and runs it
(`build.bat` has no acceptance step); the suite drives the real application with
synthetic input against a generated fixture tree and writes an HTML report below
`_build/acceptance/` on every platform, unless `MIDNIGHT_ACCEPTANCE_RUN_DIR` (or
`--run-dir` of `scripts\acceptance.ps1` and of the test executable) names another
absolute folder ([results](ACCEPTANCE.md#results)); the recorded Windows runs chose
`_build\acceptance-win`. Its only test target
requires the `acceptance` feature of `crates/midnight-acceptance`, so
`cargo test --workspace` and `scripts/ci-workspace-tests.py` never build or run it.
See [the acceptance suite](ACCEPTANCE.md) and its
[Windows results](acceptance/windows.md).

## Targeted test execution

Use `scripts/run-targeted-tests.py` for independent **headless Rust libtest** targets that have already been built. It does not change CI coverage, compile code, discover executables by glob, or delete build artifacts. Native dialogs, desktop input, renderer pacing and profiling are unsupported: run their dedicated tools serially, with compilers and other test processes idle. For example, `scripts/probe-keyboard-search.ps1` drives the release `native_keyboard_search` example with real Windows keyboard input on a desktop session (see its [verification record](../verification/2026-11-07-keyboard-native-input.md)). Idle event-loop wakeups are measured the same way: `MIDNIGHT_IDLE_WAKEUP_PROBE=<warmup_s>:<seconds>[:thumbnails]` makes `me` use in-memory default preferences, count winit wakeups by cause after the warmup and print one `idle_wakeups=` line to stderr before it quits. Only the preferences are isolated: the probe uses the plugins installed next to `me` and the real preview cache, so set `LOCALAPPDATA` (Windows) or `XDG_CACHE_HOME` (Linux) to a scratch directory for the run. Run one probe at a time with the pointer outside the window (see the [idle wakeup record](../verification/2026-11-18-ui-idle-wakeups.md)).

Build the exact affected package/target set once, using the normal repository environment and feature flags. Save Cargo metadata and successful `--no-run --message-format=json` output under `_build`. Do not start concurrent Cargo processes against a shared target directory. For example, from the repository root:

```text
cargo metadata --locked --no-deps --format-version 1 > _build/qa/targeted-metadata.json
cargo test --locked -p midnight-core --test integration --no-run --message-format=json > _build/qa/targeted-build.jsonl
py -3 -B scripts/run-targeted-tests.py --metadata _build/qa/targeted-metadata.json --receipt _build/qa/targeted-build.jsonl --test midnight-core test:integration shortcuts::
```

Most application crates link all files under `tests/` into one `integration`
binary: `tests/integration.rs` declares every other `tests/*.rs` file as a module
and Cargo `autotests` is disabled, so each test is named `<file>::<test>`. A guard
test, `every_test_file_is_a_module`, fails when a new file is not declared. One
binary per crate avoids one link step, one executable and one cold process start
per test file; select a former file with a module filter such as `shortcuts::`.
`midnight-worker-platform` and `midnight-platform-windows` keep one binary per
file because their tests change or inspect process-wide containment and terminal
state. In `midnight-worker-platform` a topic's Linux-only tests are the `linux`
module of that topic's binary (`tests/linux/spool.rs` in `spool`, likewise `store`
and `worker_policy`), so Windows does not build and start three binaries without a
test; they change process attributes (the dumpable flag, the CPU limit) only in a
re-executed child, and Linux workers start through `std::process::Command`. Its
Windows-only files stay separate binaries: on Linux an empty executable starts in
1 to 2 ms, and `worker_policy_windows` starts its child with plain
`std::process::Command` while Windows worker creation briefly makes pipe ends
inheritable (`lock_process_creation`), so it must not share a process with
`worker_policy`'s workers. Crates with at most three integration files also keep one
binary per file; CI selects some of those by name (`--test native`, `--test pdf_native`,
`--test worker`). A test that re-runs its own executable must select the child
by its module-qualified name (`<file>::<test>`, derived from `module_path!()`, run
with `--exact`). Targets without a test on any platform set `test = false`, so an
unfiltered `cargo test` neither builds nor starts an empty harness for them: the
`midnight-platform` library and the `midnight-elevated-listing` and
`midnight-worker-probe` binaries (the `midnight-worker-platform` integration tests
still receive the probe as `CARGO_BIN_EXE_midnight-worker-probe`). A unit test added
to one of them would never run, so a guard test fails on any test attribute in its
sources: `library_has_no_unit_tests` in the platform integration binary, and
`listing_binary_has_no_unit_tests` and `probe_binary_has_no_unit_tests` in the
`midnight-elevation` and `midnight-worker-platform` library tests, which read only
the binary's entry point and therefore also refuse a module file declared there.

Some `midnight-engine` integration test files include library sources with
`#[path]` to reach their `#[cfg(test)]` internals (checksums, retirement, file
targets). `cfg(test)` holds there too, so each include also compiles and runs the
unit tests that those sources declare: on Windows 193 of the executable's 485 tests are
library unit tests run again (they take 1.3–2.5 s alone; [CI time](../verification/2026-11-21-ci-time.md)).
No `cfg` tells the library's test build from such an include, and declaring the test
modules inside a macro would hide their files from `cargo fmt`, which follows no `mod`
inside a macro call; a new `#[path]` include of a source with unit tests adds them again.

On Windows with Microsoft Defender real-time protection, the first start of a newly
linked test executable costs about 1.3 s plus 0.17 s per MB (0.8 to 2.2 s below 5 MB,
17.6 s for the 108 MB `me` test executable), a later start 0.01 to 0.1 s. Most of the
cost follows a new file content (a byte-identical copy of a known executable costs
about a fifth), and it returns after some minutes without a start. A test run right
after a build pays it once per new executable, so fewer and smaller executables save
time even where they hold no test; Linux has no such cost. When comparing test times,
compare runs in the same state (each executable newly linked, or each one started
moments before) and say which. Single first starts vary widely (an unchanged 1.1 MB
executable took 1.2 s in one probe and 9.5 s in another), so compare first starts of the
executables that a change adds, removes or resizes over several samples each, not as the
difference of two sums of single starts ([test speed pass](../verification/2026-11-18-test-speed-pass.md)).

The `midnight-engine` library test executable re-runs itself for ten Copy seal
tests (`copy_seal_tests.rs`, `copy_finish_seal_tests.rs` and
`copy_nested_retirement_tests.rs`), whose deliberately panicking cleanups and
destructors must fail one child instead of aborting sibling tests. `child_case` in
`copy_seal_tests.rs` starts the same executable with `--exact <test> --nocapture`,
`MIDNIGHT_COPY_SEAL_TEST_CHILD=<test>` (the child then runs the test body) and
`RUST_BACKTRACE=0`, drains both pipes, and fails the parent test if the child has not
exited within 30 seconds; the parent kills and reaps its child when it fails. A
runner that terminates the parent process itself must also terminate those children.

When reproducing a failed CI job, match its workspace/package selection,
feature flags, profile, target, toolchain and lockfile. A targeted `-p` pass does
not establish workspace feature equivalence: shared dependencies can resolve
with different features. Retain the exact commands and resolved graph/features
with the receipt, state any differences and limit qualification to that scope.
Independent plugins still use their own workspace and normal configuration;
this does not require blanket workspace builds for independent plugin checks.

CI's `check` job runs the workspace in `scripts/ci-workspace-tests.py` gates
(`cargo test --workspace` with every member's active features, one gate per
target kind and bounded batches of integration names; a same-name group such as
the ten `integration` binaries stays one indivisible gate). Successful
integration executables are retired to bound disk use. Its later ignored-test
steps keep that selection instead of `cargo test -p`, which would rebuild shared
dependencies with one package's narrower features:

```text
python -B scripts/ci-workspace-tests.py run midnight-image-viewer-host test:native --ignored --test-threads=1
python -B scripts/ci-workspace-tests.py artifact midnight-explorer bin:me
```

`run PACKAGE lib|test:NAME|bin:NAME [LIBTEST ARGS...]` reuses (or relinks) that
package's test executable and runs it from the package directory; `artifact`
prints its path. Neither mode retires anything or needs `CI=true`. Locally,
`python -B scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast` runs
all gates without retiring executables ([full validation](#full-validation)).

Fixture readiness is not a maintenance-quiescence fence. Exact pre-ACK capture
fixtures must account for independent startup maintenance without weakening
production projection/occurrence guards. If an opt-in fixture defers hints, hold
only pure scalar `Changed` / `DirectoryChanged` through transfer terminal, then
replay the originals before reconciliation/external-write assertions; retain
native watchers and pre-ACK coverage. Tree watch checks (`tree_watch`,
`tree_clipboard`, `tree_move`) require the UI row plus native evidence for each
changed directory after the change. Evidence is either a fresh callback from a live
subscription that contains the directory and whose mask names it (an error or
overflow names every directory of its batch), or the registration of a batch that
contains the directory. A registration counts because a directory that joins a
batch is probed at once instead of waiting for a callback; a directory already in
the old batch stays covered by the old subscription until the swap ends, and the
new registration counts for it as well. The subscription may belong to either pane:
these fixtures root the other pane at `destination`, so its subscription can supply
that directory's evidence. A subscription that does not contain the directory and
the engine's periodic poll are no evidence, so the poll alone never satisfies a
wait; a batch registered after the change does, even when the old subscription
missed the change. Fences wait until a live subscription covers every changed
directory. A refresh admitted after a callback can still coalesce its hint. Test
before-admission coalescing separately from post-admission dirty-state survival,
retaining existing post-admission and default-timing coverage.

Engine watch tests assert hint pacing from worker-recorded send times
(`#[cfg(test)]` `watch::Handle::tree_fence`) and use probe-count barriers, not
wall-clock negative windows. `tree_clipboard` and the root transfers of `tree_move`
hold scalar hints from fixture construction through the transfer terminal.

To run two independent headless targets concurrently, select both in the build
and in the runner (no second Cargo process is needed):

```text
cargo test --locked -p midnight-core -p midnight-platform-common --test integration --no-run --message-format=json > _build/qa/targeted-build.jsonl
py -3 -B scripts/run-targeted-tests.py --metadata _build/qa/targeted-metadata.json --receipt _build/qa/targeted-build.jsonl --target midnight-core test:integration --target midnight-platform-common test:integration
```

The examples use the Windows Python launcher (`py -3 -B`); on Linux use `python3 -B`. `-B` suppresses bytecode files outside `_build`. Create `_build/qa` first if needed. Use UTF-8 redirection (PowerShell 7 supports this); do not mix receipts from failed builds or multiple builds of the same selected artifact. Supply the same toolchain, loader and installed-provider environment used to build and run the target. `rustc` must be available on PATH. The runner adds the native Rust runtime directory, artifact directories and receipt-declared generated native library paths under the target tree to the child's loader search path. Other required SDK/runtime paths remain the caller's responsibility. Cross-target execution is unsupported.

Selectors name a workspace package and exact `lib:`, `bin:`, `test:` or `example:` target from Cargo metadata. `--target PACKAGE KIND:TARGET` runs all nonignored tests in that target. `--test PACKAGE KIND:TARGET FILTER` selects a libtest substring; repeat it for additional filters, and use `--exact` for complete test names. Repeat `--receipt` for distinct successful build receipts. All requested package/target identities must identify one unique test executable beneath the repository's `_build` target directory.

Independent plugin workspaces use the same runner with an explicit `--workspace`.
Build and capture metadata from that plugin, pointing `CARGO_TARGET_DIR` to the
repository's `_build` tree; then, from the repository root, run for example:

```text
py -3 -B scripts/run-targeted-tests.py --workspace plugins/midnight-viewer-image --metadata _build/qa/image-metadata.json --receipt _build/qa/image-build.jsonl --target midnight-viewer-image test:svg
```

The selected workspace must be an ordinary directory within the repository and
must match the Cargo metadata exactly. Selected package manifests must stay within
that workspace. Artifacts and logs still belong to the repository's common
`_build`; choosing a plugin does not change this boundary. The runner queries
`rustc` from the selected workspace so its toolchain override is respected.

Each runner invocation owns its own budget. If three independent plugin runners
execute together on an N-logical-CPU machine, divide N between them (for example
`--thread-budget 6` each on 20 CPUs, 18 libtest workers in total); do not give each
the full default budget of N threads.
Coordinate the compilation first, especially when plugins share a target tree.

Multiple filters for the same artifact share one listing process and one test process. Libtest treats those filters as OR; overlapping tests execute once. Every requested filter must match at least one listed test, even if another selector requests the whole target. Ignored tests remain ignored. A missing match, an all-ignored selection, a nonzero child exit, or an absent/inconsistent libtest result is a failure. Matching uses the executable's own `--list` output, and summary counts are parsed at the start of complete libtest summary lines.

The default process limit is the smaller of four and the available CPU count. `--jobs 1` runs headless targets serially; `--jobs 2` through `--jobs 4` request bounded parallel execution. The limit is also clamped to CPU count, target count and `--thread-budget`. The total libtest thread budget defaults to available CPUs and is divided between child processes; an inherited smaller `RUST_TEST_THREADS` limit is preserved. Tests may create their own threads, so this is a libtest worker budget, not a system-wide thread guarantee. Prefer `--jobs 1` for targets that intentionally share a caller-provided fixture.

Each artifact runs from its package manifest directory, with private `TEMP`, `TMP` and `TMPDIR` directories. Caller-provided installed-provider variables and `MIDNIGHT_NATIVE_TEST_ROOT` remain intact. Do not parallelize targets that mutate the same external fixture simply because their temporary directories differ.

UI software-render snapshots are off by default, as in CI: the runner does not pass `MIDNIGHT_UI_RENDER_DIR` to its children, even when the caller sets it. `--render-snapshots` gives each artifact a private `render` directory beside its logs instead. `midnight-ui` tests then write every snapshot there as `<test>--<snapshot>.bmp`, where `<test>` is the libtest test name with `::` replaced by `.`, and geometry sweeps render every theme and combination instead of only their reference case, so such a run takes much longer. `MIDNIGHT_UI_RENDER_FILTER`, when set, still limits the written snapshots to names that contain it.

### Private plugin installation root

Plugin discovery trusts an installation only when it and every ancestor can be changed by no other account (`midnight_plugin_host::validate_plugin_path`). Tests that install plugin fixtures (`midnight-image-viewer-host` catalog and startup tests, the opt-in `midnight-plugin-host`, `midnight-filesystem-host` and engine filesystem-plugin tests) therefore need a private root. By default they create fixtures below the checkout's `_build` or the temporary directory, which works for a checkout under your user profile. A checkout whose ancestors other accounts may change, for example on a secondary Windows drive with the default ACL (Authenticated Users: Modify), or a temporary directory redirected there, cannot pass: the `midnight-image-viewer-host` fixtures then stop with an explanation. Point `HOST_TEST_INSTALL_ROOT` at a private directory instead, as CI's `private-test-installation` action does:

```powershell
$root = Join-Path $env:USERPROFILE ('MidnightViewerTests-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$acl = New-Object Security.AccessControl.DirectorySecurity
$acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)", 'Access')
Set-Acl -LiteralPath $root -AclObject $acl
$env:HOST_TEST_INSTALL_ROOT = $root
```

On Linux, use a `chmod 700` directory on a native filesystem whose ancestors are not group- or world-writable (sticky `/tmp`-style ancestors are accepted); drvfs mounts such as `/mnt/d` report permissive modes and are refused.

Archive tests that run a real worker (the opt-in `midnight-filesystem-host` worker and streaming tests, and engine archive tests with installed workers) stage decoded members on the real private spool volume (`%TEMP%` on Windows, usually `~/.cache` on Linux). Keep more than max(1 GiB, 2% of its capacity) plus the largest member a test stages (and at least 64 MiB) free there: a member that does not fit above the reserve is refused before staging, and content of undeclared length stops at the reserve, both with a disk-full error. Unit tests stage on fake volumes.

Listing and execution each have a 300-second deadline, configurable with `--timeout SECONDS`. On a failure the runner stops admitting new processes and lets already admitted children finish within their deadlines. On interruption or an exceptional wait, it terminates only its retained child process handles and waits for them; it does not kill by process name or manage arbitrary descendants. Tests requiring independent long-lived descendants need their own cleanup and should use a dedicated harness.

Every admitted run retains separate listing/test logs, output directories and `summary.json` under a unique `_build/qa/targeted-tests` directory. `--log-dir` selects a new directory under `_build`; existing directories are refused. The summary records selected workspace, executable SHA-256, package cwd, filters, counts, process limit, thread budget, whether render snapshots were requested, deadlines, errors and cases not run. Exit codes are 0 for success, 1 for test/execution failure, 2 for invalid inputs before execution, and 130 for interruption. Invalid receipts/selectors are rejected before any test child starts. Logs and artifacts remain available after failure.

The focused regression command is:

```text
py -3 -B -m unittest discover -s scripts/tests -p test_targeted_test_runner.py -v
```

All build, packaging, deployment and CI-planner script tests run with
`py -3 -B -m unittest discover -s scripts/tests` (Linux: `python3 -B ...`); each
test module can also run on its own (`py -3 -B scripts/tests/test_deploy_build.py`).
Windows script tests that run PowerShell helpers use PowerShell 7 when it is installed
and Windows PowerShell 5.1 otherwise. On Windows, CI also runs the PowerShell
deployment, registration, build-deployment retirement and shortcut checks
(`scripts/tests/test-*.ps1`) under both PowerShell 7 and Windows PowerShell 5.1.

The native user-command tests of the platform crates and `midnight-ui` share one
argv/cwd/environment probe executable. It is cached in
`_build/goal/commands/native/<key>/`, where the key hashes the compiler, its version,
the compile flags and the probe sources. The first test to need it compiles it once,
holding an OS file lock that other test threads and processes wait for; every
requested probe name is then a hard link to that file. A cold run therefore compiles
the probe once per key, and a warm run does not compile it at all. Every test that
builds this probe needs `rustc` on `PATH`, or `RUSTC` naming it, on any operating
system, because the probe is compiled with it (`compiler_identity` in
`scripts/tests/user-command-probe/assertions.rs`); without it these tests fail. They
are the `user_commands` integration tests and `user_command_terminal_tests` of both
platform crates and `tree_user_command_process::` in the `midnight-ui` integration
binary; the failure was observed on Linux.

Those tests use actual subprocesses and a socket rendezvous to prove simultaneous execution, bounded admission, output isolation and cleanup of owned processes while an unrelated process stays alive. They also cover filter coalescing, no-match refusal, receipt/path and independent-workspace validation, changed artifacts and anchored result parsing. They demonstrate runner behavior, not a Rust throughput improvement; report measured elapsed times with the actual selected suite and cache state when comparing runs.

### Isolated X11 tests

The ignored X11 drag/drop and clipboard tests of `midnight-platform-linux` use XTEST
input and clipboard ownership on a private Xvfb display, and run only when opted in.
CI's Linux `check` job runs them from `crates/midnight-platform-linux` with the library
test executable `$executable` (CI takes it from
`scripts/ci-workspace-tests.py artifact midnight-platform-linux lib`; locally, use the
one that `cargo test -p midnight-platform-linux --lib --locked --no-run` reports):

```sh
xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" \
    env MIDNIGHT_DND_TEST_ISOLATED=1 "$executable" file_drag_drop:: --ignored --test-threads=1
xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp -noreset" \
    env -u WAYLAND_DISPLAY MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1 "$executable" \
    clipboard::tests::x11_ file_clipboard::x11::tests:: --ignored --test-threads=1
```

`-noreset` keeps Xvfb from resetting when its last client disconnects; during a reset
it refuses connections, such as the first one of the next drag/drop test. The
clipboard session test refuses to run while `WAYLAND_DISPLAY` is set, because arboard
would then publish its text on the real desktop clipboard through Wayland.
`xvfb-run` needs the `xvfb` and `xauth` packages. Never set either opt-in variable on
your working desktop display.

### Installed archive worker qualification

The ignored real-worker tests of `midnight-filesystem-host` launch the independently
built archive worker from a [private installation root](#private-plugin-installation-root).
`test:worker` and `test:nested_worker` (archives inside archives) install
`ARCHIVE_WORKER` below `ARCHIVE_TEST_INSTALL_ROOT`;
`test:streaming_worker` and the library's `captured_source_tests::` use the package
installed at `ARCHIVE_CAPTURE_INSTALLATION/filesystems/archive`; `test:discovery_kinds`
creates its packages below `HOST_TEST_INSTALL_ROOT`. CI's `check` job runs all of them
with one root for all three variables, except the manual measurement
`real_sandboxed_worker_lists_archives_of_millions_of_members` in `test:worker`. That
test lists the archives that the archive plugin's `measure_index_memory` keeps
(`MIDNIGHT_ARCHIVE_MEMORY_KEEP`; see the plugin's README), named in
`MIDNIGHT_ARCHIVE_LARGE` with `MIDNIGHT_ARCHIVE_LARGE_MEMBERS` members each. To run
them locally:

1. Test, build and package the worker with a released worker SDK, for example
   `plugins/midnight-fs-archive/scripts/build.ps1 -SdkConfig <registry.toml> -BuildRoot _build/archive-ci -OutputDirectory _build/archive-ci/package/archive -Test -Offline`
   (Linux: `bash plugins/midnight-fs-archive/scripts/build.sh <registry.toml> <build root> <package> debug --test --offline`).
   Copy the package directory to `<root>/filesystems/archive`; on Linux make its
   directories and the `midnight-fs-archive` executable `chmod 700` and its other
   files `chmod 600`.
2. Export the plugin's owned format, cancellation and password fixtures (`password.zip`
   and `password.7z`, whose password the streaming test knows):
   `cargo test --config <registry.toml> --manifest-path plugins/midnight-fs-archive/Cargo.toml --test worker_fixtures --locked --offline -- --ignored`
   with `MIDNIGHT_ARCHIVE_FIXTURES` set to a directory below `_build`. To reuse the
   test build of step 1 instead of compiling everything again, also set
   `CARGO_TARGET_DIR` to its build root and `CARGO_INCREMENTAL=0`,
   `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`, as the build scripts do.
3. Write the large read-only ZIP with `py -3 -B scripts/archive-test-zip.py _build/archive-test.zip`
   (Linux: `python3 -B`). It is above the former 64 MiB source limit and never
   replaces an existing file.
4. Set `HOST_TEST_INSTALL_ROOT`, `ARCHIVE_TEST_INSTALL_ROOT` and
   `ARCHIVE_CAPTURE_INSTALLATION` to `<root>`, `ARCHIVE_WORKER` to the packaged
   executable, and `MIDNIGHT_ARCHIVE_FIXTURES` and `MIDNIGHT_ARCHIVE_TEST_ZIP` to the
   fixtures. Then run `cargo test -p midnight-filesystem-host` with `--test worker`,
   `--test nested_worker`, `--test streaming_worker`, `--test discovery_kinds` and
   `--lib captured_source_tests::`,
   each followed by `-- --ignored --test-threads=1`. Without the generated archives, add
   `--skip real_sandboxed_worker_lists_archives_of_millions_of_members` to `--test worker`.

### Windows Copy/Move qualification volume

On Windows, the engine's Copy/Move qualification tests (for example
`copy_planning_tests` and the scheduler, concurrency-regression, peer-refusal and
File List output tests) need exact native lookup keys for the temporary folders they
create. The platform reports those keys only on an NTFS volume whose persistent 8.3
short-name creation is disabled ([Windows namespace pins](TRANSFERS.md#windows-namespace-pins)).
Elsewhere these tests fail; each failing `copy_planning_tests` case then reports first
that its fixture "requires a supported local lookup domain". The application itself is
unaffected: Copy/Move on such a volume takes the opaque lane with full pin refreshes.

The tests create their folders in `TEMP`/`TMP`. `scripts/cargo.ps1`, the targeted runner
(`scripts/run-targeted-tests.py`) and CI put those below the checkout's `_build`, so
there the checkout's volume decides; a plain `cargo test` uses your profile's temporary
folder, normally on `C:`. Check a volume from an elevated prompt with
`fsutil 8dot3name query D:`: it must report
`The volume state is: 1 (8dot3 name creation is disabled)`. The registry line and the
summary line do not count, because a machine-wide registry setting does not set the
volume state that the platform reads. System volumes usually still create 8.3 names.
To qualify a run, point `TEMP` and `TMP` at a folder on a qualified NTFS volume (for
example the checkout's `_build/tmp` on a data volume), or disable short-name creation
on a data volume with `fsutil 8dot3name set D: 1` (elevated; it applies to names
created afterwards, and other software then gets no 8.3 names on that volume). ReFS
(including Dev Drive) and FAT volumes never qualify. `MIDNIGHT_TEST_NAMESPACE_LOOKUP`
forces a lookup mode only in `midnight-platform-windows`'s own test build, not for
engine tests.
