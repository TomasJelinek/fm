# Archive worker development

The archive worker is the independent plugin in
[`plugins/midnight-fs-archive`](../plugins/midnight-fs-archive/README.md). It reads ZIP,
7z, TAR, GZIP, XZ, Bzip2, Zstandard and compressed TAR archives for the host, read-only,
in a restricted worker process. What users see is described in
[archive formats](archive-formats.md); the plugin README describes the decoding,
streaming, memory and integrity rules in detail.

## Package and contracts

- The package is `midnight.fs.archive`, version 0.3.0, kind `filesystem-worker`, with
  the executable `midnight-fs-archive` (`midnight-fs-archive.exe` on Windows). It is
  installed in the application's `filesystems/midnight.fs.archive/` folder with its
  `plugin.toml`, `third-party.md` (the plugin's `THIRD-PARTY.md`) and the generated
  `licenses/` folder, whose Markdown notices also have lowercase names.
- Its manifest declares `protocol = 4`, `stable_entry_ids = true` and eleven formats:
  `zip`, `tar`, `gzip`, `tar-gzip`, `7z`, `xz`, `tar-xz`, `bzip2`, `tar-bzip2`, `zstd`
  and `tar-zstd`. A change to how member IDs are assigned needs a new package version.
- It speaks protocol 4 of the public filesystem API 0.5.0
  ([worker protocol](../filesystem-api/PROTOCOL.md)): protocol 2 streams verified
  members into host-owned spool files, protocol 3 cancels member reads cooperatively,
  and protocol 4 adds passwords (`Unlock`) and the reasons why a member cannot be
  read. The host also accepts installed workers of protocols 1 to 3.
- It starts through the sandbox handshake of the public worker SDK 0.2.0, takes no
  file name or command arguments, and reads the archive only through the host's
  bounded, revision-checked source broker. It never receives native or destination
  paths and never writes files.
- The host owns everything else: the extension associations
  ([`archive_format.rs`](../crates/midnight-core/src/archive_format.rs)), sessions,
  staging and the member cache (`crates/midnight-filesystem-host`), copy-out and saved
  locations. The [filesystem API reference](developer/filesystem-api.md) lists the
  limits.
- The host starts the worker only from an installation that passes the plugin trust
  checks (no links or other redirections, nothing owned by another account); one that
  other accounts can change through permissions is accepted and reported once. See
  [runtime discovery](developer/runtime-discovery.md) and
  [dynamic viewers and filesystem plugins](../SECURITY.md#dynamic-viewers-and-filesystem-plugins).
  Tests use a private installation root; never weaken the trust checks to make a test
  pass.

## Dependencies

The plugin is a Cargo workspace of its own. It depends only on the packaged public
crates `midnight-filesystem-api` and `midnight-worker-sdk` from the `midnight`
registry, never on host sources, and on pinned codec crates: `zip` (Deflate and AES),
`flate2`, `tar`, `sevenz-rust2` (AES), `lzma-rust2`, `bzip2` and `ruzstd`. It has no RAR
codec. Dependency decisions are in
[`THIRD-PARTY.md`](../plugins/midnight-fs-archive/THIRD-PARTY.md) and the
[third-party inventory](third-party.md); `scripts/notices.py` generates the package's
`licenses/` folder.

## Building

Normal builds (`build.bat`, `build.sh`) build the worker and install it beside the
host. To rebuild only the worker:

- Windows: `scripts/build-archive-filesystem.ps1 -Profile debug` (or `release`), with
  the optional `-WorkerSdkConfig`, `-AppDirectory` and `-Offline`.
- Linux: `bash scripts/build-archive-filesystem.sh [debug|release] [WORKER_SDK_CONFIG] [APP_DIRECTORY]`.

They install into `<app>/filesystems/midnight.fs.archive`, where `<app>` is by default
`_build/<profile>` (or `<CARGO_TARGET_DIR>/<profile>`). They need the public worker SDK
bundle that the viewer or thumbnailer builds prepared, or `MIDNIGHT_ARCHIVE_SDK_CONFIG`
naming its `.cargo/registry.toml`; without either they stop and say so.
`MIDNIGHT_ARCHIVE_OFFLINE=1` (or `MIDNIGHT_VIEWERS_OFFLINE=1`) builds offline.

The plugin's own scripts also build it outside the host checkout, for example from an
exported plugin folder:

- `plugins/midnight-fs-archive/scripts/build.ps1 -SdkConfig <registry.toml> -BuildRoot <build folder> [-OutputDirectory <package folder>] [-Profile debug|release] [-Test] [-Offline]`
- `bash plugins/midnight-fs-archive/scripts/build.sh <registry.toml> <build folder> [<package folder>] [debug|release] [--test] [--offline]`

The build folder (and on Linux also the package folder) must lie below a `_build`
folder. `-Test`, `--test` or `MIDNIGHT_TEST_ARCHIVE_FILESYSTEM=1` also runs the
plugin's tests. The notices need Python: `py -3` (or `python`) on Windows, `python3`
on Linux.

To produce a worker SDK bundle directly, run
`cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages --locked --offline -- <repository> <output folder>/releases`.
It prints the bundle folder; the builds take its `.cargo/registry.toml`. See the
[worker SDK guide](developer/worker-sdk.md).

## Tests

- The plugin's tests in `plugins/midnight-fs-archive/tests/` create their own fixtures.
  They cover the formats, stable member identities, streaming (the bytes requested
  from the source), passwords, timestamps, cooperative cancellation, archives with
  many members and interleaved reads. Run them with the build scripts' test switch or
  `cargo test --config <registry.toml> --manifest-path plugins/midnight-fs-archive/Cargo.toml --locked`.
- The ignored tests `measure_tar_and_sevenzip_open` and `measure_index_memory` in
  `tests/performance.rs` are manual measurements; the plugin README explains them.
- The ignored `tests/worker_fixtures.rs` exports format, cancellation and password
  fixtures into `MIDNIGHT_ARCHIVE_FIXTURES` for the host's tests.
- The host's ignored real-worker tests in `midnight-filesystem-host` (`test:worker`,
  `test:nested_worker`, `test:streaming_worker`, `test:discovery_kinds` and the
  library's `captured_source_tests::`) run the installed package; see
  [installed archive worker tests](developer/test-execution.md#installed-archive-worker-qualification).
- CI's `archive-filesystem` job checks formatting and Clippy, runs the plugin's tests
  and packages the worker on Windows and Linux from a checkout that contains only the
  plugin; its `check` job runs the host tests against that package.
  [Full validation](user-guide.md#full-validation) runs the same steps.

## Saved locations

Saved tabs and Hot Paths inside an archive store the provider ID, the decoder, the
native archive path, the captured source revision and the raw member components
separately, never a session ID or a member path joined onto the native path. Only the
active tab reopens at startup. A missing provider or a changed archive leaves a
read-only tab that says it is unavailable, with its address kept.

## Not implemented

Writing to archives, moving members, RAR and opening members in other applications
are not implemented (ARCH-03 in the [implementation backlog](implementation-backlog.md);
the other ARCH rows there list the remaining limits). Dated evidence starts with the
[archive copy-out record](verification/2026-11-07-archive-copy.md).
