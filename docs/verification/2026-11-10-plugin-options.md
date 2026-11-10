# Plugin catalog and Options presentation

Status: source, backend, Windows plugin UI and actual installed-catalog checks
passed. The Windows debug application was built, deployed, registered and
startup-tested; see the [final artifact record](2026-11-10-settings-polish.md#exact-windows-artifact)
for its exact scope. This is not a published package release.

The Plugins page has Viewers and Filesystems subviews. Viewer priority shows
8–12 rows, with reorder arrows beside its heading and Refresh at the bottom.
Short names identify each viewer; descriptions, versions, IDs and locations stay
in details. Source viewer details identify its 17 documented lexical profiles.
Extension overrides, priority, disabled IDs and restart behavior retain the same
identities and routing contracts.

The extra `html` row was a false native-host diagnostic: native discovery did not
recognize a web-only bundle and attempted to find plugin.toml. Web discovery also
reported the real HTML provider. Native discovery now delegates pure image/web
bundles to their respective validators. Mixed native/image/web transports remain
unavailable; malformed-package reasons remain visible and distinct reasons for
the same viewer package are combined without dropping them.

Native viewer and filesystem manifests accept optional friendly names of at most
128 UTF-8 bytes, nonempty after trimming and without controls. Older manifests
remain accepted. Published short viewer names are Text and binary viewer, Source
viewer, Markdown viewer, Image viewer, PDF viewer and HTML viewer. Legacy installed
manifests receive the same readable titles without rebuilding their binaries.

Filesystems is a real backend catalog from the existing native/network and archive
hosts. It lists declared kinds, versions, locations and metadata diagnostics.
Installed means a trusted compatible manifest and regular artifact presence were
validated. It does not claim that code was loaded, a worker started, a share
connected or a provider's runtime initialization succeeded. Listing is read-only
and creates no viewer preference or filesystem activation action.

Evidence under `_build/qa/plugin-options/`:

- Six observed discovery/name RED cases, including a public image-host mixed
  transport fixture with non-executable payloads.
- Four Linux host suites: **118 passed**, seven existing real-plugin/worker opt-ins
  ignored. Core legacy/publisher naming: **2 passed**. Trusted filesystem catalog
  fixtures: **4 passed**, covering real manifest records, malformed/missing
  artifacts, duplicate identities and ambiguous native-provider claims.
- Strict Clippy for core and all four affected host crates passed with
  `--all-targets -- -D warnings`. No dependencies or Cargo.lock changed.
- Full Linux library regression after shared retention cleanup passed:
  **core 51/51; engine 661 passed, seven documented opt-ins ignored**.
  See `full-backend-linux.log`.
- The actual protected Windows installation reproduced its old technical basic
  title in `red-windows-installed-catalog.log`. The exact follow-up command script
  is `installed-catalog-windows.ps1`. Both real installed-catalog checks then passed
  under the desktop account: **one viewer test and one filesystem test**. They
  verify one HTML row, clean names/source profile details and real installed native,
  archive and network entries. Logs are `installed-viewers-windows.log` and
  `installed-filesystems-windows.log`. A sandbox owner-mismatch refusal was retained;
  source ownership checks were not relaxed to make that context pass.
- Native UI regressions cover list geometry, header actions, bottom Refresh,
  existing viewer preference transactions and read-only filesystem selection.
  `plugin-viewers` and `plugin-filesystems` snapshots use the shared
  MIDNIGHT_UI_RENDER_DIR environment. Windows `plugin_options` passed **5/5** in
  the parent's combined integration gate. Both 1340×820 BMPs were visually reviewed
  by the parent and implementer: titles, functional details, header actions,
  bottom Refresh and checkbox-free filesystem rows are readable. These UI images
  use fixture records; the actual installation is qualified by the separate
  metadata tests above. Artifacts are
  `_build/qa/settings-polish/final/plugin-viewers.bmp` and `plugin-filesystems.bmp`.

The handwritten source-size check passed for 1682 files, all at most 100000 bytes.
Developer contracts are updated in QUICK_START, VIEWER_PREFERENCES,
NATIVE_FILESYSTEM_API, FILESYSTEM_API and IMAGE_VIEWER_API. They document optional
metadata and internal `FilesystemPluginsListed` behavior without inventing runtime
filesystem enable/disable support.

The build and deployed copy share SHA-256
`E71ED919E0B0EAE14DEC5B3062213F9DA6978F287206AC849CB20FA0BCAC6E11`.
Backend and native Windows UI startup checks passed with empty stderr; the
registration, snapshot and tested paths are recorded in the
[settings qualification](2026-11-10-settings-polish.md). That record also bounds
the checks and leaves the broader implementation goal active.

Existing qualified HTML provider/runtime packages and their hash inventories were
preserved. Source manifest label changes apply to future packages; the new legacy
name mapping handles already installed metadata. No new library, feature, license
choice, paid service or mandatory fee was introduced.
