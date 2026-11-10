# Archive filesystem implementation checkpoint

This is work in progress, not release acceptance. The latest implementation adds
lazy ZIP indexing, XZ/BZIP2/Zstandard and compressed TAR variants, plus verified
F5 copy-out. See [the current measurements and security checks](verification/2026-11-07-archive-copy.md).
The historical checkpoints below describe earlier implementations and test counts.
The resumed generic streaming implementation uses filesystem API 0.3.0/protocol 2
and worker SDK 0.2.0, preserving protocol-1 and legacy startup compatibility.
ZIP and TAR index captured ranges; verified member content streams into host-owned
private TEMP files. There are no artificial source/member/aggregate disk byte
ceilings. Metadata, chunks, decoder windows and process memory remain bounded.
Compressed TAR still requires complete outer-stream decoding and integrity checks.
See [the resumed design and Salamander source references](superpowers/specs/2026-11-09-archive-streaming-design.md).
The current author-facing reference is [Filesystem API](developer/FILESYSTEM_API.md).

Integrated verification on 7 November 2026:

- Shared Windows UI: all 354 library tests passed, including nine archive tests
  for opaque duplicate-member selection/F3, read-only operation guards, navigation,
  unavailable provider tabs, persistent addresses, filter retention and failed sort
  rollback. This precedes the later media UI integration; the combined UI needs
  the final application verification.
- After media source integration, all 10 focused archive presenter tests passed;
  the additional test verifies supported native archive activation selects the
  provider and disables native mutations while opening.
- Backend failed archive-open regression reproduced a stale virtual capability
  marker, then passed after correction. Both controller tests pass: failed native
  opens restore native capabilities, while unavailable saved tabs remain read-only.
  Late native size requests are rejected whenever archive navigation/state exists.
- Native Linux: public API 3, independent archive fixture tests 11, actual restricted
  filesystem host tests 2 passed. Strict Clippy passed for the public API, independent
  archive project and filesystem host, including their tests. Generated Linux test
  installations set only the copied worker executable to mode 0700; copying a 0777
  DrvFS artifact initially failed the production trust check as intended.
- Canonical worker SDK bundle hashes were verified, and only the two public SDK
  checksum entries changed in the independent plugin lockfile. Codec versions and
  released viewer SDK bytes did not change. The final bundle is
  `_build/media-thumbnails/releases/worker-sdk-1794009336527797800/.cargo/registry.toml`.
- The final exported plugin passed 11 Windows fixture tests and built its release
  helper outside the host checkout. Staging is under
  `_build/archive-filesystems/stage/filesystems/archive`. Python is absent from the
  Windows PATH, so notice generation used Linux Python against the same pinned
  metadata. That exposed and fixed case-sensitive matching of lowercase upstream
  licence filenames. The complete pinned notices were generated successfully.
- The staged Windows release helper passed both real AppContainer host tests.
  Actual release staging and ZIP executable permissions were also checked across
  its 73 package files. The native installation fixture requires execution outside
  the workspace sandbox to write its existing private C: test directory.
- Release packaging regression tests: 15 passed, including both worker manifest
  layouts, hashes/notices, invalid targets/protocols/paths, corresponding FFmpeg
  source requirements and executable permissions in both ZIP and TAR.GZ.

Verified before the requested `_build` cleanup on 6 November 2026:

- Public filesystem protocol: 3 tests passed (round trips, length and framing limits).
- Independent archive decoder: 8 fixture tests passed, including Unicode,
  GNU long names, unsafe names, links, GZIP members/trailer, malformed/truncated
  inputs, expansion limits and duplicate-name solid 7z members.
- Real Windows AppContainer host integration: 2 tests passed, covering absent
  plugins and an independently built helper through installed manifest discovery,
  sandbox startup, broker source reads, member reads, source changes, cancellation
  before opening and duplicate plugin IDs.
- Host/backend compilation passed before subsequent pane and viewer integration
  edits. Pane integration, generic viewer content and subsequent edits still need
  compilation and behavioral tests. No Linux archive qualification yet.

Verified again after cleanup on 7 November 2026, with debug information and
incremental compilation disabled, in `_build/archive-filesystems/backend`:

- Filesystem protocol: 3 passed; independent archive fixtures: 11 passed.
  Additional fixtures exercise ZIP CRC failure, encrypted members and raw invalid
  Unicode names. The build uses packaged registry APIs, not host source paths.
- Core: 23 passed; common platform codec: 31 passed, including typed archive tab
  address round trips, raw member bytes and invalid/oversized locator rejection.
- Plugin host: 21 passed, 5 existing real-DLL tests ignored. Generic provider reads
  freeze length/version, enforce bounds and reject changes before/after reading.
- Real Windows AppContainer archive integration: 2 passed, including source
  brokering, changed-source rejection and absent/duplicate installed providers.
- Engine: 154 passed, 1 benchmark ignored. Viewer engine: 12 passed, 1 existing
  real-DLL fixture ignored. Archive tests cover exact virtual snapshot filtering,
  duplicate names, retained viewer leases and changed source rejection.
- Rust formatting, handwritten source size check and per-plugin license/notice
  generation passed. UI/presenter archive tests await the shared host Slint build;
  no duplicate UI target was built in this worktree. Linux remains unqualified.
- The final archive-address strict schema tests passed again after the validation
  edits. An exported standalone plugin passed all 11 fixture tests with
  `--locked --offline`, from a working directory outside the host checkout and
  using only the packaged public SDK configuration.

Archive tab addresses use preferences version 11; native-only saves keep version
10. The optional address records carry a separately versioned provider identity,
native container path, captured source revision and raw member components. They
never persist session IDs or encode members in `PathBuf`. Active tabs restore via
backend validation; inactive tabs remain lazy. Missing providers and changed
sources retain unavailable tabs instead of silently switching to native paths.

The decoder holds an immutable bounded mount in memory, with ZIP contents decoded
and CRC-checked on demand: at most
64 MiB source, 64 MiB per member, 128 MiB decoded aggregate, 100,000 entries,
8 MiB name storage and expansion ratio 200. Host process limits are 512 MiB and
15 CPU seconds, with a 20-second operation deadline and at most two live mounts.
Over-budget archives return an explicit error. This is not an unlimited streaming
implementation. Copy-out uses host-owned verified native destinations. No plugin-side
extraction, archive writing, password handling, RAR codecs or
automatic external launch of archive members exists.

Regeneration after deleting build outputs (PowerShell; adjust checkout location
and use the canonical bundle path printed by the packaging command):

```powershell
$env:CARGO_HOME = 'D:\experiments\dev\FM\.tools\cargo'
$env:RUSTUP_HOME = 'D:\experiments\dev\FM\.tools\rustup'
$env:PATH = 'D:\experiments\dev\FM\.tools\cargo\bin;' + $env:PATH
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_INCREMENTAL = '0'
$env:TEMP = 'D:\experiments\dev\FM\_build\tmp'
$env:TMP = $env:TEMP
New-Item -ItemType Directory -Force $env:TEMP | Out-Null
$env:CARGO_TARGET_DIR = 'D:\experiments\dev\FM\_build\api-tools'
cargo run --manifest-path api/Cargo.toml -p midnight-api-package --bin worker-packages --offline -- D:/experiments/dev/FM D:/experiments/dev/FM/_build/worker-sdk-releases
$sdkConfig = 'D:/experiments/dev/FM/_build/worker-sdk-releases/<emitted-bundle>/.cargo/registry.toml'
$env:CARGO_TARGET_DIR = 'D:\experiments\dev\FM\_build\archive-filesystems\backend'
cargo test --manifest-path filesystem-api/Cargo.toml --locked --offline
cargo test --manifest-path plugins/midnight-fs-archive/Cargo.toml --config $sdkConfig --locked --offline
cargo build --release --manifest-path plugins/midnight-fs-archive/Cargo.toml --config $sdkConfig --locked --offline
```

The independent plugin consumes packaged public registry artifacts, not host source
paths. The canonical bundle contains both public packages. The earlier
`filesystem-api/scripts/package.py` directory-source bundles are development
artifacts with different archive checksums and no longer match the final plugin
lockfile. Do not replace canonical artifacts with them. The existing viewer API
bundle remains separate.

The real host integration test requires `ARCHIVE_WORKER` to point to the generated
helper and `ARCHIVE_TEST_INSTALL_ROOT` to a private generated installation directory.
Use `cargo test -p midnight-filesystem-host -- --include-ignored`. The Windows test
installation used `C:\Users\tjelinek\_build\archive-filesystems\install-tests`.
Its ACL grants write only to its owner, SYSTEM and Administrators; AppContainer
packages have read/execute only. The D: checkout has shared-writable ancestors and
is correctly rejected by installation trust checks. Do not weaken those checks or
change source-tree ACLs to make the test pass.

Before release acceptance: run combined application verification after media
integration, regenerate the combined third-party inventory, and exercise final
release installation. Native provider
migration remains a separate unfinished part of the accepted architecture.

Integration prerequisites are the final `worker-api/midnight-worker-sdk` and
`crates/midnight-worker-platform` from the media worker branch. The copies used
for these isolated tests are intentionally excluded from the archive integration
commit. The combined dependency inventory must include `filesystem-api/Cargo.toml`
and `plugins/midnight-fs-archive/Cargo.toml`. The newly pinned runtime libraries are
ZIP 8.6.0 (MIT), sevenz-rust2 0.23.0 and lzma-rust2 0.21.0 (Apache-2.0); TAR 0.4.46
and flate2 1.1.10 use their MIT branches with default features disabled. No RAR or
fee-based codecs are included. Full per-plugin notices are generated by
`plugins/midnight-fs-archive/scripts/notices.py`.
