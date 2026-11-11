# Thumbnailer Settings catalog and application 0.2

The development checkout is `D:\experiments\dev\FM`, on
`codex/viewer-plugins`. The separate `codex/main-deep-audit` checkout predates
the plugin implementation and was not used as this application's build source.

## Implemented contract

Options → Plugins has Viewers, Filesystems and Thumbnailers tabs. Thumbnailer
metadata uses the existing trusted manifest discovery on a backend blocking
worker. It does not execute a decoder, read media, or touch the cache. Installed
describes validated metadata and helper presence, not a running worker. The
overview shows image/video capabilities, extensions, decoder version and helper
location; global preview options remain on Previews.

Refresh awaits three independent catalog results, including errors, and repeated
requests are coalesced. Failure clears stale rows. Selection is retained by ID.
Filesystems and Thumbnailers share the read-only Slint catalog component.
No plugin ABI, third-party dependency, license or persisted settings schema changed.

## Version ownership

All 23 application workspace packages are 0.2.0. The title/About display 0.2,
the Windows assembly manifest is 0.2.0.0, and current README/package labels agree.
Release filenames already derive the workspace version.

The version bump exposed implicit enrollment of independent API path dependencies
in the root Cargo workspace. Explicit root exclusions now preserve their nested
workspace versions. This follows [Cargo's workspace membership rules](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-members-and-exclude-fields).
Metadata confirmed exactly 23 application members under `crates/`; the root
lockfile changes only their versions and the local engine-to-thumbnail-host edge.
The version bump did not rewrite independent plugin/API manifests or their lockfiles.

## Verification scope

Evidence is retained in the audit checkout's ignored
`_build/qa/thumbnailer-settings/` directory. The initial UI regression failed
because Thumbnailers did not exist (`ui-red.log`), then passed after the fix.

- Settings UI: 29 tests across `plugin_options`, `options_footer`, `options_reflow`,
  `options_transaction` and `thumbnail_options`. Checks include minimum geometry,
  OK/Cancel/reset, catalog selection, errors, and three-result refresh completion.
- Thumbnail host: 24 ordinary tests plus five explicitly enabled trusted catalog
  fixtures/installed-provider checks; 29 distinct tests. The seven-test catalog
  rerun includes two already counted ordinary tests.
- Engine: one public integration regression exercises all three responses through
  a capacity-one event channel and drains normal startup events during shutdown.
- A fixture-based Thumbnailers screenshot was visually inspected: the list,
  capabilities, details, Refresh and dialog actions fit without overlap.
- Changed Rust files pass rustfmt; the source-size check covers 1791 files,
  each at most 100000 bytes.

An initial opt-in fixture run under the writable audit checkout was rejected by
the Windows plugin trust policy. Repeating the same tests under the existing
private runtime passed; no production ACL checks were weakened. Early engine
test attempts assumed catalog events were the only startup messages; the final
test correctly tolerates unrelated icon and availability events.

Native Linux/macOS execution and a full branch-wide test pass were not performed
for this change. Checkpoint commits preserve earlier work; they are not a claim
that every historical feature or worktree has been requalified.

## Debug artifact

The 0.2 debug build completed in 6m04s using Rust 1.99.1/MSVC, the checked-in
development optimization overrides, two build jobs, debug information disabled
and incremental compilation disabled. No release build was requested or produced.

Executable: `D:\experiments\dev\FM\_build\debug\me.exe`

Size: 94,712,832 bytes.

SHA-256: `534A46B085E3328453CE2937AB4F8E1DDE4EC5B92C5F5ACE7CFAA375C225B71E`.

The built EXE was copied with the existing compatible viewer/filesystem/thumbnail
packages to `C:\Users\tjelinek\MidnightExplorer-20261111-thumbnailer-settings`.
Deployment verified file hashes and registered that exact executable hash under
`%USERPROFILE%\.midnight-explorer\plugin-runtimes`. Existing deployments and
checkout ACLs were not changed. Backend `--smoke-test` and software-rendered
`--ui-smoke-test --snapshot` both passed from the original D-drive debug EXE,
with empty stderr. The startup snapshot was visually inspected. These smoke tests
exercise startup and panel browsing, not every decoder or interactive workflow.

Strict Clippy passed for thumbnail-host, engine and UI libraries with
`--locked --offline -- -D warnings` on the 0.2.0 workspace.

The same 59 distinct focused tests also passed on the final 0.2.0 workspace:
29 Settings UI, 29 thumbnail host (including all five opt-ins), and one engine
catalog delivery test. Final logs carry the `-0.2` suffix. Test-body times were
1.10 s across Settings targets, 0.22 s for the engine regression, 0.36 s for the
ordinary host suite and 0.08 s for the seven-test catalog rerun. Compilation is
reported separately in those logs; it is not hidden in the execution times.

## CI checksum follow-up

The archive CI jobs exposed a stale `midnight-filesystem-api` 0.3.0 checksum in
`plugins/midnight-fs-archive/Cargo.lock`. Comparing a freshly generated canonical
worker SDK bundle with the previous bundle found exactly one changed member:
`PROTOCOL.md` lost its final blank line (6800 to 6799 bytes). The other five SDK
archives were byte-identical; Git provenance, tar metadata and line endings did
not cause this mismatch. Only that first-party checksum was updated, preserving
all package versions, third-party checksums and `--locked` enforcement. Evidence
for this follow-up is under `D:/experiments/dev/FM/_build/qa/sdk-checksum/`.
The existing 0.2 application tests, executable and deployment recorded above
remain unchanged; no new application build was needed.

- A fresh bundle was generated with `cargo run --manifest-path api/Cargo.toml -p
  midnight-api-package --bin worker-packages --locked --offline --
  D:/experiments/dev/FM D:/experiments/dev/FM/_build/qa/sdk-checksum/releases`
  (`fresh-package.log`). Its configuration is
  `releases/worker-sdk-1794356045532782900/.cargo/registry.toml` beneath the
  evidence directory, referenced below as `$sdk`.
- `cargo --config $sdk fetch --manifest-path plugins/midnight-fs-archive/Cargo.toml
  --locked --offline` reproduced the checksum failure before the correction
  (`archive-red.log`) and passed afterward (`archive-fetch-green.log`).
- `cargo test --config $sdk --manifest-path plugins/midnight-fs-archive/Cargo.toml
  --all-targets --locked --offline`: **37 passed, 0 failed, 3 documented opt-ins
  ignored**, across 12 targets (`archive-tests.log`).
- `cargo clippy --config $sdk --manifest-path plugins/midnight-fs-archive/Cargo.toml
  --all-targets --locked --offline -- -D warnings` passed (`archive-clippy.log`).
- `cargo test --manifest-path api/Cargo.toml -p midnight-api-package --lib --locked
  --offline`: **4 passed**, including canonical archive determinism checks
  (`packager-tests.log`). Archive `cargo fmt --all -- --check` passed using its
  manifest (`archive-format.log`); `git diff --check` passed.
