# File comparison implementation and verification

Ctrl+Shift+C is the configurable `files.compare` action. Two selected files in
the active pane take precedence. Otherwise the comparator uses one selected file
or the focused file from each pane, preserving left/right pane order. More than
two active selections, ambiguous other-pane selections, directories and virtual
archive entries are rejected. The UI captures immutable selection/index snapshots;
the backend expands them in displayed order and opens native identities.

Salamander's `src/plugins/filecomp/filecomp.cpp`, `ExecuteMenuItem`, was inspected
for selection behavior. No Salamander implementation or assets were copied. This
implementation deliberately reports ambiguous selection instead of opening its
editable path-selection dialog.

Text comparison uses Similar **2.7.0**, pinned with default features disabled and
only `text` enabled. The actual downloaded crate's `Cargo.toml`, `LICENSE` and
source were inspected: Apache-2.0, no mandatory fees, no enabled optional runtime
dependencies, and no upstream NOTICE file. Retain the exact Apache license in
generated distribution notices. The upstream implementation and API are described
at <https://docs.rs/similar/2.7.0/similar/>. No application license is selected by
this change.

The backend accepts UTF-8 (optional BOM) or BOM-marked UTF-16, rejects malformed
encoding and binary control characters, and retains newline distinctions. Text
inputs are limited to 4 MiB each, 100,000 lines and 16 KiB per line. A 750 ms
algorithm deadline yields a valid but potentially coarser diff for difficult
inputs. Rows are aligned by edit group with line numbers and red/green changes.
F7/Shift+F7 and the toolbar navigate differences; wheel, arrows, Page Up/Down,
Ctrl+Home/Ctrl+End and the position control navigate the synchronized view. All
keyboard actions use the configurable shortcut map, sharing viewer navigation,
close and zoom actions. Previously saved overrides take precedence over the new
defaults; two explicitly conflicting overrides still fail validation. Reading,
decoding, diffing, text shaping, rasterization and reclamation run on workers.

Image comparison reuses the existing installed restricted image decoder. Both
sources use a common scale and top-left origin, including different dimensions.
Side-by-Side places them in equal panels; Swipe reveals each side of a movable
divider; Onion Skin cross-fades both images over a checkerboard using the right
opacity slider. This follows the established three image modes discussed in
[GitLab's own tracker](https://gitlab.com/gitlab-org/gitlab/-/issues/16802).
Images support Fit, zoom and shared pan. Animated sources compare their first
still frame. The existing per-process 512 MiB memory, 128 MiB decoded RGBA,
source read and execution limits remain enforced. No image codec is added.

Two comparator workers are admitted at once. Input channels and the latest-frame
mailbox are bounded; the UI coalesces and retries pending controls. Escape and
window close signal cancellation immediately. The diff deadline bounds the
non-interruptible algorithm interval; image process supervision observes cancel.
Native OS reads retain their worker until they return. Source identity/revision
is validated around reads and frame publication; this is a live-validated read,
not an atomic two-file snapshot. The comparator never writes either source.

Windows debug acceptance passed: nine backend tests (including a real captured
Unicode source rendered through the asynchronous service), one actual Slint
keyboard/window test with a reassigned close shortcut, and two shortcut catalog
and migration tests. Tests cover changed sources, cancellation, malformed/binary
text, missing newlines, Unicode alignment, selection expansion, compositing
endpoints, alpha handling and distinct Side-by-Side/Swipe/Onion Skin pixels.
Strict Clippy passed for both comparator crates and all their targets with
`-D warnings`. Main-window presenter/shortcut integration is included in the
parent task's final Windows UI suite.

The `native_comparator` example was run with the production FemtoVG-WGPU renderer
beside an existing trusted private image decoder installation. The probe waits
for the displayed backend geometry revision and image mode before capture. Both
native windows closed through Escape. All four screenshots were inspected:

- `_build/compare-native-final-text/comparison-0.bmp`: aligned source rows,
  red/green changes, original line numbers, Czech/Chinese/emoji and missing-newline
  marker.
- `_build/compare-native-final-images/comparison-0.bmp`: different source widths
  retained at the same pixel scale.
- `_build/compare-native-final-images/comparison-1.bmp`: shared-origin Swipe
  divider crosses the centered image bounds.
- `_build/compare-native-final-images/comparison-2.bmp`: half-opacity Onion Skin
  exposes a moved rectangle, changed color and the wider source's extra edge.

The earlier `_build/compare-native-images/` captures were diagnostic and were
superseded after adding explicit frame synchronization to the probe. No release
build was used for these comparator checks.
