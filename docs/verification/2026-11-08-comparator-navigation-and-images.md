# Comparator navigation and retained image presentation — 2026-11-08

Text comparison now supports configurable next/previous and first/last difference
actions. Existing F7/Shift+F7 remain defaults; Alt+Down/Up, Alt+Right/Left, N/P and
Alt+Home/End follow Salamander's comparator accelerators. The reference was the
local `src/plugins/filecomp/filecomp.rc` accelerator table and
`help/hh/filecomp/shortcuts_keyboard.htm` in `D:/experiments/salamander-main`.
Alt+J/K can be assigned through Options; the existing four-alias limit remains.
Navigation stops at boundaries, aligns both panes on the target row, and preserves
rapid repeated jumps. Shortcut replacements disable the default aliases; editor
controls retain their editing keys. Ordinary scrolling and viewer search actions
retain their separate routes.

The previous image path composed a complete RGBA buffer in the backend for each
amount change, then copied/uploaded it to the renderer. A standalone extraction
of that exact composition function, compiled at the same unoptimized development
level, measured five 1600×900 samples per mode:

| CPU composition only | Observed range |
| --- | --- |
| Side-by-Side | 78.9–112.1 ms |
| Swipe | 82.0–119.9 ms |
| Onion Skin | 195.9–209.1 ms |

The extraction, executable and raw measurements are under
`_build/qa/comparator-navigation/compose-baseline.*`. These measurements isolate
the old CPU work and exclude worker sampling, UI copying and display latency.

The backend now keeps two sampled layouts: half-width Side-by-Side and full-width
overlay. The persistent restricted decoder workers retain the decoded sources.
Transparent pixels are flattened against the original checkerboard once per
layout. Swipe clips two retained textures and Onion adjusts opacity; amount
changes neither send backend commands nor compose/copy/upload full pixel buffers.
Warm mode switches reuse sampled buffers and renderer images. Resize, zoom and
pan invalidate sampling. First opening and the first use of a layout still pay
decode/sampling/upload costs. File identities are revalidated around backend
frame publication; moving a slider changes presentation of the captured images.

The layout cache retains at most three canvas equivalents of pixels: one split
pair plus two full-size overlay images. With the existing four-megapixel viewport
cap, that is at most 48 MiB of sampled source buffers and another 48 MiB of
renderer-owned image data in steady state, excluding decoder state, in-flight
frames and GPU implementation overhead. The UI uses weak source identities so
it does not become the final owner that frees backend pixel vectors.

The compact native slider labels its operation as Divider or Right opacity and
shows a percentage. The slider row keeps the canvas geometry constant across
image modes. Swipe's clipped rectangle is explicitly anchored at the left edge.
Independent review also identified the odd-width split: the right sample now
receives the extra pixel with the same scale/origin, and the renderer uses the
sampled width ratio. A 25-pixel nonuniform gradient test checks every pixel so
neither half is silently stretched. The backend test checks both viewport origins
and the checkerboard phase across the unequal boundary.

Focused Windows development checks passed:

- Core shortcut integration tests: 16 passed. The new test first failed on the
  missing alias, then passed after catalog changes.
- Comparator engine unit tests: 26 passed. The navigation regression first
  failed because Previous at the first difference wrapped to the last.
- Comparator UI tests: 6 passed, including real key presses for jumps, boundaries,
  repeated input, scrolling and shortcut replacement. Both visible hunk sides
  are asserted. The production image component is rendered through Slint's
  software renderer to check Side-by-Side, left-anchored Swipe at 0/25/50/100%,
  and Onion at 0/50/100% (one channel value of blend rounding tolerance).
- Strict Clippy for both comparator crates and all targets: passed. The native
  example was rebuilt after the final iterator/style correction.

The final native probe used `winit-femtovg-wgpu`, two generated 4096×3072 BMPs,
and a 2475×1620 native capture on this Windows desktop. It ran from the existing
trusted private decoder installation, exited successfully through Escape, visited
all three modes three times, performed 24 amount changes and four actual Slint
pointer-drag sequences. The latter moved both the Swipe divider and the native
Onion slider and asserted their resulting amounts. Final observations:

| Native measurement (includes forced redraw and GPU readback) | Observed time |
| --- | --- |
| Initial open, including process/decoder initialization | 2253 ms |
| First full-width overlay sampling | 498 ms |
| Later mode switches (six samples) | 76.6–127.8 ms |
| Swipe amount callback + snapshot (12 samples) | 23.5–34.4 ms |
| Onion amount callback + snapshot (12 samples) | 23.4–39.0 ms |
| Swipe pointer sequence + snapshot (two samples) | 28.4–28.8 ms |
| Onion pointer sequence + snapshot (two samples) | 22.6–35.9 ms |

Pointer dispatch alone took 0.45–0.77 ms; this is callback work, not measured
physical input-to-display latency. The old CPU-only baseline and these native
snapshot timings differ in workload and pixel count; no percentage speedup is
inferred. Initial decode/sampling remains observable, and these are local
development-build measurements rather than a hardware-independent latency bound.
The causal improvement is removal of full-frame composition/copy/upload on
amount input and reuse of sampled layouts on later mode switches.

Native artifacts are in `_build/qa/comparator-navigation/native-final/`:
`stderr.log` contains all timings; `comparison-0.bmp`, `comparison-1.bmp` and
`comparison-2.bmp` show split/Swipe/Onion. All modes were visually inspected;
an independent reviewer also checked the earlier equivalent `native-images/`
captures for path-label, control, alignment, divider and opacity correctness.

Logs: `_build/qa/comparator-navigation/{keys-red,engine-red,keys-green,engine-green,ui-tests}.log`.
Final additional logs: `engine-final.log`, `odd-width-final.log`,
`clippy-final.log` and `example-final.log` in the same directory.
These are focused local checks, not a claim that the entire dirty worktree or CI
has passed.
