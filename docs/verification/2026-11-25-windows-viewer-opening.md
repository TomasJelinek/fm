# Windows viewer opening and long folder paths

## Native viewer opening

| File | Previous median | Updated median | Time saved |
| --- | ---: | ---: | ---: |
| Text, 17 bytes | 1,555 ms | 1,108 ms | 29% |
| PNG, 1920 × 1080 | 1,692 ms | 1,151 ms | 32% |
| JPEG, 1920 × 1080 | 1,721 ms | 1,199 ms | 30% |

The Windows `open_latency` probe timed the call to open a viewer through its
first accepted content frame. It used the native winit GPU renderer, installed
0.5.3 viewer plugins and a visible parent window at 2250 × 1575 physical pixels
and 225% scaling. The previous debug build created its graphics device per window
and started file capture after showing the window. The updated build shared a
device and started capture while constructing the window.

The builds ran in the order previous, updated, updated, previous. Each process
opened and closed each file three times, yielding six measurements per cell.
These are viewer-open times from a running application. Preparing the shared
device took 1.17–1.18 s before the parent appeared in the two timed runs.
A separate `--together` run opened the text fixture twice, a PNG and a JPEG.
All four native windows simultaneously remained visible with content.
The timings and renderer selections are in `_build/viewer-startup/native-verified/`
in the benchmark workspace.

## Regression coverage

The engine tests for opening an image and a PDF in Fit both passed. They cover
previously saved zoom and a settings load that arrives after opening. The viewer
UI test also verified that such a load preserves a zoom command already accepted
in the open window. Three opening and resource-retirement tests, 15 layout tests
and five viewer-opening tests passed.

Eight command-bar tests passed. The long archive-path regression changes a
hidden ancestor and compares the rendered prompt pixels, checks the frame
at both edges, and scrolls back to the start of the path. A rendered tooltip
test hovered a folder link, supplied a long Windows target and verified that
the popup exceeded the previous 560 px width while staying inside the 1000 px
window.

Windows acceptance run `20261125-141741` passed `viewers_presentation`
(13 screenshots) and `viewers_combinations` (18 screenshots). The restart check
verified the fitted dimensions of an 800 × 3000 image previously closed at 1:1.
`viewers_files` completed with 137 screenshots, eight review flags and one
Linux-only subcheck skipped. All three scenarios finished with zero failures.
