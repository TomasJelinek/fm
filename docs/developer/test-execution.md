# Test execution

## Full validation

`scripts/validate.ps1` (Windows) and `scripts/validate.sh` (Linux) are the
full-validation entry point: they run every test that CI
(`.github/workflows/ci.yml`) runs for the host platform, with CI's own test commands,
and build only the SDK bundles, packages and fixtures that those tests need.
`build.bat --validate` runs `validate.ps1` after its build steps and before
the private deployment; `./build.sh --validate` runs `validate.sh` after its last
build step (native backend only). Run them on their own, from the repository root,
once the toolchain and Python 3.12+ are installed. Synopsis (square brackets mark
optional arguments, `|` alternatives, capitals values you supply):

```text
pwsh -NoProfile -File scripts\validate.ps1 [debug|release] [--strict] [--offline] [--dry-run] [--jobs N] [--only ID[,ID...]] [--plugin-sdk CONFIG] [--worker-sdk CONFIG] [--web-sdk CONFIG]
bash scripts/validate.sh [debug|release] [--strict] [--offline] [--dry-run] [--jobs N] [--only ID[,ID...]] [--plugin-sdk CONFIG] [--worker-sdk CONFIG] [--web-sdk CONFIG]
```

For example, validate a release build on Windows with at most eight parallel jobs and
fail on any skipped CI step, or rerun only the workspace tests on Linux:

```powershell
pwsh -NoProfile -File scripts\validate.ps1 release --strict --jobs 8
```

```sh
bash scripts/validate.sh --only workspace-tests
```

| Option | Meaning |
| --- | --- |
| `debug` or `release` | Profile of the `host-build` and `smoke-test` steps (default `debug`); tests always use the debug test profile. |
| `--strict` | A step skipped for a missing prerequisite fails the run. |
| `--offline` | Forbids downloads, as `CARGO_NET_OFFLINE=true` does. |
| `--dry-run` | Prints every step with its commands, variables and skip reasons, and runs nothing. |
| `--jobs N` | Limits Cargo's build jobs, libtest's test threads and the native dav1d builds to N. |
| `--only ID[,ID...]` | Runs the named steps (IDs from the [step table](#validation-steps)) and the steps they need. |
| `--plugin-sdk CONFIG` | Uses an already packaged plugin SDK bundle, CONFIG being its `.cargo/config.toml`, instead of packaging one. |
| `--worker-sdk CONFIG` | The same for the worker SDK bundle's `.cargo/registry.toml`. |
| `--web-sdk CONFIG` | The same for the web SDK's `config.toml`. |

The `--*-sdk` options use an already packaged SDK bundle instead of packaging one, as
CI's plugin jobs download theirs.

Builds and tests use every logical CPU: Cargo's build jobs, libtest's test threads
and the native dav1d builds of the raster plugins. `--jobs N` limits all of them to N
(it sets `CARGO_BUILD_JOBS` and `RUST_TEST_THREADS` for every step);
`CARGO_BUILD_JOBS` and `RUST_TEST_THREADS` set by the caller have the same effect.
`--dry-run` prints every step with its commands, the variables they use and why a step
would be skipped, and runs nothing.

### Validation steps

Both scripts list the same 42 steps in CI's job
order (`test_validate.py` keeps them in step); a step that belongs to the other platform
is listed as skipped:

| CI job | Steps |
| --- | --- |
| `public-sdk` | `sdk-api` (tests), `sdk-abi` (C/Rust layouts; MSVC on Windows), `sdk-package` |
| `public-worker-sdk` | `sdk-protocols` (tests of the [worker](../glossary.md#worker), thumbnail, filesystem, image-viewer and web-viewer APIs), `sdk-web-package`, `sdk-worker-package` |
| `viewers`, `viewer-conformance` | `viewer-basic`, `viewer-source`, `viewer-markdown` (the test command of their `scripts/ci.*`, then their package build), `viewer-conformance` (tests, fixture build and the native C loader test) |
| `image-viewer`, `thumbnail-image` | `viewer-image`, `viewer-pdf`, `thumbnail-image` (the test commands of their `scripts/ci.*`; the two viewers then build their packages) |
| not in CI | `thumbnail-video` (needs an FFmpeg 9.0.2 prefix; it also runs the platform decoder helpers' tests, which CI runs in its `thumbnail-video-platform` job), `html-viewer` (`--no-default-features`, without native CEF) |
| `archive-filesystem`, `filesystem-libraries` | `archive` (tests, release package, fixture export), `filesystem-api`, `filesystem-native` and `filesystem-network` (tests, release package) |
| `scripts-and-lint` | `test-install`, `script-tests`, `decoder-native-tests`, `powershell-tests-7`, `powershell-tests-5` |
| `check` | `workspace-tests`, `trash-qualification`, `bad-descriptor`, the eleven steps that run ignored tests after the gates (`host-plugin-ignored`, `viewer-engine-ignored`, `native-fs-ignored`, `engine-native-ignored`, `network-metadata-ignored`, `app-native-viewers`, `image-process-ignored`, `pdf-process-ignored`, `archive-worker-ignored`, `archive-nested-ignored`, `archive-streaming-ignored`), `x11-dnd`, `x11-clipboard` |
| `release` | `host-build` and `smoke-test` for the selected profile |

CI runs the `public-sdk` and `public-worker-sdk` jobs on ubuntu-24.04 only; its
Windows plugin jobs download the bundles that those jobs package. Windows validation
runs these steps in addition: it packages the bundles its plugin steps need, and
`sdk-api`, `sdk-abi` (`api/scripts/check-abi.ps1`, which CI never runs) and
`sdk-protocols` are extra Windows coverage.

`workspace-tests` runs CI's gates ([Workspace gates](#workspace-gates)) with
`scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast`: the same Cargo
selections and features, including doc tests and example builds (its `--jobs N`
limits Cargo's jobs and the test threads, as `validate`'s does), but it retires no
test executable, needs no `CI=true` or `_build` target directory, adds Cargo's
`--no-fail-fast` and runs every gate after a failure (exiting with the first failing
status). The later qualification steps (the steps that run ignored tests against
installed packages, listed in the `check` row of the
[step table](#validation-steps)) reuse those executables through its `run` and
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
temporary files go to `_build/tmp`, and the Windows
[private installation root](#private-installation-root) is created in `%USERPROFILE%`.
`test-install` mirrors CI's `private-test-installation` and `plugin-packages` actions
with the packages this run built: on Windows a new `%USERPROFILE%\MidnightViewerTests-<guid>` whose DACL
grants only the user, SYSTEM and Administrators; on Linux a mode-700
`_build/tmp/viewer-tests-*` directory. It sets `HOST_TEST_INSTALL_ROOT` and the other
installation variables of that action
([Test environment variables](#test-environment-variables)) for every later step, and is
removed when
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
`--only workspace-tests` reruns the workspace tests with their private installation
root.
A flag given a value, such as `--strict=no`, is rejected with exit status 2.

Validation continues after a failure. Each step's output is shown and saved as
`_build/validate/logs/<run>/NN-<step>.log`, next to the generated step script
(Windows) and `summary.txt`. The exit status is 0 when no step failed, 1 when a step
failed (or a `--strict` skip), 2 for invalid arguments and 127 when cargo, rustc or
Python 3.12+ is missing.

`summary.txt` is the closing summary, also printed at the end of the run. After a
header line (`Validation summary: <platform> (<target>), profile <profile>, started
<time>`) it has one row per step with these columns:

| Column | Meaning |
| --- | --- |
| `#` | The step's number in CI's job order. |
| `Step` | The step ID, as in the [step table](#validation-steps). |
| `Result` | `PASS`, `FAIL` or `SKIPPED`. |
| `Time` | The step's duration; `-` for a skipped step. |
| `Passed`, `Failed`, `Ignored` | The counts of the step's libtest and unittest summaries, empty when it printed none. |
| `CI job / reason` | The step's CI job, or the reason it was skipped or failed. |

The rows are followed by the totals (`Steps: … passed, … failed, … skipped in <time>.
Tests: … passed, … failed, … ignored.`), the `--only` note when steps were selected, the
`WARNING` line for skipped CI steps, `VALIDATION PASSED` or `VALIDATION FAILED`, and the
log folder.

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
`CARGO_PROFILE_TEST_DEBUG=0` and `CARGO_INCREMENTAL=0` (set them yourself to reproduce
a CI build exactly; see [CI jobs and build time](#ci-jobs-and-build-time));
the filesystem API tests run once instead of in both filesystem jobs; and the two
modules that CI does not build (`thumbnail-video` and `html-viewer`, the "not in CI" row
of the [step table](#validation-steps)) are included when their inputs exist. CI's `thumbnail-video-platform` job has
no step of its own; its tests run inside `thumbnail-video` and are skipped, without a
WARNING line, when no FFmpeg prefix exists.
Validation does not run inside `build.sh`'s Docker backend: the build image has no
Xvfb, mounts the source read-only and has not been tested for this, so
`build.sh --validate` refuses Docker (exit status 2).

## CI jobs and build time

CI's `check` and `release` jobs start with the run instead of after the plugin jobs.
The workspace gates use no plugin package, only the empty
[private installation root](#private-installation-root) (`HOST_TEST_INSTALL_ROOT`) that
the `private-test-installation` action creates; every test that reads an installed
package is `#[ignore]`d and runs in a later qualification step. So both jobs build and test first, and only then does the `plugin-packages`
action wait for this run's viewer and filesystem packages
(`scripts/ci-wait-for-artifacts.py`), download them and install them into that root.
The wait lists every artifact its pattern downloads select, so none starts with a
partial set; it fails at once when a plugin job leg of the same runner label has
finished without success, and after 60 minutes. Formatting, Clippy, the source size
check and the script, native helper and PowerShell tests run beside them in
`scripts-and-lint`.

`check` and `scripts-and-lint` build on every CPU of the runner (four, with 16 GB of
memory; Cargo's default, as nothing sets a job count) and with the default linkers.
`check`, `scripts-and-lint` and
`release` restore the compiled third-party dependencies of the
root workspace from the Actions cache
([Swatinem/rust-cache](https://github.com/Swatinem/rust-cache), pinned by commit);
workspace crates are always compiled. Pushes to `main` save a new entry when its key
changes: the job, the installed rustc versions, any Cargo manifest, lockfile or Cargo
configuration, and the `CARGO_*` and `RUST*` environment. Other runs only restore; when
no key matches they take the newest entry of the same job, toolchains and environment.
The plugin jobs cache their downloads only. Measurements and the decisions behind
these settings: [CI time](../verification/2026-11-21-ci-time.md).

Locally too, Cargo, libtest and the validation and test scripts use every logical CPU;
no Cargo configuration sets a job count. Limit them where several builds share a
machine, or where memory is short (one generated UI crate alone needs up to 7 GB): with
Cargo's `-j N`, the scripts' `--jobs N` (`validate.*`, `ci-workspace-tests.py`) or
`CARGO_BUILD_JOBS` and `RUST_TEST_THREADS`. On Windows,
`CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld.exe` (the pinned toolchain's LLVM
linker) can build the test executables faster on a developer machine; on the hosted
runners it made no measurable difference, so CI does not use it
([CI time](../verification/2026-11-21-ci-time.md)). Set it only for test builds: release
builds keep the MSVC linker.

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
absolute folder ([acceptance results](acceptance.md#results)). Its only test target
requires the `acceptance` feature of `crates/midnight-acceptance`, so
`cargo test --workspace` and `scripts/ci-workspace-tests.py` never build or run it.
See [the acceptance suite](acceptance.md) and its
[Windows results](acceptance/windows.md).

The suite's own test target is built at `opt-level = 2` even in the debug test profile
(`[profile.dev.package.midnight-acceptance]` in the root `Cargo.toml`): its probes copy,
hash and compare whole rendered frames many times a second, and unoptimized those loops,
not the product, set the pace of the viewer and UI polish scenarios. The product crates
it drives keep their debug profiles. Each scenario's report ends with `timing` notes,
the time spent per named phase ([acceptance results](acceptance.md#results)); look there before
making a scenario faster ([acceptance time](../verification/2026-11-24-acceptance-time.md)).

## Targeted test execution

Use `scripts/run-targeted-tests.py` for independent **headless Rust libtest** targets that have already been built. It does not change CI coverage, compile code, discover executables by glob, or delete build artifacts. Native dialogs, desktop input, renderer pacing and profiling are unsupported: run their dedicated tools serially, with compilers and other test processes idle. For example, `scripts/probe-keyboard-search.ps1` drives the release `native_keyboard_search` example with real Windows keyboard input on a desktop session (see the [native keyboard input verification](../verification/2026-11-07-keyboard-native-input.md)). Idle event-loop wakeups are measured the same way: `MIDNIGHT_IDLE_WAKEUP_PROBE=<warmup_s>:<seconds>[:thumbnails]` makes `midnight-explorer` use in-memory default preferences, count winit wakeups by cause after the warmup and print one `idle_wakeups=` line to stderr before it quits. Only the preferences are isolated: the probe uses the plugins installed next to `midnight-explorer` and the real preview cache, so set `LOCALAPPDATA` (Windows) or `XDG_CACHE_HOME` (Linux) to a scratch directory for the run. Run one probe at a time with the pointer outside the window (see the [idle wakeup record](../verification/2026-11-18-ui-idle-wakeups.md)).

A target directory shared between checkouts of different commits (a `CARGO_TARGET_DIR` outside the checkout) can silently link another checkout's code. Cargo names a path package's artifacts by its path relative to the workspace root, so every checkout of this workspace uses the same artifact names, and it judges freshness by modification times, so a checkout whose sources are older than another checkout's artifacts reuses them unchanged. This holds for every path package of the dependency graph, not only the workspace members: `filesystem-api/*`, `api/*` and the vendored crates (`vendor/*`) too. Give each checkout its own target directory, or before building in another checkout run `cargo clean -p <package id>` for every path package (`cargo metadata` with dependencies lists them as the packages whose `source` is null; registry dependencies may stay).

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
per test file; select one file's tests with a module filter such as `shortcuts::`.
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
unit tests that those sources declare: on Windows a large share of that executable's
tests are library unit tests run again ([CI time](../verification/2026-11-21-ci-time.md)).
No `cfg` tells the library's test build from such an include, and declaring the test
modules inside a macro would hide their files from `cargo fmt`, which follows no `mod`
inside a macro call; a new `#[path]` include of a source with unit tests adds them again.

On Windows with Microsoft Defender real-time protection, the first start of a newly
linked test executable takes a second or more, growing with its size, and a later start a
small fraction of that. Most of the cost follows new file content, and it returns after
some minutes without a start. A test run right after a build pays it once per new
executable, so fewer and smaller executables save time even where they hold no test;
Linux has no such cost. When comparing test times, compare runs in the same state (each
executable newly linked, or each one started moments before) and say which. Single first
starts vary widely, so compare first starts of the executables that a change adds,
removes or resizes over several samples each, not as the difference of two sums of
single starts ([test speed pass](../verification/2026-11-18-test-speed-pass.md)).

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
with different features. Record the exact commands and the resolved graph and
features, state any differences, and claim results only for that scope.
Independent plugins still use their own workspace and normal configuration;
this does not require blanket workspace builds for independent plugin checks.

<a id="workspace-gates"></a>

CI's `check` job runs the workspace in `scripts/ci-workspace-tests.py` gates
(`cargo test --workspace` with every member's active features, one gate per
target kind and bounded batches of integration names; a same-name group such as
the crates' `integration` binaries stays one indivisible gate). Successful
integration executables are retired to bound disk use. Its later ignored-test
steps keep that selection instead of `cargo test -p`, which would rebuild shared
dependencies with one package's narrower features:

```text
python -B scripts/ci-workspace-tests.py run midnight-image-viewer-host test:native --ignored --test-threads=1
python -B scripts/ci-workspace-tests.py artifact midnight-explorer bin:midnight-explorer
```

`run PACKAGE lib|test:NAME|bin:NAME [LIBTEST ARGS...]` reuses (or relinks) that
package's test executable and runs it from the package directory; `artifact`
prints its path. Neither mode retires anything or needs `CI=true`. Locally,
`python -B scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast` runs
all gates without retiring executables ([full validation](#full-validation)).

The next rules concern the inline-tree watch fixtures; hints, watch fences, the
acknowledged projection and occurrences are described in
[Native watches and reconciliation](tree-panel.md#native-watches-and-reconciliation).
Fixture readiness is not a maintenance-quiescence fence. Exact pre-ACK capture
fixtures (captures made before the UI acknowledges a tree publication) must account
for independent startup maintenance without weakening
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
new registration counts for it as well. The subscription may belong to either panel:
these fixtures root the other panel at `destination`, so its subscription can supply
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

<a id="private-plugin-installation-root"></a>

### Private installation root

Plugin discovery accepts an installation that other accounts can change only through permissions and reports it (`midnight_plugin_host::installation_sharing`), but the tests that inspect that report, the runtime registrations and the strict checks (`trust::check_private`) need fixtures whose ancestors no other account can change. Tests that install plugin fixtures (`midnight-image-viewer-host` catalog and startup tests, the opt-in `midnight-plugin-host`, `midnight-filesystem-host` and engine filesystem-plugin tests) therefore need a [private installation root](../glossary.md#private-installation-root): a folder that only you, the system and administrators can change, named by `HOST_TEST_INSTALL_ROOT`. By default they create fixtures below the checkout's `_build` or the temporary directory, which works for a checkout under your user profile. A checkout whose ancestors other accounts may change, for example on a secondary Windows drive with the default ACL (Authenticated Users: Modify), or a temporary directory redirected there, cannot pass: the `midnight-image-viewer-host` fixtures then stop with an explanation. Create a private installation root instead, as CI's `private-test-installation` action does.

On Windows, in PowerShell:

1. Create a new folder in your profile:

   ```powershell
   $root = Join-Path $env:USERPROFILE ('MidnightViewerTests-' + [guid]::NewGuid().ToString('N'))
   New-Item -ItemType Directory -Path $root | Out-Null
   ```

2. Give it a protected ACL that grants full control only to you, SYSTEM and
   Administrators:

   ```powershell
   $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
   $acl = New-Object Security.AccessControl.DirectorySecurity
   $acl.SetSecurityDescriptorSddlForm("D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)", 'Access')
   Set-Acl -LiteralPath $root -AclObject $acl
   ```

3. Point `HOST_TEST_INSTALL_ROOT` at it:

   ```powershell
   $env:HOST_TEST_INSTALL_ROOT = $root
   ```

On Linux, use a `chmod 700` directory on a native filesystem whose ancestors are not group- or world-writable (sticky `/tmp`-style ancestors are accepted); drvfs mounts such as `/mnt/d` report permissive modes and are refused. For example, in your home folder:

```sh
root=$(mktemp -d "$HOME/midnight-viewer-tests.XXXXXXXX")
chmod 700 "$root"
export HOST_TEST_INSTALL_ROOT="$root"
```

Archive tests that run a real [worker](../glossary.md#worker) (the opt-in `midnight-filesystem-host` worker and streaming tests, and engine archive tests with installed workers) stage decoded members on the real private [spool](../glossary.md#spool) volume (`%TEMP%` on Windows, usually `~/.cache` on Linux). Keep more than max(1 GiB, 2% of its capacity) plus the largest member a test stages (and at least 64 MiB) free there: a member that does not fit above the reserve is refused before staging, and content of undeclared length stops at the reserve, both with a disk-full error. Unit tests stage on fake volumes.

Listing and execution each have a 300-second deadline, configurable with `--timeout SECONDS`. On a failure the runner stops admitting new processes and lets already admitted children finish within their deadlines. On interruption or an exceptional wait, it terminates only its retained child process handles and waits for them; it does not kill by process name or manage arbitrary descendants. Tests requiring independent long-lived descendants need their own cleanup and should use a dedicated harness.

Every admitted run retains separate listing/test logs, output directories and `summary.json` under a unique `_build/qa/targeted-tests` directory. `--log-dir` selects a new directory under `_build`; existing directories are refused. The summary records selected workspace, executable SHA-256, package cwd, filters, counts, process limit, thread budget, whether render snapshots were requested, deadlines, errors and cases not run ([summary.json](#summaryjson)). Exit codes are 0 for success, 1 for test/execution failure, 2 for invalid inputs before execution, and 130 for interruption. Invalid receipts/selectors are rejected before any test child starts. Logs and artifacts remain available after failure.

#### summary.json

One indented UTF-8 JSON object:

| Field | Type | Meaning |
| --- | --- | --- |
| `workspace` | string | The selected workspace folder. |
| `exit_code` | number | The runner's exit code: 0, 1 or 130. |
| `jobs` | number | The process limit after clamping. |
| `thread_budget` | number | The total libtest thread budget. |
| `threads_per_process` | number | The `--test-threads` value given to each child. |
| `render_snapshots` | boolean | Whether `--render-snapshots` was given. |
| `peak_processes` | number | The most child processes that ran at once. |
| `interrupted` | boolean | Whether the run was interrupted. |
| `timeout_seconds` | number | The listing and execution deadline of each child. |
| `cleanup_errors` | array of strings | Children that could not be terminated or waited for. |
| `cases` | array of case objects | One per selected target, in selection order. |

| Field of a case | Type | Meaning |
| --- | --- | --- |
| `package`, `kind`, `target` | string | The selected package and target, for example `"midnight-core"`, `"test"`, `"integration"`. |
| `filters` | array of strings | The requested libtest filters. |
| `all_tests` | boolean | Whether the whole target was requested (`--target`). |
| `status` | string | `"passed"`, `"failed"` or `"not-run"` (not admitted after another case failed or the run was interrupted). |
| `executable`, `sha256` | string | The test executable and its SHA-256. |
| `cwd` | string | The package directory the executable ran in. |
| `list_log`, `test_log` | string | The listing and test logs. |
| `filter_matches` | object | Listed tests matched per filter. |
| `matched` | number | Tests selected from the listing. |
| `exit_code` | number | The child's exit status, when it ran. |
| `counts` | object or `null` | `passed`, `failed`, `ignored`, `measured` and `filtered` from the last complete libtest summary line. |
| `error` | string | Why the case failed or was not run, when it did not pass. |
| `elapsed_seconds` | number | The case's duration. |

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
binary.

Those tests use actual subprocesses and a socket rendezvous to prove simultaneous execution, bounded admission, output isolation and cleanup of owned processes while an unrelated process stays alive. They also cover filter coalescing, no-match refusal, receipt/path and independent-workspace validation, changed artifacts and anchored result parsing. They demonstrate runner behavior, not a Rust throughput improvement; report measured elapsed times with the actual selected suite and cache state when comparing runs.

### Isolated X11 tests

The ignored X11 drag/drop and clipboard tests of `midnight-platform-linux` use XTEST
input and clipboard ownership on a private Xvfb display, and run only when opted in.
CI's Linux `check` job runs them from `crates/midnight-platform-linux` with the library
test executable in `$executable`, which `scripts/ci-workspace-tests.py artifact`
prints (locally, the one that
`cargo test -p midnight-platform-linux --lib --locked --no-run` reports works too). From
the repository root:

```sh
executable=$(python3 -B scripts/ci-workspace-tests.py artifact midnight-platform-linux lib)
cd crates/midnight-platform-linux
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
built archive worker from a [private installation root](#private-installation-root).
`test:worker` and `test:nested_worker` (archives inside archives) install
`ARCHIVE_WORKER` below `ARCHIVE_TEST_INSTALL_ROOT`;
`test:streaming_worker` and the library's `captured_source_tests::` use the package
installed at `ARCHIVE_CAPTURE_INSTALLATION/filesystems/archive`; `test:discovery_kinds`
creates its packages below `HOST_TEST_INSTALL_ROOT`
([Test environment variables](#test-environment-variables)). CI's `check` job runs all
of them with one root for all three variables, except the manual measurement
`real_sandboxed_worker_lists_archives_of_millions_of_members` in `test:worker`. That
test lists the archives that the archive worker plugin's `measure_index_memory` test
keeps (`MIDNIGHT_ARCHIVE_MEMORY_KEEP`; see the
[archive worker README](../../plugins/midnight-fs-archive/README.md)), named in
`MIDNIGHT_ARCHIVE_LARGE` with `MIDNIGHT_ARCHIVE_LARGE_MEMBERS` members each.

The commands below are PowerShell 7, run at the repository root; on Linux run them in
`pwsh` too, as CI does. Before you start:

- Create a private installation root as
  [Private installation root](#private-installation-root) describes, so that
  `HOST_TEST_INSTALL_ROOT` names it, and keep it in `$root`:

  ```powershell
  $root = $env:HOST_TEST_INSTALL_ROOT
  ```

- Set `$sdk` to the `.cargo/registry.toml` of a public worker SDK bundle, a released
  one or one that a normal build packaged. `build.bat` records the one it packaged as
  `config` in `sdk-state.json` in `_build/thumbnailers`, `_build/image-viewer` or
  `_build/pdf-viewer`; `./build.sh` writes its path into `linux-sdk-config` in the
  same folders. For example, on Windows:

  ```powershell
  $sdk = (Get-Content _build/thumbnailers/sdk-state.json -Raw | ConvertFrom-Json).config
  ```

  On Linux:

  ```powershell
  $sdk = Get-Content _build/thumbnailers/linux-sdk-config
  ```

Then:

1. Test, build and package the worker:

   ```powershell
   & plugins/midnight-fs-archive/scripts/build.ps1 -SdkConfig $sdk -BuildRoot _build/archive-ci -OutputDirectory _build/archive-ci/package/archive -Test -Offline
   ```

   On Linux: `bash plugins/midnight-fs-archive/scripts/build.sh $sdk _build/archive-ci _build/archive-ci/package/archive debug --test --offline`.
2. Copy the package folder into the private installation root:

   ```powershell
   New-Item -ItemType Directory -Force -Path (Join-Path $root 'filesystems') | Out-Null
   Copy-Item -Recurse -LiteralPath _build/archive-ci/package/archive -Destination (Join-Path $root 'filesystems/archive')
   ```

3. On Linux only, make the copied package private: its folders and the
   `midnight-fs-archive` executable mode 700, its other files mode 600.

   ```powershell
   find "$root/filesystems/archive" -type d -exec chmod 700 '{}' '+'
   find "$root/filesystems/archive" -type f -exec chmod 600 '{}' '+'
   chmod 700 "$root/filesystems/archive/midnight-fs-archive"
   ```

4. Export the plugin's owned format, cancellation and password fixtures (`password.zip`
   and `password.7z`, whose password the streaming test knows) to an absolute folder
   below `_build`:

   ```powershell
   $env:MIDNIGHT_ARCHIVE_FIXTURES = Join-Path (Get-Location) '_build/archive-fixtures'
   cargo test --config $sdk --manifest-path plugins/midnight-fs-archive/Cargo.toml --test worker_fixtures --locked --offline -- --ignored
   ```

   - To reuse the test build of step 1 instead of compiling everything again, first
     set `CARGO_TARGET_DIR` to its build root and `CARGO_INCREMENTAL=0`,
     `CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0`, as the build
     scripts do.
5. Write the large read-only ZIP. It is larger than 64 MiB and never replaces an
   existing file.

   ```powershell
   $env:MIDNIGHT_ARCHIVE_TEST_ZIP = Join-Path (Get-Location) '_build/archive-test.zip'
   py -3 -B scripts/archive-test-zip.py $env:MIDNIGHT_ARCHIVE_TEST_ZIP
   ```

   On Linux use `python3 -B` instead of `py -3 -B`.
   - When `_build/archive-test.zip` already exists, run only the first line: the
     script refuses to replace the file and exits with status 1.
6. Point the remaining variables at the root and the packaged worker (on Linux the
   executable has no `.exe`):

   ```powershell
   $env:ARCHIVE_TEST_INSTALL_ROOT = $root
   $env:ARCHIVE_CAPTURE_INSTALLATION = $root
   $env:ARCHIVE_WORKER = (Resolve-Path _build/archive-ci/package/archive/midnight-fs-archive.exe).Path
   ```

7. Run each test target with `--ignored --test-threads=1`:

   ```powershell
   cargo test -p midnight-filesystem-host --test worker -- --ignored --test-threads=1 --skip real_sandboxed_worker_lists_archives_of_millions_of_members
   cargo test -p midnight-filesystem-host --test nested_worker -- --ignored --test-threads=1
   cargo test -p midnight-filesystem-host --test streaming_worker -- --ignored --test-threads=1
   cargo test -p midnight-filesystem-host --test discovery_kinds -- --ignored --test-threads=1
   cargo test -p midnight-filesystem-host --lib captured_source_tests:: -- --ignored --test-threads=1
   ```

   - The `--skip` leaves out the manual measurement; drop it when
     `MIDNIGHT_ARCHIVE_LARGE` and `MIDNIGHT_ARCHIVE_LARGE_MEMBERS` name generated
     archives.

### Windows Copy/Move qualification volume

On Windows, the engine's Copy/Move qualification tests (for example
`copy_planning_tests` and the scheduler, concurrency-regression, peer-refusal and
File List output tests) need exact native lookup keys for the temporary folders they
create. The platform reports those keys only on an NTFS volume whose persistent 8.3
short-name creation is disabled ([Windows namespace pins](transfers.md#windows-namespace-pins)).
Elsewhere these tests fail; each failing `copy_planning_tests` case (and each scheduler
test that runs two jobs side by side) then reports first that its fixture "requires a
supported local lookup domain". The application itself is unaffected: Copy/Move on such
a volume takes the opaque lane, the path of a job whose native footprint is unknown
([Verification and ownership](transfers.md#verification-and-ownership)), with full pin
refreshes, and so runs alone: another job
waits for it, even beside a paused one, and says so ("Waiting for another job: jobs
whose folders cannot be confirmed run alone", the wait reason `UnconfirmedFolders`).
Tests that expect the wait reason `SameFolders` see that reason there instead.
Acceptance scenarios that check jobs side by side (`files_transfer_jobs` scenario T-J01
of the [transfers acceptance coverage](acceptance/transfers.md),
`journeys_concurrent_jobs`) therefore need their `--run-dir` on a qualified volume too.

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
(`short-names-disabled` or `unqualified`) forces a lookup mode only in
`midnight-platform-windows`'s own test build, not for engine tests.

### Test fixtures under `_build/qa`

Tests that need real files on the checkout's volume create them below `_build/qa`: the
real Windows Copy, Move and Delete operations (`transfer-real-*`), Find (`find-*`) and the
native UI harness (`native-ui-*`) below `_build/qa/operations-find`, the comparator's
polish fixtures below `_build/qa/native-dialogs`. A passing test removes its fixture. A
failing test keeps it, with its `EVIDENCE.txt` where it records one, and prints
`kept test fixture <path>`. Set `MIDNIGHT_KEEP_TEST_FIXTURES=1` to keep the fixtures of
passing tests too. A fixture that still cannot be removed after its read-only
attributes are cleared and a short wait for closing handles is named on stderr
(`could not remove test fixture <path>`); the test does not fail for it. The other tests
that write there use a temporary folder inside a fixed one, removed when the test ends,
and the ignored manual performance fixtures reuse a fixed tree.

## Test environment variables

The tests and tools on this page read these variables. Set paths as absolute paths:
a test runs in its package's folder.

| Variable | Value | Read by | Effect |
| --- | --- | --- | --- |
| `HOST_TEST_INSTALL_ROOT` | a [private installation root](#private-installation-root) | tests that install plugin fixtures, `test:discovery_kinds`, the `extracted_installation` tests | Root below which they install packages. Validation's `test-install` step and CI's `private-test-installation` action set it. |
| `ARCHIVE_WORKER` | the packaged archive worker executable | `midnight-filesystem-host` `test:worker`, `test:nested_worker` | The worker these tests install. |
| `ARCHIVE_TEST_INSTALL_ROOT` | a private installation root | the same tests | Where they install `ARCHIVE_WORKER`, in a temporary folder of their own. |
| `ARCHIVE_CAPTURE_INSTALLATION` | a private installation root that holds `filesystems/archive` | `test:streaming_worker`, the library's `captured_source_tests::` | The installed archive package they use. |
| `MIDNIGHT_ARCHIVE_FIXTURES` | a folder below `_build` | the archive worker plugin's `worker_fixtures` test (writes), `test:streaming_worker` (reads) | The exported format, cancellation and password fixtures. |
| `MIDNIGHT_ARCHIVE_TEST_ZIP` | path of the ZIP from `scripts/archive-test-zip.py` | `test:streaming_worker` | The large read-only ZIP. |
| `MIDNIGHT_ARCHIVE_LARGE` | archive paths, separated as in `PATH` (`;` on Windows, `:` on Linux) | `real_sandboxed_worker_lists_archives_of_millions_of_members` in `test:worker` | The archives to list. |
| `MIDNIGHT_ARCHIVE_LARGE_MEMBERS` | a number | the same test | Members in each of those archives. |
| `MIDNIGHT_ARCHIVE_MEMORY_KEEP` | any value | the archive worker plugin's `measure_index_memory` test | Keeps the generated archives. |
| `MIDNIGHT_THUMBNAILERS_TEST_PACKAGE` | an unpacked release's `thumbnailers` folder | the ignored `extracted_installation` tests | The package they copy ([Before publishing](../release-build.md#before-publishing)). |
| `MIDNIGHT_NATIVE_TEST_ROOT` | a folder | thumbnail cache, worker store and native UI operation tests | Parent folder of their native fixtures; unset, each uses its own default below `_build` or a temporary folder. The targeted runner passes it through. |
| `MIDNIGHT_KEEP_TEST_FIXTURES` | `1` | tests with fixtures below `_build/qa` | Keeps the fixtures of passing tests ([Test fixtures under `_build/qa`](#test-fixtures-under-_buildqa)). |
| `MIDNIGHT_UI_RENDER_DIR` | a folder | `midnight-ui` tests | Writes software-render snapshots there; the targeted runner never passes it ([Targeted test execution](#targeted-test-execution)). |
| `MIDNIGHT_UI_RENDER_FILTER` | text | `midnight-ui` tests | Writes only snapshots whose names contain it. |
| `MIDNIGHT_IDLE_WAKEUP_PROBE` | `<warmup_s>:<seconds>[:thumbnails]`, for example `5:30` | `midnight-explorer` | Measures idle event-loop wakeups and quits. |
| `MIDNIGHT_COPY_SEAL_TEST_CHILD` | a test name | the `midnight-engine` library test executable | Set by a Copy seal test for the child it starts, which then runs the test body. |
| `MIDNIGHT_DND_TEST_ISOLATED` | `1` | `midnight-platform-linux` `file_drag_drop::` tests | Opts in on a private Xvfb display ([Isolated X11 tests](#isolated-x11-tests)). |
| `MIDNIGHT_CLIPBOARD_TEST_ISOLATED` | `1` | `midnight-platform-linux` clipboard tests | The same for the clipboard tests. |
| `MIDNIGHT_TEST_NAMESPACE_LOOKUP` | `short-names-disabled` or `unqualified` | `midnight-platform-windows`'s own test build | Forces a lookup mode ([Windows Copy/Move qualification volume](#windows-copymove-qualification-volume)). |
| `MIDNIGHT_ACCEPTANCE_RUN_DIR` | an absolute folder | the acceptance suite | Its run folder instead of one below `_build/acceptance/` ([Acceptance suite](#acceptance-suite)). |

Standard variables that this page uses:

| Variable | Effect here |
| --- | --- |
| `CARGO_BUILD_JOBS`, `RUST_TEST_THREADS` | Limit Cargo's build jobs and libtest's test threads; `--jobs N` of the scripts sets both. |
| `CARGO_NET_OFFLINE=true` | Forbids downloads, like `--offline`. |
| `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`, `CARGO_INCREMENTAL=0` | Set by CI's `check` and `scripts-and-lint` jobs and by the build scripts; set them to reproduce a CI build. |
| `CARGO_TARGET_DIR` | Cargo's target folder; give each checkout its own. |
| `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld.exe` | Faster test links on a developer machine; test builds only. |
| `RUSTC` | Names `rustc` for the user-command probe when it is not on `PATH`. |
| `TEMP`, `TMP`, `TMPDIR` | Where tests create temporary folders; validation, the targeted runner and CI put them below `_build`. |
| `LOCALAPPDATA` (Windows), `XDG_CACHE_HOME` (Linux) | Point them at a scratch folder for the idle wakeup probe. |

### Manual measurements, probes and traces

No CI job, validation step or script runs the ignored tests and examples below or sets
their variables: they are measurements and native checks that a developer runs on
demand, one at a time, usually in release with `--ignored --nocapture`. Each test's
`#[ignore]` reason or the comment beside the variable gives its values and command. The
variables that a test sets only for a child process it starts are not listed.

| Source | Variables |
| --- | --- |
| `midnight-core` `tests/checksum_verification.rs`; `midnight-ui` `src/checksum_windows.rs` | `MIDNIGHT_CHECKSUM_ROWS` |
| `midnight-compare-engine` `src/binary_tests.rs` | `MIDNIGHT_SCAN_MIB` |
| `midnight-engine` `src/copy_cadence_measure_tests.rs` | `MIDNIGHT_MEASURE_SOURCE`, `MIDNIGHT_MEASURE_DESTINATION`, `MIDNIGHT_MEASURE_JOBS` |
| `midnight-engine` `src/copy_small_files_measure_tests.rs` | `MIDNIGHT_MEASURE_ROOT`, `MIDNIGHT_MEASURE_RUNS` |
| `midnight-engine` `src/copy_streamed_tests.rs` | `MIDNIGHT_STREAMED_DIR`, `MIDNIGHT_STREAMED_ITEMS`, `MIDNIGHT_STREAMED_INTENT`, `MIDNIGHT_STREAMED_MOVE_TO`, `MIDNIGHT_STREAMED_FOLDER`, `MIDNIGHT_STREAMED_SECONDS` |
| `midnight-engine` `src/copy_timing_tests.rs` | `MIDNIGHT_TIMING_MOVES`, `MIDNIGHT_TIMING_FILES` |
| `midnight-engine` `src/copy_unprotected_source_tests.rs` | `MIDNIGHT_TEST_UNQUALIFIED_ROOT` |
| `midnight-engine` `src/checksum_worker_tests.rs` | `MIDNIGHT_CHECKSUM_LISTING_FILES` |
| `midnight-engine` `src/comparison/native_bench_tests.rs` | `MIDNIGHT_COMPARE_BENCH_DIR` |
| `midnight-engine` `src/providers/tree_probe_tests.rs` | `MIDNIGHT_OBSERVED_TIMING_DIR`, `MIDNIGHT_OBSERVED_TIMING_COLD` |
| `midnight-engine` `src/archive_copy_tests.rs` | `MIDNIGHT_PERF_FILES` |
| `midnight-engine` `tests/tree_refresh/` | `MIDNIGHT_TREE_TEST_PARENT`, `MIDNIGHT_TREE_TEST_CHILD` |
| `midnight-explorer` `src/production_pool_repro.rs`, `src/production_pool_maximum_state.rs` | `MIDNIGHT_POOL_REPRO_WORK` |
| `midnight-explorer` `src/native_archive_tests.rs`, `src/native_content_tests.rs` | `MIDNIGHT_ARCHIVE_TEST_SOURCE`, `MIDNIGHT_NATIVE_MEDIA_TEST_FILES` |
| `midnight-explorer` `src/viewer_filesystem_tests.rs` | `MIDNIGHT_FIND_TEST_FILES` |
| `midnight-explorer` `src/comparator_external_native_tests.rs` | `MIDNIGHT_NATIVE_COMPARE_WORK`, `MIDNIGHT_USER_COMMAND_PROBE` |
| `midnight-explorer` `src/native_thumbnail_linux_tests.rs` | `MIDNIGHT_NATIVE_THUMBNAIL_PNG`, `MIDNIGHT_NATIVE_THUMBNAIL_CACHE` |
| `midnight-explorer` `--ui-smoke-test` | `MIDNIGHT_UI_SMOKE_DWELL_MS` (how long the started window stays open) |
| `midnight-explorer` `examples/archive_thumbnails.rs` | `MIDNIGHT_ARCHIVE_PROBE_DIRECT_DECODE` |
| `midnight-native-filesystem-host` `src/network/installed_linux_tests.rs` | `MIDNIGHT_NETWORK_TEST_ALLOW_LAN` |
| `midnight-platform-linux` `src/transfer_namespace_tests.rs` | `MIDNIGHT_CASEFOLD_TMPFS_TEST_DIR` |
| `midnight-platform-linux` `src/mutation/tests.rs` | `MIDNIGHT_MODELESS_TEST_DIR` |
| `midnight-platform-linux` `src/sort_keys_tests.rs` | `MIDNIGHT_COLLATION_TIMING_LOCALE` |
| `midnight-platform-linux` `src/file_drag_drop_tests.rs` | `MIDNIGHT_DND_IDLE_MEASUREMENT` ([Isolated X11 tests](#isolated-x11-tests)) |
| `midnight-platform-windows` `src/open_junction_tests.rs` | `MIDNIGHT_TEST_COMPATIBILITY_JUNCTION` |
| `midnight-thumbnail-host` `tests/deployment.rs` | `MIDNIGHT_DEPLOYMENT_TEST_ROOT` |
| `midnight-thumbnail-host` `tests/native_image.rs` | `MIDNIGHT_IMAGE_TEST_HELPER`, `MIDNIGHT_HANG_TEST_HELPER` |
| `midnight-thumbnail-host` `tests/native_video.rs` | `MIDNIGHT_VIDEO_TEST_HELPER`, `MIDNIGHT_PLATFORM_VIDEO_TEST_HELPER`, `MIDNIGHT_PLATFORM_VIDEO_TEST_FIXTURES` |
| `midnight-ui` `tests/busy_folder_native.rs` | `MIDNIGHT_BUSY_FILES`, `MIDNIGHT_BUSY_SECONDS` |
| `midnight-ui` `tests/delete_latency_native.rs` | `MIDNIGHT_DELETE_ROUNDS` |
| `midnight-ui` `tests/native_job_windows.rs` | `MIDNIGHT_MEASURE_JOBS` |
| `midnight-viewer-engine` `src/office_preview_native_tests.rs` | `MIDNIGHT_OFFICE_VIEWER_TEST_APPLICATION`, `MIDNIGHT_OFFICE_NATIVE_TEST_REPORT` |
| `midnight-viewer-engine` `src/pdf_scroll_probe_tests.rs` | `MIDNIGHT_PDF_SCROLL_FIXTURES` |
| `midnight-viewer-engine` `examples/seek_latency.rs` | `MIDNIGHT_VIEWER_SEEK_SCALE` |
| `midnight-viewer-ui` `src/raster_render_tests.rs` | `MIDNIGHT_RETAINED_RENDER_DIR` (writes the rendered images there) |
| `midnight-viewer-ui` `tests/wrap_control.rs` | `MIDNIGHT_VIEWER_TOOLBAR_SCREENSHOTS` (writes the toolbar images there) |
| `midnight-viewer-ui` `examples/open_latency.rs` | `MIDNIGHT_PROBE_PARENT`, `MIDNIGHT_PROBE_REPEAT`, `MIDNIGHT_PROBE_SETTLE_MS`, `MIDNIGHT_PROBE_PAUSE_MS`, `MIDNIGHT_PROBE_SCROLL_MS`, `MIDNIGHT_PROBE_NATIVE_CAPTURE` |
| `midnight-viewer-ui` `examples/performance_probe.rs` | `MIDNIGHT_VIEWER_PROBE_EXPLICIT_RUNTIME`, `MIDNIGHT_VIEWER_PROBE_FOCUS_CLICK`, `MIDNIGHT_VIEWER_PROBE_MODE_INDEX`, `MIDNIGHT_VIEWER_PROBE_SIZE`, `MIDNIGHT_VIEWER_PROBE_START`, `MIDNIGHT_VIEWER_PROBE_UI_FAMILY`, `MIDNIGHT_VIEWER_PROBE_UI_SIZE`, `MIDNIGHT_VIEWER_PROBE_WHEEL_MS`, `MIDNIGHT_VIEWER_PROBE_SPATIAL_PROOF` |
| `midnight-viewer-ui` `examples/html_performance.rs` | `MIDNIGHT_HTML_PROBE_TIMELINE`, `MIDNIGHT_HTML_PROBE_RENDER_MARKERS` |
| `midnight-viewer-engine`, `midnight-viewer-render` (debug builds) | `MIDNIGHT_VIEWER_SCROLL_TRACE`, `MIDNIGHT_VIEWER_GLYPH_TRACE`: timing traces on stderr |
| `plugins/midnight-fs-archive` `tests/performance.rs` | `MIDNIGHT_ARCHIVE_PERF_RUNS` |
| `plugins/midnight-fs-archive` `tests/passwords.rs` | `MIDNIGHT_ARCHIVE_SHARE_ZIP` |
| `plugins/midnight-viewer-html` `tests/native_browser.rs`, `tests/support/runtime_linux.rs` | `MIDNIGHT_HTML_SHOT_DIR`, `MIDNIGHT_HTML_SHOT_LABEL`, `MIDNIGHT_HTML_SHOT_SCRIPTS`, `MIDNIGHT_HTML_LINUX_EVIDENCE_DIR` |
| `plugins/midnight-thumbnail-video` `tests/media_foundation.rs` | `MIDNIGHT_MF_FIXTURE_DIR` |
| `plugins/midnight-viewer-markdown` (any build) | `MIDNIGHT_MARKDOWN_TRACE`: viewport request trace on stderr |
