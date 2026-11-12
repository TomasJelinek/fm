# Targeted test execution

Use `scripts/run-targeted-tests.py` for independent **headless Rust libtest** targets that have already been built. It does not change CI coverage, compile code, discover executables by glob, or delete build artifacts. Native dialogs, desktop input, renderer pacing and profiling are unsupported: run their dedicated tools serially, with compilers and other test processes idle.

Build the exact affected package/target set once, using the normal repository environment and feature flags. Save Cargo metadata and successful `--no-run --message-format=json` output under `_build`. Do not start concurrent Cargo processes against a shared target directory. For example, from the repository root:

```text
cargo metadata --locked --no-deps --format-version 1 > _build/qa/targeted-metadata.json
cargo test --locked -p midnight-core --test shortcuts --no-run --message-format=json > _build/qa/targeted-build.jsonl
py -3 -B scripts/run-targeted-tests.py --metadata _build/qa/targeted-metadata.json --receipt _build/qa/targeted-build.jsonl --target midnight-core test:shortcuts
```

To run two independent headless targets concurrently, select both in the build
and in the runner (no second Cargo process is needed):

```text
cargo test --locked -p midnight-core --test shortcuts --test panel_selected_navigation --no-run --message-format=json > _build/qa/targeted-build.jsonl
py -3 -B scripts/run-targeted-tests.py --metadata _build/qa/targeted-metadata.json --receipt _build/qa/targeted-build.jsonl --target midnight-core test:shortcuts --target midnight-core test:panel_selected_navigation
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
execute together on this 20-logical-CPU machine, assign `--thread-budget 6` to each
(18 libtest workers total); do not give each the full default 20-thread budget.
Coordinate the compilation first, especially when plugins share a target tree.

Multiple filters for the same artifact share one listing process and one test process. Libtest treats those filters as OR; overlapping tests execute once. Every requested filter must match at least one listed test, even if another selector requests the whole target. Ignored tests remain ignored. A missing match, an all-ignored selection, a nonzero child exit, or an absent/inconsistent libtest result is a failure. Matching uses the executable's own `--list` output, and summary counts are parsed at the start of complete libtest summary lines.

The default process limit is the smaller of four and the available CPU count. `--jobs 1` runs headless targets serially; `--jobs 2` through `--jobs 4` request bounded parallel execution. The limit is also clamped to CPU count, target count and `--thread-budget`. The total libtest thread budget defaults to available CPUs and is divided between child processes; an inherited smaller `RUST_TEST_THREADS` limit is preserved. Tests may create their own threads, so this is a libtest worker budget, not a system-wide thread guarantee. Prefer `--jobs 1` for targets that intentionally share a caller-provided fixture.

Each artifact runs from its package manifest directory, with private `TEMP`, `TMP`, `TMPDIR` and `MIDNIGHT_UI_RENDER_DIR` directories. Caller-provided installed-provider variables and `MIDNIGHT_NATIVE_TEST_ROOT` remain intact. Do not parallelize targets that mutate the same external fixture simply because their temporary directories differ.

Listing and execution each have a 300-second deadline, configurable with `--timeout SECONDS`. On a failure the runner stops admitting new processes and lets already admitted children finish within their deadlines. On interruption or an exceptional wait, it terminates only its retained child process handles and waits for them; it does not kill by process name or manage arbitrary descendants. Tests requiring independent long-lived descendants need their own cleanup and should use a dedicated harness.

Every admitted run retains separate listing/test logs, output directories and `summary.json` under a unique `_build/qa/targeted-tests` directory. `--log-dir` selects a new directory under `_build`; existing directories are refused. The summary records selected workspace, executable SHA-256, package cwd, filters, counts, process limit, thread budget, deadlines, errors and cases not run. Exit codes are 0 for success, 1 for test/execution failure, 2 for invalid inputs before execution, and 130 for interruption. Invalid receipts/selectors are rejected before any test child starts. Logs and artifacts remain available after failure.

The focused regression command is:

```text
py -3 -B -m unittest discover -s scripts/tests -p test_targeted_test_runner.py -v
```

Those tests use actual subprocesses and a socket rendezvous to prove simultaneous execution, bounded admission, output isolation and cleanup of owned processes while an unrelated process stays alive. They also cover filter coalescing, no-match refusal, receipt/path and independent-workspace validation, changed artifacts and anchored result parsing. They demonstrate runner behavior, not a Rust throughput improvement; report measured elapsed times with the actual selected suite and cache state when comparing runs.
