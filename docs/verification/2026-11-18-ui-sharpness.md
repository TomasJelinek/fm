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
| Native icons: one 32 px bitmap per icon drawn at 14 to 20 px and at 96 px in thumbnail tiles; on a GTK desktop without `gtk-3.0/settings.ini` the lookup falls back to Adwaita's 16 px PNGs, enlarged to 32 px first; straight-alpha filtering darkens edges | Soft row, location and tile icons; folder tiles are a 6x enlargement | Fixed on Linux for rows, tiles and locations; Windows and action icons unchanged |
| Glyphs are 8-bit coverage, hinted vertically only, placed in 0.1 px steps | Vertical stems cover two grey columns; the same letter is rasterized differently within one string | Fitted to whole pixel columns since 19 November; the earlier rendering remains selectable |

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

## Native icons

Measured before the change by replaying the lookup and the two resampling steps on
this machine:

- The Linux provider read the icon theme only from `gtk-3.0/settings.ini`, which GNOME
  does not write, tried only `<name>.png`, and without an exact `NxN` directory took
  the first directory in manifest order. Here that was `Adwaita/16x16`, enlarged to
  32 px with a triangle filter.
- Every `IconKey` asked for 32 px (capped at 64 px). The UI drew that bitmap at 20 px
  in rows and location buttons, 14 to 16 px for actions and 96 px in thumbnail tiles,
  with bilinear filtering and no mipmaps.
- Both steps filtered straight alpha.

What changed:

- **The UI reports where it draws icons.** `midnight_ui::icon_view` reads the row,
  tile and location boxes from the window (theme sizes times the scale factor, and
  which pane shows tiles) in device pixels, and the explorer's pulse sends them to the
  backend as `Command::SetIconEdges` whenever density, fonts, view modes, the window's
  size or the display scale change them. A refused send is retried on the next pass.
- **The backend delivers each icon at those edges.** Its icon worker treats a request
  as naming an icon, loads it at every edge its kind is drawn at (`IconEdges::of`),
  remembers which edges each of its 256 icons has, and after an edge change loads only
  what is missing, most recently used icon first and after any new request.
  `IconService::icon_edge` lets a provider with one size keep it: the default answers
  32 for every edge, so the Windows provider is asked exactly as before.
- **The UI keeps several bitmaps per icon** and gives each place the one of its edge,
  else the next larger, else the largest. Bitmaps are uploaded premultiplied. The cache
  holds the backend's 256 icons; with tiles of 96, 192 or 256 device pixels that is
  about 10, 38 or 66 MiB when 256 file types have been listed, and under 1 MiB with
  rows only.
- **The Linux provider** asks the XSETTINGS manager for the desktop's icon theme
  (`Net/IconThemeName`, cached for 5 s, waiting at most 250 ms for the display) before
  `gtk-3.0/settings.ini`; searches the themes it inherits depth first before the
  Adwaita, breeze and hicolor fallbacks; ranks a theme's directories by pixel size
  (`Size` times `Scale`): exact, then the nearest larger, then the nearest smaller;
  returns artwork of the exact size untouched and scales anything else once, in
  premultiplied alpha. It still reads only `<name>.png` under the fixed system roots.

On this machine the desktop theme is Yaru, which ships PNGs of 16, 24, 32, 48, 64, 96,
256 and 512 px:

| Place | Drawn at | Before | Now |
| --- | --- | --- | --- |
| Rows, location buttons | 20 px | Adwaita 16 px, enlarged to 32, reduced to 20 | Yaru 24 px reduced once to 20 |
| Thumbnail tiles | 96 px | Adwaita 16 px, enlarged to 32, enlarged to 96 | Yaru 96 px, untouched |
| Toolbar and function-key actions | 14 to 32 px | the built-in vector icon, or Adwaita 16 px enlarged to 32 | the built-in vector icon, or Yaru 32 px |

Consequences to know:

- **The icons change, not only their sharpness.** They are now the desktop's own:
  Yaru's folders and file types instead of Adwaita's blue ones. Yaru also has PNGs for
  20 of the 36 actions (new file, find, delete, the tab close button and others),
  where Adwaita had three, so more toolbar buttons show theme artwork beside the
  built-in vector icons, as they already do with Windows stock icons.
- **20 px is not a size themes ship.** Row and location icons are therefore still
  resampled once, from 24 px. Only 16 and 24 logical pixels would be untouched at scale
  1.0; changing the row icon box is a layout decision that was not taken here.
- **Action icons are one bitmap** drawn at 14, 16, 18, 20, 24 and 32 px in different
  places, so they stay at 32 px and are scaled by the renderer.
- **Not covered:** the Windows provider (unchanged, still one 32 px bitmap); themes
  that ship only SVG, such as Breeze and current Adwaita, where lookup is deliberately
  PNG-only and falls back as before; desktops without an X display or XSETTINGS
  manager, which keep the settings-file lookup; executable icons of user commands,
  which are requested at 64 px; tiles larger than 256 device pixels, which get the
  256 px bitmap.

## Text fitting

FemtoVG rasterizes glyphs with swash in a mode that grid-fits vertically only, and
places them at tenths of a pixel horizontally, so a stem about 1.1 px wide covers two
gray columns at a phase that differs from letter to letter. The desktop itself uses
LCD subpixel rendering, so native applications look sharper beside it. Neither the
software renderer (no hinting, measurably softer) nor the choice of font changes this.

The vendored renderer now has two other fittings ([vendor notes](../../vendor/README.md)).
Since 19 November 2026 the explorer uses `fitted`. That was the user's decision
after comparing the three on this machine; until then both were opt-in.
`MIDNIGHT_GLYPH_FITTING=subpixel` or `=snapped` in the environment selects another
one for a run, to compare them or to rule the fitting out as a cause:

- **subpixel**: the renderer's own placement, as before this change.

- **snapped**: one bitmap per glyph, on a whole pixel, with a mild contrast curve.
- **fitted**: also fits outlines horizontally, through the hinting engine swash
  itself uses: the font's own program where that puts stems on whole pixels (DejaVu
  Sans), the automatic hinter elsewhere. Glyphs inside a word follow each other at
  their fitted, whole-pixel advances, move by a pixel where they would stray from
  the layout or reach into the next glyph's place, and the pen returns to the
  layout after every blank, which keeps at least its layout width less half a
  pixel.

Measured on snapshots of the main window on this machine (DejaVu Sans 12 px, light
text on dark rows; "ink" is a pixel with at least 10 % coverage):

| Fitting | Names: ink pixels | of them at least 90 % covered | Dates and times: ink pixels | of them at least 90 % covered |
| --- | ---: | ---: | ---: | ---: |
| subpixel | 2,221 | 20.8 % | 5,665 | 24.8 % |
| snapped | 1,976 | 41.4 % | 4,949 | 42.8 % |
| fitted | 1,884 | 38.6 % | 4,603 | 37.8 % |

The share understates `fitted`: its stems are single full columns with nothing beside
them, which is why it has the fewest ink pixels, while `snapped` reaches its share by
raising the contrast of two-column stems. A standalone comparison with the same
libraries measured the stem of `l` and `i` at two columns for `subpixel` and for
`snapped` and at one for the font's own program; it also showed that fitted outlines
on unrounded linear positions open visible gaps ("Nam e"), which the word-wise pen
removes.

**Blanks between words (20 November 2026).** The word-wise pen first only had to stay
within a pixel of the layout either way. Fitted advances at small sizes are often wider
than linear ones, so words grew into the blank after them, and the About dialog's
license details showed 11 px text such as "copyright notice that is" with 1 to 2 px
spaces. A glyph now may not lead its layout by more than half a pixel nor end more than
half a pixel past where layout starts the next glyph, and a word after a blank keeps that
blank at least its layout width less half a pixel ([vendor notes](../../vendor/README.md)).
Measured with the vendored rasterizer on nine About dialog lines, each laid out from ten
tenth-pixel starts (620 blanks per face and size; "short" is the layout width minus the
drawn width):

| Face and size | Worst blank, before | Worst blank, after | Blanks over 0.5 px short, before → after |
| --- | ---: | ---: | ---: |
| DejaVu Sans 11 px | 2.50 px short | 0.50 px short | 86 → 0 |
| DejaVu Sans 12 px | 1.81 px short | 0.19 px wide | 406 → 0 |
| DejaVu Sans 13 / 14 px | 1.13 / 1.45 px short | 0.13 / 0.45 px short | 27 → 0 |
| DejaVu Sans Bold 20 / 26 px | 0.96 / 1.05 px short | 0.04 px wide / 0.05 px short | 479 → 0 |
| Noto Sans 11 / 12 / 13 px | 1.86 / 2.12 / 2.38 px short | 0.14 px wide / 0.12 / 0.38 px short | 710 → 0 |

The narrowest blank between the ink of two words grew from 2 to 3 px (DejaVu Sans
11 px), from 2 to 4 px (12 px) and from 0 to 1 px (Noto Sans 12 and 13 px). Letters move
closer instead: neighboring glyph pairs whose bitmaps overlap went from 962 to 1,085 of
27,450, most of the added ones in Noto Sans at 11 and 12 px. This is a rasterizer
measurement with linear layout and no kerning, not a snapshot of the running
application; an ignored regression test in the vendored crate repeats it for DejaVu
Sans.

Known limits of `fitted`, accepted with that decision and not yet exercised in the
running application beyond the snapshots:

- Layout, carets, selections and elision still use linear advances, so ink can be a
  pixel, and at the start of a word after a blank about a pixel and a half, from where
  layout placed it. The recolored quick-search prefix in file
  names is clipped at layout positions and can cut a pixel column of the next letter.
- A word that crosses the left edge of a clip can move by a pixel while its text
  scrolls horizontally, as in a scrolled line edit.
- Fonts whose program does not fit horizontally (Ubuntu, Lato, Liberation Sans) get
  the automatic hinter with the contrast curve: crisper than `subpixel`, but their
  stems keep a second, faint column.
- Viewer and comparator content is rasterized by their own engine and is unchanged;
  their toolbars follow the process setting.
- Verified at scale 1.0 only. At a fractional scale factor hinted outlines use the
  rounded size while layout uses the fractional one, as they already do today.

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

Row and tile icons looked the same before and after the alignment change, as
expected. After the icon change, snapshots of the same window show Yaru's artwork in
rows, location buttons and, in the Thumbnails view of the idle-wakeup probe
(`MIDNIGHT_IDLE_WAKEUP_PROBE=8:2:thumbnails`, captured with `xwd`), 96 px folder tiles
with single-pixel outlines. The three text fittings were captured while `fitted` was
still opt-in, with `MIDNIGHT_GLYPH_FITTING` unset (then `subpixel`), `snapped` and
`fitted`. After the default changed, a snapshot taken without the variable is
identical, pixel for pixel, to one taken with `fitted` in the same session, and so is
one taken with an unknown value; the `subpixel` and `snapped` snapshots each differ
from it in the text. Those five were 320 by 200 pixels, because that session's smoke
test captured before the window had its size; the earlier ones were 1340 by 820.

The compared images are in `_build/verification/2026-11-18-ui-sharpness/` on the
machine that produced them, not in the repository.

## Tests of the icon change and the text fitting

Docker build image, Linux, debug profile, headless software renderer, on the final
code based on main `544b48db`. `cargo test --locked --no-fail-fast` for the listed
packages with `--lib --bins --tests` (examples were not built), and the vendored
package with the command in the vendor notes:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-core` library | 99 | 0 | 0 |
| `midnight-core` integration | 136 | 1 | 2 |
| `midnight-engine` library | 1,671 | 20 | 16 |
| `midnight-engine` integration | 417 | 0 | 7 |
| `midnight-platform-linux` library | 308 | 37 | 33 |
| `midnight-platform-linux` integration | 39 | 0 | 0 |
| `midnight-ui` library | 1,220 | 10 | 2 |
| `midnight-ui` integration | 761 | 4 | 1 |
| `midnight-ui-input` library and `placement` | 32 | 0 | 0 |
| `midnight-viewer-ui` library | 167 | 3 | 0 |
| `midnight-viewer-ui` integration | 25 | 2 | 0 |
| `midnight-explorer` | 123 | 0 | 17 |
| `midnight-compare-ui` integration | 15 | 0 | 0 |
| vendored `femtovg`, `text::fitting`, with `swash` and the two font tests | 10 | 0 | 0 |
| vendored `femtovg`, `text::fitting`, without `swash` | 7 | 0 | 0 |

No failure is in code this change touches:

- The 19 failures of `midnight-ui` and `midnight-viewer-ui` are the same 19 tests that
  fail on main `544b48db` in this environment
  ([list](2026-11-18-slint-compile-time.md)).
- `midnight-engine` had no baseline: neither of its test targets compiled on main
  `544b48db`. With local fixture repairs, which main's merge repair has since
  replaced, 19 of its 20
  library failures are in `copy_operations` (eight stop with "requires a supported
  local lookup domain", the others are timeouts and assertions of the copy scheduler
  and its peer-refusal tests) and one, in `size_warning_requests`, asserts a worker
  order. All 83 tests under `desktop::`, which hold the icon worker, pass.
- 33 of the 37 `midnight-platform-linux` failures stop with "fixture requires
  qualified local byte lookup"; the other four are assertions in
  `transfer::namespace`. All 37 are in the transfer modules.
- The `midnight-core` failure is its guard `every_test_file_is_a_module`:
  `tests/file_comparators.rs` is not declared on main.

Those failures were not investigated. The engine tests with a sized provider cover
delivery at each drawn edge, reloads after an edge change, a request overtaking a
reload batch, edges that change while loads are queued and a single-size provider.
The Linux tests look icons up in temporary theme trees (inheritance order, size
ranking, the probe budget, links out of the roots) and parse XSETTINGS blobs. The UI
tests cover the cache, the edges read from the window, and a left pane of rows beside
a right pane of tiles whose parent icons are 20 and 96 px bitmaps in boxes of exactly
that size.

### After merging main `2e6ee4ff`

Main gained the merge repair ([active work](../ACTIVE_WORK.md)) while this change was
made. Merged with it, the same commands give:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-core` library | 99 | 0 | 0 |
| `midnight-core` integration | 151 | 0 | 2 |
| `midnight-engine` library | 1,670 | 19 | 16 |
| `midnight-engine` integration | 417 | 0 | 7 |
| `midnight-platform-linux` library | 308 | 37 | 33 |
| `midnight-platform-linux` integration | 39 | 0 | 0 |
| `midnight-ui` library | 1,232 | 0 | 2 |
| `midnight-ui` integration | 765 | 0 | 1 |
| `midnight-ui-input` library and `placement` | 32 | 0 | 0 |
| `midnight-viewer-ui` library | 170 | 0 | 0 |
| `midnight-viewer-ui` integration | 27 | 0 | 0 |
| `midnight-compare-ui` library | 25 | 0 | 0 |
| `midnight-compare-ui` integration | 15 | 0 | 0 |
| `midnight-explorer` | 123 | 0 | 17 |

The remaining failures are the same 19 `copy_operations` tests of the engine and 37
transfer tests of the Linux provider as before the merge. Main `2e6ee4ff` alone was
not run in this environment, so that they fail there too follows from their messages
and their modules, not from a baseline run. The vendored renderer is untouched by the
merge. The application builds, and `cargo clippy --locked --lib --bins --tests -- -D
clippy::all` passes for the five packages named below.

Not verified: Windows (nothing was built or run there); a Wayland session; a
fractional scale factor; any font other than DejaVu Sans in the application itself;
`fitted` in windows with selections, carets or scrolled line edits beyond reading the
code.

`cargo clippy --locked --lib --bins --tests -- -D clippy::all` passes for
`midnight-core`, `midnight-engine`, `midnight-platform-linux`, `midnight-ui` and
`midnight-explorer`; Clippy on the vendored library reports nothing in the files this
change touches; `rustfmt --check` passes for every changed Rust file. The full
workspace gates were not run.
