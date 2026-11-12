# Status fields and viewer readability

## Implemented behavior

- The filename/selection summary takes its natural width before secondary status
  fields and drive capacity, bounded by the panel width rather than 220 pixels.
  Clipping does not change the captured clipboard identity or copied value.
- Windows attributes use single letters with a tooltip legend. Archive is `A`,
  compressed is `C`; other known flags remain represented and unknown bits use `?`.
  Unix symbolic/octal permissions retain their existing representation.
- Content sniffing recognizes printable UTF-8 records separated by NUL, including
  compiler flags. Dense zero padding, invalid UTF-8 and binary control bytes still
  choose binary mode. The source bytes and manual Text/Hex choice are unchanged.
- Syntax colors retain their saturation and are adjusted against the actual
  background to reach 4.5:1 contrast. Panels and viewers share the same contrast
  calculation; the viewer calculates a small palette once per decoded page.
- Glyph font/scaler setup runs only on cache misses, avoiding repeated setup for
  cached glyphs during scrolling. Shaping, I/O and rendering remain on workers.

## Verification recorded during implementation

- Windows compact attribute tests: causal failure with the old words, then three
  passing tests including unknown flags, zero and extended attributes.
- Windows status geometry/clipboard regression passes for wide and narrowed
  panels; the full filename remains copyable. The focused UI batch passed 495
  tests (468 library, five Copy, 20 keyboard, one panel, one status).
- Windows plugin-host suite: 43 passed, five installation-dependent tests ignored.
  Trust tests require a deliberately private fixture; unset/inherited-shared
  fixture roots initially failed and were corrected without weakening production
  trust checks or modifying checkout/drive permissions.
- NUL-delimited synthetic source regression: binary before, text after, exact
  bytes preserved. Read-only verification of the user's `D:\tmp\flags.nul` also
  chose Text. The fixture tests do not copy the user's file contents.
- Linux viewer engine: 31 passed, two installed-plugin tests ignored. Strict
  all-target Clippy passed for the viewer engine, renderer and plugin host.
  Renderer tests include exact scrolling crops, Unicode, styled cached glyphs,
  selection, transparency and word wrapping.
- Keyword-color regression demonstrates removal of the old 50% foreground blend.
  Palette tests check semantic colors on light, dark and intermediate backgrounds.

## Rendering measurement

The existing `render_probe` was linked against the old and new optimized Windows
renderer using the same pinned dependencies. Three interleaved runs measured ten
scrolling frames per fixture. Median time for 4,096 short lines fell from 80.14 ms
to 61.16 ms (8.01 to 6.12 ms per frame, approximately 24% less renderer time).
The pathological 256 KiB single-line fixture stayed approximately 21.8 ms/frame;
this change does not resolve that case. Measurements are local renderer timings,
not end-to-end native input-to-display guarantees. Paging and native interaction
latency require additional qualification before claiming instant scrolling.

Build/test artifacts and raw measurements remain under `_build/goal/native-plugin`.

## Final focused UI checks

After extracting the shared contrast helper, the selected Windows UI suite passed
496 tests (468 library, five Copy, two filename-color, 20 keyboard, one status).
The corresponding Linux suite passed 492 (464 library plus the same integrations).
The filename-color test checks all nine themes and three layouts. Wide and narrow
status screenshots were inspected. Windows and Linux strict workspace Clippy,
and Windows rustfmt checks passed. An independent review found and resolved an undocumented attribute
bit mapping and a checksum-test race that sampled global I/O capacity too early.

## Release and installed viewers

The Windows release build completed with LTO. `me.exe` is 43,024,384 bytes with
SHA-256 `B44BB7F3BCAF9F3F2E17018E935253E061FA0144308B8AC8AEBB48C9AB8878C7`.
The direct build executable passed isolated native WGPU/DX12 startup (exit 0)
with its protected plugin runtime, both panels and bottom toolbar visible.
The normal user preferences file was unchanged by the smoke test.
Normal startup was also checked on the actual user's desktop: the window was
visible, responsive, not minimized or DWM-cloaked, with its expected title.
Sandbox-only process queries reported no window for the same PID; those queries
cannot establish visibility on the user's desktop.

Both installed-plugin viewer integration tests passed on Windows, exercising
basic/source/Markdown rendering, search, copy, cycling, mode changes, changed-file
refusal and the image viewer's fit/zoom/pan/cycle behavior. These are functional
integration checks, not a claim that all end-to-end scrolling latency is solved.

Raw native acceptance and viewer logs remain under `_build/goal/native-plugin`.
