# PDF viewer verification

The independent `plugins/midnight-viewer-pdf` project uses the public raster SDK
0.2.0 and Hayro 0.9.0. Public API tests cover bounded page metadata, wire roundtrips,
truncation rejection, page selection and refusal of out-of-document requests.
Existing image messages and broker tests remain unchanged and passing.

Plugin tests generate valid cross-reference PDFs with independent page sizes,
colored vector rectangles and standard-font text. They verify later-page colors,
vector zoom/pan origins, white blank pages, embedded fonts, invalid source data,
out-of-range page requests, nonfinite geometry, oversized viewport and rejection
of encoded inputs above 128 MiB before reading them. No downloaded PDF fixture is
included in the repository.

Pinned licenses are inspected and included under the plugin's `notices/` tree.
Hayro's upstream notice is retained from crate commit
`3f3825c3e50bb41ed6ec6c3f4b6007978bd13def`; it was missing from the crate archive.
The alloc-stdlib BSD notice is retained from crate commit
`ae42d22078b98549e987d2f03d12df7b984fde47` for the same reason. Bundled fonts/CMaps
carry their separate permissive notices. This does not relicense the host or SDK.

Windows sandbox acceptance uses an explicitly staged private installation and
`MIDNIGHT_PDF_VIEWER_TEST_APPLICATION`; the ignored native test must be invoked
with that variable set.

Verified on 7 November 2026 using debug builds, with debug information and
incremental compilation disabled:

- Public API/SDK: 10 tests pass; formatting and strict Clippy pass.
- Independent PDF plugin: 6 tests pass on Windows and native Ubuntu 24.04 WSL;
  strict Windows Clippy passes.
- Existing image plugin: 8 tests pass after migration to public SDK 0.2.0.
- Native Windows AppContainer: both host tests pass against a new protected
  installation under `C:/Users/tjelinek/_build/PdfViewerQA-20261107`. They render
  pages 0, 1 and 0 with distinct dimensions/colors, confirm no further source
  reads, reject changed source versions permanently and reject reuse after
  cancellation. These are actual sandbox worker runs, not mocked raster output.
- Release packaging: 5 image/PDF packaging tests pass, including protocol 2
  acceptance, unsupported/bool protocol rejection, attribution and executable
  preservation. PowerShell build scripts parse successfully.
- Native Windows and Linux metadata plus Cargo-tree selections were captured
  for `pdf-viewer` and the migrated `image-viewer` inventory scopes.
- The public member versions are explicit 0.2.0. Root `cargo metadata --locked
  --offline` confirms 0.2.0, and both independent plugins pass locked offline
  all-target build checks against the final canonical bundle
  `_build/pdf-viewer/releases/worker-sdk-1794044008878358900/.cargo/registry.toml`.
  Earlier bundles remain untouched; only the two public raster package checksum
  entries changed in the independent plugin locks after the manifest correction.

Linux sandbox integration and manual visual fidelity checks remain distinct from
the native Linux plugin tests. The initial checks above used debug builds.

## Release follow-up

The image and PDF release workers were subsequently built with the final SDK
bundle using `scripts/build-image-viewer.ps1 -Profile release -Offline -Viewer`
and staged under `_build/release/viewers/`. The post-0.2 host test binary passed
both native AppContainer tests against the actual release PDF executable in a
fresh protected `C:/Users/tjelinek/_build/PdfViewerReleaseQA-20261107` installation.
The staged executable hash matched the release output. Required manifests,
executables, plugin licenses and complete third-party notices were checked.
API/plugin manifest and lockfile hashes remained unchanged.

The sequential wrapper check exposed an environment restoration issue: .NET's
string argument conversion restored an originally absent `CARGO_TARGET_DIR` as
an empty variable, causing the next Cargo call to fail. Both PowerShell viewer
build wrappers now explicitly remove originally absent variables. Sequential
cached image/PDF release builds pass with `CARGO_TARGET_DIR` initially absent,
all six saved environment variables compare equal before/after each call, and a
separate invocation preserves an explicitly supplied target directory.
