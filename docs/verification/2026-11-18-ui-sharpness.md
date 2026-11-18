# UI sharpness — 18 November 2026

## Scope

Why the main window looks soft on a Linux X11 desktop at scale factor 1.0 (FemtoVG on
WGPU, the default renderer), what was changed and what remains. Source reading of the
pinned Slint 1.18.1 renderer and the vendored FemtoVG, checked against snapshots from
`me --ui-smoke-test --snapshot`. Nothing here was verified on Windows or at a
fractional scale factor.

## What the renderer aligns

FemtoVG draws text runs, image origins and `cache-rendering-hint`/opacity layers on
whole device pixels. It does not align item translations in general, bordered or
rounded rectangles (anti-aliased fill and stroke at the raw position), paths, image
sizes, or the texture it uses for a `clip: true` element with a border radius. Slint's
layouts do not round.

## Causes and status

| Cause | Visible effect | Status |
| --- | --- | --- |
| Each file pane was clipped with its 5 px radius, so the whole pane was drawn into a texture and sampled at the pane's unrounded position | Every row, icon and line of a pane that does not start on a device pixel is resampled. With a 50 % split the right pane starts on a half pixel at every odd window width | Fixed |
| The pane split was `usable * ratio`, unrounded | Pane frames and everything aligned to a pane's right edge between two pixel columns | Fixed |
| Function bar cells were `width / count`, unrounded | Vertical edges of the F-key buttons two pixels wide and dim | Fixed |
| Fractional metrics: 19.8 px picker icon, 16.5 px status line, fitted function-key font size, dialog centered in an odd remainder | Resampled picker icons, smeared borders below the status line, uneven key captions, smeared dialog frames | Fixed |
| Native icons: one 32 px bitmap per icon drawn at 14 to 20 px and at 96 px in thumbnail tiles; on a GTK desktop without `gtk-3.0/settings.ini` the lookup falls back to Adwaita's 16 px PNGs, enlarged to 32 px first; straight-alpha filtering darkens edges | Soft row, location and tile icons; folder tiles are a 6x enlargement | Open |
| Glyphs are 8-bit coverage, hinted vertically only, placed in 0.1 px steps | Vertical stems cover two grey columns; the same letter is rasterized differently within one string | Open |

### Panes

`FilePane` now clips itself with a square clip, which the renderer applies as a
scissor instead of an offscreen texture. Its rounded frame is a fill below the content
and an outline above it. The content differs from the rounded clip only inside the
four corner arcs, where the pane's 1 px layout padding already keeps content away.
This also removes one full-pane offscreen pass per pane and frame.

The split is rounded to whole device pixels (`1phx`), which keeps the stored ratio
unchanged. `commander_edges::split_panes_start_and_end_on_whole_pixels` checks both
panes at window widths 1339, 1340 and 1705 and four ratios.

### Function bar and small metrics

Function-bar cells are whole pixels and the Jobs key takes the remainder. The fitted
caption size, the status line and status height, the picker icon size and the centered
dialog position are whole pixels.

## Open: native icons

Measured by replaying the lookup and the two resampling steps on this machine:

- The Linux provider reads the icon theme only from `gtk-3.0/settings.ini`, tries
  only `<name>.png`, and without an exact `NxN` directory takes the first directory in
  manifest order. Here that is `Adwaita/16x16`, enlarged to 32 px with a triangle
  filter.
- Every `IconKey` asks for 32 px (capped at 64 px). The UI draws that bitmap at 20 px
  in rows and location buttons, 14 to 16 px for actions and 96 px in thumbnail tiles,
  with bilinear filtering and no mipmaps.
- Both steps filter straight alpha.

A complete fix requests each icon at the device-pixel size it is drawn at. That needs
a UI-to-engine command, a size-ranked theme lookup, a decision on rasterizing
`scalable/*.svg` theme icons (the provider is deliberately PNG-only today), a byte
budget for the UI icon cache and the same sizes in the Windows provider. It is not
part of this change.

## Open: text

FemtoVG rasterizes glyphs with swash in a mode that grid-fits vertically only, and
places them at tenths of a pixel horizontally. In an existing snapshot only 21 % of the
ink pixels of a path field reach 90 % coverage. The desktop itself uses LCD subpixel
rendering, so native applications look sharper beside it. Neither the software
renderer (no hinting, measurably softer) nor font choice changes this. The lever is
glyph rasterization in the vendored FemtoVG: whole-pixel pen positions make repeated
letters identical, and horizontal stem fitting or LCD rendering would make stems
sharp, at the cost of slightly uneven spacing. That is a renderer change that needs
visual sign-off and is not part of this change.

## Evidence

Snapshots are from `me --ui-smoke-test --snapshot` on this machine (X11, FemtoVG on
WGPU, in-memory default preferences), before and after the change; they are not kept
in the repository.

- At scale 1.0 and the default 1340 px width the split is already whole pixels. The
  two snapshots differ in 4,307 of 1,098,800 pixels by more than 8/255: the vertical
  edges of the function keys, which are single pixel columns after the change, and a
  few pixels at the pane corners.
- With `SLINT_SCALE_FACTOR=1.25` the pane does not start on a device pixel. Before the
  change its row text and disclosure marks are visibly resampled while the toolbar
  beside it is not; after the change the row text is as sharp as the toolbar text. The
  two snapshots differ in window size, so this is a visual comparison of the same rows,
  not a pixel difference.
- The user's reported window was 1705 px wide: with a 50 % split the right pane then
  started at x = 855.5. No snapshot of that size was taken; the geometry test covers
  it.

Row and tile icons look the same before and after, as expected.
