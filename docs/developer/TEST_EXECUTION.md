# Targeted test execution

Use `scripts/run-targeted-tests.py` for independent **headless Rust libtest** targets that have already been built. It does not change CI coverage, compile code, discover executables by glob, or delete build artifacts. Native dialogs, desktop input, renderer pacing and profiling are unsupported: run their dedicated tools serially, with compilers and other test processes idle. For example, `scripts/probe-keyboard-search.ps1` drives the release `native_keyboard_search` example with real Windows keyboard input on a desktop session (see its [verification record](../verification/2026-11-07-keyboard-native-input.md)).

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
state. Crates with at most three integration files also keep one binary per
file; CI selects some of those by name (`--test native`, `--test pdf_native`,
`--test worker`). A test that re-runs its own executable must select the child
by its module-qualified name (`<file>::<test>`, derived from `module_path!()`, run
with `--exact`).

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
prints its path. Neither mode retires anything or needs `CI=true`.

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
the probe once per key, and a warm run does not compile it at all.

Those tests use actual subprocesses and a socket rendezvous to prove simultaneous execution, bounded admission, output isolation and cleanup of owned processes while an unrelated process stays alive. They also cover filter coalescing, no-match refusal, receipt/path and independent-workspace validation, changed artifacts and anchored result parsing. They demonstrate runner behavior, not a Rust throughput improvement; report measured elapsed times with the actual selected suite and cache state when comparing runs.

### Installed archive worker qualification

The ignored real-worker tests of `midnight-filesystem-host` launch the independently
built archive worker from a [private installation root](#private-plugin-installation-root).
`test:worker` installs `ARCHIVE_WORKER` below `ARCHIVE_TEST_INSTALL_ROOT`;
`test:streaming_worker` and the library's `captured_source_tests::` use the package
installed at `ARCHIVE_CAPTURE_INSTALLATION/filesystems/archive`; `test:discovery_kinds`
creates its packages below `HOST_TEST_INSTALL_ROOT`. CI's `check` job runs all of them
with one root for all three variables. To run them locally:

1. Build and package the worker with a released worker SDK, for example
   `plugins/midnight-fs-archive/scripts/build.ps1 -SdkConfig <registry.toml> -BuildRoot _build/archive-ci -OutputDirectory _build/archive-ci/package/archive -Offline`
   (Linux: `bash plugins/midnight-fs-archive/scripts/build.sh <registry.toml> <build root> <package> debug --offline`).
   Copy the package directory to `<root>/filesystems/archive`; on Linux make its
   directories and the `midnight-fs-archive` executable `chmod 700` and its other
   files `chmod 600`.
2. Export the plugin's owned format and cancellation fixtures:
   `cargo test --config <registry.toml> --manifest-path plugins/midnight-fs-archive/Cargo.toml --test worker_fixtures --locked --offline -- --ignored`
   with `MIDNIGHT_ARCHIVE_FIXTURES` set to a directory below `_build` (and
   `CARGO_TARGET_DIR` set to the build root above to reuse its test build).
3. Write the large read-only ZIP with `py -3 -B scripts/archive-test-zip.py _build/archive-test.zip`
   (Linux: `python3 -B`). It is above the former 64 MiB source limit and never
   replaces an existing file.
4. Set `HOST_TEST_INSTALL_ROOT`, `ARCHIVE_TEST_INSTALL_ROOT` and
   `ARCHIVE_CAPTURE_INSTALLATION` to `<root>`, `ARCHIVE_WORKER` to the packaged
   executable, and `MIDNIGHT_ARCHIVE_FIXTURES` and `MIDNIGHT_ARCHIVE_TEST_ZIP` to the
   fixtures. Then run `cargo test -p midnight-filesystem-host` with `--test worker`,
   `--test streaming_worker`, `--test discovery_kinds` and `--lib captured_source_tests::`,
   each followed by `-- --ignored --test-threads=1`.

### Windows Copy/Move qualification volume

On Windows, the engine's Copy/Move qualification tests (for example
`copy_planning_tests` and the scheduler, concurrency-regression, peer-refusal and
File List output tests) need exact native lookup keys for the temporary folders they
create. The platform reports those keys only on an NTFS volume whose persistent 8.3
short-name creation is disabled ([Windows namespace pins](TRANSFERS.md#windows-namespace-pins)).
Elsewhere these tests fail; `copy_planning_tests` then says that the fixture
"requires a supported local lookup domain". The application itself is unaffected:
Copy/Move on such a volume takes the opaque lane with full pin refreshes.

The tests create their folders in `TEMP`/`TMP`. `build.bat`, the targeted runner and
CI put those below the checkout's `_build`, so there the checkout's volume decides; a
plain `cargo test` uses your profile's temporary folder, normally on `C:`. Check a
volume from an elevated prompt with `fsutil 8dot3name query D:`: it must report
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
