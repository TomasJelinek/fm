# About icon motion — 27 November 2026

## Scope

The About box's application icon moved in visible jumps and its edges looked jagged.
`9f3927f9` (prefiltered in-app icon images at fixed sizes) addressed the edges; this
record covers the motion, which it did not change, and the About sky's own icon
images. Measured on Windows 11 at the display's 225 % scale factor with FemtoVG on
WGPU, the application's renderer, and in the UI tests with the software renderer,
which is also the fallback when the GPU fails. Debug builds.

## Causes

| Cause | Visible effect |
| --- | --- |
| Both renderers put an image on whole device pixels: FemtoVG moves an image's origin to the pixel grid (`pixel_align_origin`, Slint 1.18.1), the software renderer rounds the image's target rectangle. The icon floats 2 px up and down every 8 s, at most 1.6 px per second at 100 % | The icon stood still, then jumped a whole device pixel: 8 jumps per float at 100 %, 18 at 225 %, and near the top of the float the software renderer jumped up and back within one frame |
| Neither renderer filters a reduction: FemtoVG's WGPU backend samples bilinearly from a texture with one mip level, the software renderer takes the nearest pixel | Before `9f3927f9` the 256 px icon reduced to 162 px (225 %) or 72 px (100 %) had uneven, stair-stepped outlines. `9f3927f9` draws a prefiltered image of the nearest size instead, enlarged or reduced by the renderer where none matches (144 px for the 162 px box at 225 %, 96 px for 90 px at 125 %) |

Frame pacing was not a cause: the `native_about` probe draws the sky at 60 frames per
second (gaps 16.4 to 16.6 ms median, 21.5 ms at most on `9f3927f9`).

## Change

`about_logo.rs` scales the chosen icon's full-size image
(`AppBranding.image-for(AppBranding.icon-index)`, 256 px) once to the 72 px box in
whole device pixels (`Math.round(72px / 1phx)`) with a Catmull-Rom filter widened by
the reduction, 16 times, each copy moved down by another sixteenth of a pixel, and
stacks the copies in one image (162 × 2,608 pixels, 1.7 MB, at 225 %). Each copy is
sampled from the 256 px image, so all are equally sharp. The sky draws one copy one to
one through `source-clip`: the float's whole device pixels move the image and its
remainder picks the copy. Both renderers then draw the same pixels without
resampling. The copies are rebuilt when the scale factor or the chosen icon changes
and reused when About opens again at the same size; per frame only the image's
position and clip change. Without copies (pixels that cannot be read, or above about
700 %, where the image would exceed 8,192 pixels) `9f3927f9`'s
`AppBranding.display-image` stands in and holds still. A still sky (less motion)
shows the first copy. The toolbar and the Options preview keep `9f3927f9`'s images.
No renderer source changed.

## Evidence

### Motion, software renderer

`about_dialog::the_application_icon_floats_smoothly_at_{100,125,225}_percent` draw six
seconds of the float, a frame every 40 ms, from 15.4 s, when no cloud or shooting star
passes the icon. The icon's position is the centre of the brightness it adds to the
sky. Wobble is how far a frame strays from the mean of the 17 frames around it once
the designed float is taken out. The same tests were run against `9f3927f9`'s sky.

| Scale | `9f3927f9`: largest step, wobble, frames that move | This change: largest step, wobble, frames that move |
| --- | --- | --- |
| 100 % | 0.998 px, 0.483 px, 18 of 150 | 0.109 px, 0.044 px, 108 of 150 |
| 125 % | 0.990 px, 0.889 px, 24 of 150 | 0.156 px, 0.086 px, 119 of 150 |
| 225 % | 0.997 px, 0.858 px, 36 of 150 | 0.233 px, 0.070 px, 136 of 150 |

The float moves at most 0.063, 0.079 and 0.141 device pixels per 40 ms frame; the
frames that do not move are those near the top of the float. The sky behind the
icon's transparent corners shifts the measured centre slowly by a few tenths of a
pixel over the six seconds, which the wobble leaves out. The tests require a wobble
under 0.2 px and no step larger than the float's per-frame movement plus 0.25 px.
The sky of `4d731e98`, before `9f3927f9`, measured the same as `9f3927f9`: largest
steps 0.996 to 0.998 px, wobble 0.48 to 0.88 px.

### Motion, FemtoVG on WGPU

The `native_about` example's window (1340 × 820 logical pixels, 225 %) captured with
`PrintWindow` about every 0.11 s while About was open, centre measured as in the tests
and fitted with the 8 s float, two rounds each:

| Build | Captures unchanged from the one before | Largest step between captures | Largest stray from the fitted float |
| --- | --- | --- | --- |
| `9f3927f9` | 41 of 51, 34 of 42 | 0.981, 0.987 px | 0.540, 0.565 px |
| This change | 2 of 32, 5 of 42 | 0.658 px (captures 0.18 s apart), 0.440 px | 0.107, 0.156 px |

On `9f3927f9` every change is a whole pixel (336.69, 337.68, 338.68, 339.66 …); after
the change the centre moves a little at every capture (337.43, 337.96, 338.48, 338.87,
339.23 …). At most the float moves 0.39 px in 0.11 s.

### Edges

Crops of the icon at 100 %, 125 % and 225 % from the tests' frames, and at 225 % from
the GPU captures, compared at 3 to 6 times enlargement. At 100 % `9f3927f9` and this
change draw the 72 px box one to one and look alike. Where `9f3927f9`'s image is
resized by the renderer, the software renderer duplicates or drops whole rows and
columns (144 to 162 px, 96 to 90 px); this change draws those sizes one to one. On
the GPU at 225 % both are smooth; the needle's outline is slightly crisper after the
change. `about_logo::tests::edges_are_shaded_by_how_much_of_a_pixel_they_cover` checks
that a hard edge reduced 3.6 times is shaded by its coverage and evenly along its
length; the other unit tests check that each copy is the first moved by its share of
a pixel within 0.02 px and that colour never exceeds opacity.
`about_dialog::the_application_icon_is_drawn_one_to_one_on_every_display` checks the
image size, clip and drawn size at 100, 110, 125, 150, 175 and 225 % and that another
icon chosen in Options is prepared;
`about_dialog::without_copies_the_prefiltered_icon_stands_in_and_holds_still` checks
the stand-in.

### Cost

`native_about` with About open, two alternating rounds with no compiler running:
`9f3927f9` 60.2 and 60.4 frames per second, process CPU 43 % and 31 % of one core;
this change 59.8 and 60.2 frames per second, 33 % and 32 % (the whole window redraws
each frame in a debug build). One round of this change had a single 68 ms gap; the
other's longest gap was 18.4 ms. The copies are built once per size and icon: the
unit test that builds them for 72, 90 and 162 px (16 steps) and 300 px (4 steps) and
checks every copy takes 0.55 to 0.66 s in the debug build.
