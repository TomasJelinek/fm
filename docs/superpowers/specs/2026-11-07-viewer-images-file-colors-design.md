# Binary/text viewer, image viewer and file color rules

This extends the user's approved plugin/viewer architecture and the explicit
2026-11-07 implementation request. Existing Rust/Slint/Tokio boundaries, independent
plugin workspaces/tests, Unicode identities, security and free-dependency policy
remain mandatory. The implementation choices below are engineering decisions.

The later [panel/viewer follow-ups](2026-11-07-panel-viewer-followups.md) add
viewport-driven loading/performance, source line numbers (default on) and logical
line/column status, and GitHub/Compact/Comfortable Markdown presets. They are
accepted requirements; their implementation state is tracked separately.

## Text and binary viewing

The basic viewer retains explicit Text and Binary modes. Binary displays offset,
16 hexadecimal bytes and a printable character column, following Salamander's
hex viewer; automatic opening must recognize binary content without corrupting
UTF-8 or BOM-marked UTF-16. Alt+F3 remains the explicit basic-text override, with
Binary available in the mode chooser. Search stays cancellable and backend-owned.
Add a word-wrap toggle for Text/Source; Binary remains fixed-column. Reflow must
preserve source anchors, correct selection/search highlights, page navigation and
bounded memory for huge lines. Wrap is a renderer policy, not a parser rewrite.

## Image viewing

F3 routes supported image extensions to an installed image-viewer package; Alt+F3
continues to use basic text. The package is independently built/tested and removable
under `viewers/`. Use pinned image-rs 0.25.10 and moxcms 0.8.1, matching the thumbnail
provider's PNG/JPEG/GIF/WebP/BMP/ICO/TIFF decoders, orientation and color handling.
Show the first still frame of animated image formats in this stage.

The alternative of decoding directly in a native viewer DLL is rejected: the
thumbnail design already requires untrusted media decoding in a restricted process.
A host-owned public raster-viewer protocol carries opaque source reads, dimensions,
viewport requests and bounded sRGB RGBA output. No native filenames, filesystem
access, network or UI objects are exposed to the decoder. Reuse the existing native
worker sandbox/identity validation, with bounded memory, read volume, dimensions,
output size and response time. Cache decoded pixels within the isolated session;
zooming must not enlarge a thumbnail or reread/redecode the entire file each time.

The common viewer window retains Escape, captured-panel Space cycling and theme
integration. Add fit-to-window, actual pixels, zoom in/out, percentage display,
Ctrl+wheel zoom and panning/scrollbars. Pointer input/painting stay in the frontend;
decoding, resampling, content validation and large buffer cleanup stay in backend
workers. New public API and host integration are host-owned changes; the plugin
never patches application sources. Missing/invalid packages expose no image feature.

## Panel filters and colors

Ctrl+F12 already supports persistent per-panel masks and bounded Rust regexes.
Semicolon-separated inclusion masks and `|` exclusions are supported; directories
stay visible in those two modes. The later metadata Expression implementation
also evaluates folders; see [the current syntax](../../developer/METADATA_FILTERS.md).

Add Options > Colors with ordered enabled rules using the same mask/regex syntax,
case option and backend compiler. First matching rule wins. Set a base foreground
and optional background; keep theme defaults where unspecified. Derive focused,
selected and focused+selected variants from the theme, retaining selection and
focus cues and readable contrast. Show all four variants in a preview, without
requiring four independent user configurations. Default rules are empty so existing
folder/file theme distinctions remain. Rules target file names in all panel views;
parent navigation, progress/error colors and native identity remain unchanged.

Color swatches open a movable, resizable modal popup above Options. HSV/RGB
controls and presets scroll; Use color and Cancel stay fixed. Escape/X cancel
only the chooser. Accepting a color stages the rule; the outer Options OK/Cancel
controls persistence. See [the settings transaction](2026-11-08-options-transaction.md).

Rules have explicit count/expression bounds, strict persistent decoding and safe
late-settings merging. Invalid expressions do not replace active rules. Matching
and recompilation run on backend workers; painting consumes immutable results.
No per-row regex compilation or filesystem I/O is allowed in the UI.

## Acceptance and references

Consult Salamander `src/viewer*.cpp` for binary, wrap and find behavior and
`src/dialogs4.cpp` (`CHighlightMasksItem`, Colors configuration) for color rules.
Do not copy code/assets. Tests cover Unicode, malformed/binary input, wrapping
across pages, search direction/cancellation, zoom geometry, image corruption and
bounds, source replacement, plugin absence, rule precedence, persistence and all
theme/selection states. Update developer API, design, hotkeys and dependency docs.

## Follow-up presentation requirements

- Append Carbon Green as theme index 8, with dark charcoal surfaces and a green
  accent. Preserve all previous stored theme identities and readable contrasts.
- The drive dropdown uses a neutral menu icon, with the actual current drive
  highlighted in the stable drive buttons and checked in its menu. Its popup uses
  available window height, keeping overflow scrollable and keyboard accessible.
- Only one primary OneDrive shortcut appears on the compact toolbar. Distinct
  accounts remain named in the full menu; Shell/account registrations for the same
  directory merge by native identity while preserving the original path.
- Hot Paths appear only in dropdowns, with assigned slot numbers 1–9/0. No bare
  number buttons remain on the toolbar. The Hot Paths split button sits in the
  right-hand toolbar group beside the view-mode and sort dropdowns in each panel.
  Special-location artwork grows 10% within the existing button geometry.
- Archive display paths concatenate the native container and member names using
  the platform separator, without added spaces. Breadcrumb prefixes/history use
  the same representation; actions still carry opaque provider identities.
- Browsable archives without associated artwork use a project-authored vector
  archive icon. Generic-icon metadata is determined on the backend. Associated
  artwork, the entrance badge and filesystem-compression indication remain distinct.
  Record asset provenance separately from license assurances.
- Publish builds retain full LTO, symbol stripping, unwind recovery and maximum
  optimization for backend hot paths. Further size optimization must be measured
  against the same source and exercised with representative UI workloads before
  becoming the release default.

## Image opening and zoom correction, 11 November 2026

The initial image viewport uses the realized native canvas in physical pixels.
Rendered image tiles retain their physical dimensions while DPI or image controls
settle; a 100% image must never appear enlarged before the first resize response.
Both GPU and software presentation follow this rule. A failed viewer opening must
close its newly created native window.

Zoom requests reuse source-resolution tiles and texture identities for
magnification; the renderer scales those pixels instead of receiving new enlarged
rasters. Display zoom still changes camera surface identity and requires source
validation. Downsample filtering and PDF rasterization retain their own paths.
Transparent tiles outside the
image are generated in the backend without decoder IPC; boundary tiles use normal
sampling. Complete camera coverage, bounded memory, stale-response rejection and
source validation remain required. See the [image API](../../developer/IMAGE_VIEWER_API.md)
for the implementation contract and [verification](../../verification/2026-11-11-image-opening-zoom.md)
for the focused regression and native before/after evidence.
