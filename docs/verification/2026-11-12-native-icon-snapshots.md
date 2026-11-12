# Native command icon snapshots — 2026-11-12

This note records native resource/storage qualification. The integrated request routing,
restricted image decoder and legacy-path policy are covered by
[Managed Custom Command artwork](2026-11-12-command-artwork.md).

## Windows resource fixture diagnosis

The first 128-pixel regression copied `System32/shell32.dll` into an owned Unicode
fixture directory. Extraction failed there because this Windows installation stores
its icons in `SystemResources/shell32.dll.mun`. Native diagnostics found 335 icons in
the system module, zero in the isolated DLL copy, and 335 in a copy of the resource
sidecar. Both ordinary and extended-length (`\\?\`) paths worked for indices 0 and 1
at 64 and 128 pixels when the resources were present. The isolated DLL copy returned
`S_FALSE` without an icon handle for every corresponding case.

The regression now copies the self-contained sidecar to `resource 文档.dll`, or uses
the embedded system DLL resources on Windows versions without that sidecar. Production
extraction was unchanged. This correction preserves the real selected-index, Unicode,
128-pixel and source-removal checks; it does not replace them with a synthetic icon.
The diagnostic data are in
`_build/qa/icon-picker-20261112/resource-native-diagnostic.json`.

## Final targeted checks

All commands used the pinned Windows toolchain, locked/offline dependencies, the existing
SDK configuration and the shared `_build` target after acquiring the Windows Cargo slot.

| Package / filter | Result | Test time | Evidence under `_build/qa/icon-picker-20261112/` |
| --- | --- | --- | --- |
| `midnight-platform-windows` / `icon_picker::tests::selected_resource_index_is_rasterized_at128_and_snapshot_survives_source_removal` | 1 passed | 0.11 s | `windows-resource128-green.log` |
| `midnight-platform-windows` / `icon_storage::tests::managed_snapshot_is_native_unicode_and_cannot_replace_a_directory` | 1 passed | 0.01 s | `windows-storage128-green.log` |
| `midnight-platform-common` / `icon_snapshot::` | 3 passed | 0.19 s | `common-snapshot128-green.log` |

The native resource test captures two actual resource indices and verifies their pixels
differ, checks a 128 × 128 RGBA result, saves the managed PNG, deletes the selected source
module, and verifies the saved bytes and 128 × 128 PNG dimensions remain unchanged.
It also checks that an already-cancelled capture returns interruption. The storage test
uses a Unicode application directory and proves a directory at the content-addressed
filename is refused and retained.

The common tests decode the encoded PNG to verify a small source is fitted across the
128-pixel canvas, with source alpha and transparent aspect-ratio padding. Identical
snapshots reuse one managed file; altered existing content is refused without overwrite.
Cancelled or oversized input cannot publish, and an existing 1,024-entry directory
refuses an additional snapshot.

These automated checks exercise native extraction and publication. They do not claim
an interactive `PickIconDlg` acceptance/cancellation run. Combined Windows/Linux
application gates and broader qualification are recorded by the integration owner.

## Linux qualification

The final coordinated Linux native checks also pass on the frozen snapshot source:
`midnight-platform-common` / `icon_snapshot::` has 3 passes in 0.06 seconds, and
`midnight-platform-linux` / `icon_` has 6 passes in 0.12 seconds. These cover the
common publication contract, bounded static SVG handling, and native private
storage/refusal behavior. Logs are `common-icons.log` and `linux-icons.log` under
`_build/qa/find-actions-20261112/linux-final/`; `post-build.log` records the shared
build. These are backend tests, not an interactive Linux portal-selection claim.

## Dependency evidence

Fresh Windows and Linux application metadata/tree captures regenerated the dependency
inventory, and its check passed: 625 pinned packages, 512 names and 64 directly declared
packages. The direct `resvg` edge reuses an already pinned version. The common `image`
edge enables PNG encoding/resizing; arbitrary binary image import remains in the
restricted thumbnail worker. Seven original license notices for the four relevant
pinned packages were copied and SHA-verified in the QA notice staging directory.
Evidence: `inventory-generate.log`, `inventory-check.log`, `notices-verified.json` and
`notices/` under `_build/qa/icon-picker-20261112/`; policy and feature details are in
[THIRD_PARTY.md](../THIRD_PARTY.md).
