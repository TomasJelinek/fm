# Native filesystem volume and storage parity

This verification covers the public native filesystem API/SDK and independent
Windows/Linux plugins. Application routing and deployment are verified separately
by the native filesystem host integration. It does not establish write/watch or
mutation support.

## Public contract

ABI minor 1 preserves the C table layouts, operations 1–14 and reply tags 1–11.
The optional `volume-details` capability (64) adds VolumeLabel (15) and
VolumeInformation (16), with reply tags 12 and 13. Existing adapters inherit
Unsupported defaults; old plugins remain compatible. Old hosts reject the unknown
manifest capability before loading a newer package. See
[the protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md) for exact wire order,
limits, raw text semantics and capability negotiation.

Returned roots and textual metadata preserve raw UTF-16/Unix bytes. Physical total,
filesystem free and caller available are independent optional values. Invalid
encodings, impossible capacity relationships, repeated properties, unknown property
tags and excessive metadata counts/lengths are rejected. Partial native failures
retain other measured fields.

Windows uses literal-path root resolution, thread-local media-dialog suppression,
GetVolumeInformationW and quota-independent GetDiskSpaceInformationW. Legacy
SpaceInfo retains its quota-aware total. Linux uses a retained O_PATH descriptor,
fstatvfs, exact mount ID/device matching and bounded mountinfo/udev metadata.
The migrated physical-storage checks only classify SSD after positive local device
proof; virtual/stacked/removable/unsupported cases remain conservative. Recognized
network storage is reported as Network.

## Causal tests and evidence

Evidence is under `_build/goal/native-volume-parity/`:

- `api-red.log`: both initial wire tests failed because optional operations and
  label reply were unsupported.
- `windows-red.log`, `linux-red.log`: three public C ABI integration tests per OS
  failed for absent capability/Unsupported replies/raw-long query failure.
- `root-address-red.log`: a native root longer than 64 KiB was rejected as malformed
  despite fitting the reply transport. The correction keeps native Address limited
  only by transport, with a dedicated codec regression.
- `api-windows-final.log`, `api-linux-green.log`: 37 workspace tests and one compile-fail documentation
  test passed, including seven volume codec/validation tests and optional adapter
  compatibility on each OS. `api-windows-clippy.log` and `api-linux-clippy.log`:
  strict all-targets checks passed.
- `windows-listing-red.log`: a captured directory entry lost its known size after
  the file was removed, proving that the old path performed a fresh per-row stat.
  Cached DirEntry metadata fixes the behavior and restores enumeration performance.
- `windows-final.log`: all 14 native tests passed, including loaded DLL rich queries,
  physical-device proof, raw/long paths, volume properties and cached enumeration.
  `windows-clippy.log`: strict all-targets checks passed.
- `linux-green.log`: all 16 native tests passed, including a loaded shared-library
  rich volume query, raw-byte paths, descriptor paths beyond 4300 bytes, FIFO/link
  refusal, independent capacity and cancellation. `linux-clippy.log`: strict
  all-targets checks passed.
- `header-linux.log`, `header-windows.log`: C11 public header size/offset assertions
  compiled with strict GCC/MSVC warnings; corresponding Rust layout tests passed.
- `linux-release.log`: the independent optimized plugin built through the
  production package helper using the existing native target directory.

The first complete Linux run accidentally used the WSL Windows-mounted fixture
directory: existing tests
`symlinks_and_fifos_are_not_read_as_content_and_pseudo_files_do_not_fake_zero_length`
and `public_table_streams_raw_names_and_captures_selected_content` failed. The preserved
`linux-drvfs-fixture-failure.log` records these failures. Repeating with the package's
documented `MIDNIGHT_NATIVE_FIXTURE_ROOT=/tmp/fm-interaction/_build/tmp/native-filesystem`
passed every test; no assertion was relaxed.

All Cargo invocations used locked/offline dependencies, disabled development/test
debug info and incremental compilation, and existing `_build/goal/native-plugin/`
targets. The Linux native runner used its existing `_build` TMPDIR. No normal app
launch with test paths or preferences was performed for this subtask.

## Artifacts and limits

The final source SDK export is
`_build/goal/native-volume-parity/filesystem-sdk-abi1.1-20261108-r2.zip`, accompanied by
a JSON manifest of all 35 public-workspace file hashes. Archive SHA-256:
`f1a56345aee041e0e5b949a079ec01ead6d3c8aa4621dd2e39b47baf933b72d6`.
The r2 bundle supersedes the initial archive with the corrected native Address
budget. Both remain immutable evidence artifacts.

The Windows optimized package is
`_build/goal/native-volume-parity/windows-package/filesystems/native.windows/`;
DLL SHA-256:
`b5fe150439bedb68be3ff4d2757bd4feffc747f7286b2e69f2ab8ac1239cef18`.
It was built using `scripts/build-native-filesystem.ps1`; evidence is in
`windows-release.log`. Feature-resolved dependency metadata is recorded in
`native-windows-metadata.json`.

The Linux optimized package is
`_build/goal/native-volume-parity/linux-package/filesystems/native.linux/`;
shared-library SHA-256:
`955d83786f0542adef8e95133e2864c8317921ce9bab9dd77d81d01fb26968d1`.

Cancellation is cooperative between synchronous OS calls. Missing labels or
unprovable physical device identity remain absent/Unknown. Native metadata is a
live observation, not a filesystem snapshot. Tests establish behavior on the
available x86-64 Windows/Linux systems and synthetic positive/negative physical
descriptors; they do not claim that every storage driver or remote mount was
available for live qualification.

No library versions changed. Additional Windows feature modules use the existing
windows-sys 0.61.2/windows-link 0.2.1 MIT alternative. Pinned license text was
inspected and the independent package's complete THIRD-PARTY notice was updated;
the shared dependency inventory is integrated by the root task.
