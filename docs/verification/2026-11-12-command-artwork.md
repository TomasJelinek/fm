# Managed Custom Command artwork — 2026-11-12

New icon/image choices create an independent 128 × 128 PNG beside preferences,
under `icons/`. The original image or resource DLL can subsequently disappear.
The source is fitted to the square without distortion and transparent padding
preserves its aspect ratio. A small original cannot gain detail through upscaling;
Windows resource extraction requests 128 pixels directly. SHA-256 content names
reuse identical encoded images. The toolbar retains the full 128-pixel image.

Windows uses `%LOCALAPPDATA%/Midnight Explorer/icons`; Linux uses
`${XDG_CONFIG_HOME:-$HOME/.config}/midnight-explorer/icons`. These are configuration
assets, not disposable thumbnail-cache files. The ordinary thumbnail cache may
cache their presentation, but clearing it does not delete the saved artwork.

## Boundaries

The UI only requests a typed picker and accepts its guarded response. It cannot
introduce an arbitrary icon path through an edit field. New choices require a
managed copy; decode, storage or cancellation failures never fall back to the
original file. Existing unchanged paths in legacy configurations remain readable
until the user reselects the artwork. Options Cancel leaves the previous command
configuration intact; an imported but unused deduplicated PNG may remain.

`command_artwork` composes the existing bounded backend picker, installed native
filesystem capture, restricted thumbnail worker and native storage service.
Binary image decoding reuses the image thumbnailer, including its orientation and
color handling. Installed provider manifests choose the worker; the source file
cannot name executable code. No image codec or shell launch is added to the UI.
The thumbnailer must be installed for image import. Missing or unsupported image
providers produce an explicit error and retain the previous draft icon.

Windows resource selection/extraction reads icons without executing the selected
module. Linux offers theme PNG/SVG files; static SVG is limited to 128 KiB of
UTF-8, 4,096 XML/expanded nodes and depth 64. DTD, external/embedded images and
fonts are disabled; marker, pattern, mask, clip-path and filter effects are
refused. Its generated PNG, or a chosen theme PNG, passes through the same image
import composition. Resource handling is selected by a platform
capability; no Windows/Linux implementation branches enter the application file.

Decoder cancellation accepts a cooperative predicate so linked engine lifetimes
reach worker/broker checks without a polling thread. Existing admitted native I/O
can still drain before cancellation completes; capacity is retained until that
call returns. Successful decoding validates the captured source revision again.

## Evidence

- Windows restricted provider pixel/cancellation test: 1 passed (1.46 seconds),
  `_build/qa/find-metadata-scroll-20261112/windows-artwork-worker.log`.
- Windows in-flight source and queued-I/O cancellation/deadline tests: 2 passed
  (4.12 seconds), `windows-artwork-worker-cancel.log` in the same directory.
- Cooperative linked-cancellation test: 1 passed, `windows-artwork-cancel.log`.
- Added application tests cover managed-only results, ordinary picker behavior,
  resource capability routing, cancellation and decode/storage failures.
- Windows native resource selection at 128 pixels and persistence after deleting
  the source module: 1 passed (0.11 seconds),
  `_build/qa/icon-picker-20261112/windows-resource128-green.log`. The fixture uses
  a self-contained resource module: copying modern Windows `shell32.dll` alone
  omits its sidecar icon resources. Native diagnostics confirmed both ordinary
  and extended-length system paths work; no production workaround was needed.
- Windows managed storage/Unicode/refusal test: 1 passed; common 128px fitting,
  alpha, deduplication, cancellation and capacity tests: 3 passed. Evidence:
  `windows-storage128-green.log` and `common-snapshot128-green.log` in that folder.
- Windows application/UI/native integration passed in the coherent 1,844-test
  affected gate; strict Clippy and the debug build passed. Linux managed artwork
  application cases passed (5 targeted cases including the presentation bridge).
  The final gate scope and startup screenshots are recorded in
  [the integration report](2026-11-12-panel-controls-and-notices.md).
