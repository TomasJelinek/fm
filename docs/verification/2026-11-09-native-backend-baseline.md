# Native filesystem backend baseline

This candidate starts from `f80c47e` and contains the independent Windows/Linux
Browse/Read API, SDK, packages and host consumer. The core additions are the
optional volume-information contract and its data types. The plugin-host addition
is the shared trusted-library boundary; its existing native content opener is
unchanged. The checked-read hook is already in the parent commit.

Application composition, UI, archive suffix policy, build/CI integration and
macOS are outside this commit. This report makes no deployment or production
cutover claim.

## Windows verification

All commands ran against the isolated candidate on 9 November 2026, with Cargo
build output under `_build/qa/native-baseline/target`, caches under `.tools/`, and
temporary fixtures under `_build/`. Cargo commands used `--locked --offline`.

| Gate | Result |
| --- | --- |
| `cargo test -p midnight-core` | 96 passed; 1 subprocess fixture ignored |
| `cargo test -p midnight-native-filesystem-host` | 42 unit tests passed |
| `cargo test -p midnight-native-filesystem-host --test native -- --ignored --test-threads=1` | Both actual installed-module tests passed |
| `cargo test -p midnight-plugin-host --lib` | 46 passed; 5 separate viewer-module fixture tests ignored |
| `cargo test --manifest-path filesystem-api/Cargo.toml --workspace` | 37 tests and 1 compile-fail doctest passed |
| `cargo test --manifest-path plugins/midnight-filesystem-native-windows/Cargo.toml` | 14 passed, including actual DLL loading |
| `python -B -m unittest scripts.tests.test_dependency_inventory -v` | 8 passed |
| Strict Clippy, all targets: the three host/core crates, public API/SDK workspace and Windows native package | Passed |
| Formatting: affected host/core crates, public API/SDK workspace and both native packages | Passed |

The host integration tests used an existing permission-protected native package.
Trust tests used the configured protected `HOST_TEST_INSTALL_ROOT`; an initial
run without that required fixture environment failed and was rerun successfully
with the configuration present. The independent Windows plugin tests built and
loaded the candidate package itself.

The already-built core, plugin-host and native-host binaries were also rerun
successfully with their complete output captured for commit review. Windows logs
are under `_build/qa/native-baseline/`: `core-tests.log`, `plugin-host-tests.log`,
`native-host-tests.log`, `native-installed-tests.log`, `api-sdk-tests.log`,
`windows-plugin-tests.log`, `host-clippy.log`, `api-sdk-clippy.log` and
`windows-plugin-clippy.log`.

## Linux verification

The same frozen candidate passed backend-only qualification under Ubuntu 24.04
on 9 November 2026. No application/UI build was invoked. All Cargo commands used
`--locked --offline`; native fixtures and the permission-protected test runtime
lived on a native Linux filesystem under `_build/`.

| Gate | Result |
| --- | --- |
| Core tests | 94 passed |
| Plugin-host tests | 34 passed; 5 separate viewer-module fixture tests ignored |
| Native-host unit and installed-module tests | 42 unit and 2 actual-module tests passed |
| Public filesystem API/SDK workspace | 38 passed, including doctest |
| Independent Linux native package | 16 passed, including actual SO loading |
| Strict Clippy, all targets: the three host/core crates, API/SDK and Linux package | Passed |

Total: 226 passed, zero failed. The audit compared all 1,812 source inputs before
and after qualification and found no changes. Their manifest SHA-256 was
`365c86016cbf5b195693a8db4ebed38333e26b41b2ed1721e4d720158ddd0330`.
The installed native SO matched the candidate-built artifact, and no workers
remained after the gate. Evidence and individual logs are under
`_build/qa/native-baseline/linux/`; `results.json` records the command, counts,
source manifest and runtime audit. This report's Linux section was added after
the frozen-source checks; implementation files remained unchanged.

These targeted backend checks do not establish whole-workspace application
qualification or production cutover.

## Dependency reconciliation

The API/SDK are `std`-only. Native packages retain complete selected MIT notices
for pinned windows-sys/windows-link and libc. The host uses already selected
serde, toml and test-only tempfile. Their exact cached manifests and license
alternatives were inspected. No third-party versions or license choices changed.

The generated inventories retain the parent's 488 package/version/source records
and 456 crate names, adding only native package scopes and direct dependants.
The generator gains the two independent package scopes. This is a reconciliation
against unchanged pinned dependencies, not a new full native Windows/Linux
inventory capture or a claim about unrelated working-tree features.
