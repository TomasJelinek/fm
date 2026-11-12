# Archive options scrolling

The Archives page was a conditional direct child of the Options `ScrollView`.
The pinned Slint compiler does not derive a Flickable's content extent from
repeated children, including conditional layouts. The page therefore rendered
past the viewport without contributing scrollable height.

An isolated software-window probe imports the actual `ArchiveOptions` component
and supplies 50 association rows. Before the fix, its 360-pixel viewport reported
360 pixels of content, and a wheel event left its offset at zero. The same page
inside an unconditional layout reported 1,931 pixels of content and scrolled to
an offset of -1,571. The probe and log are under `_build/qa/archive-scroll/`.

Archives now sits inside the existing unconditional Options content layout,
alongside the other long forms. It receives the same bottom spacing that keeps
the final control clear of the overlay scrollbar.

The full-window regression also exposed a second extent problem at font size 32:
repeated horizontal rows reported less height than their wrapped labels used at
the allocated column width. On Windows the scroll offset reached its exact lower
limit of -2,831 for 3,226 pixels of content in a 395-pixel viewport, while the last
checkbox still extended 331 pixels below that viewport. Sixteen additional
25-millisecond frames left that geometry unchanged, ruling out wheel-animation
timing. Linux reproduced the same extent problem with its font metrics.

Both association and custom-extension rows now explicitly report the maximum of
the label's preferred height, the control's preferred height and the themed
control height. This preserves wrapped descriptions and accounts for every line
in the scroll extent. An isolated probe with actual ZIP associations at font size
32 measured 3,084 pixels before this change and 3,372 afterward; the last checkbox
then ended at y=359 inside the y=20..415 viewport, with the intended bottom space.
The diagnostic sources and logs remain under `_build/qa/archive-scroll/`.

The regression tests in `crates/midnight-ui/tests/archive_options.rs` use real
wheel and pointer events. They require the last built-in checkbox to become
fully visible and respond to a click at normal, small, narrow and enlarged-font
sizes. Another case populates 32 custom extensions, scrolls to remove the last
one, returns to the Add control, and adds a Unicode compound extension. It also
adds and removes a wrapped 32-character Unicode extension at font size 32. The
footer remains outside the scrolling content. The tests retain actual wheel and
pointer input and complete-control/footer bounds assertions; they do not replace
scrolling with direct offset assignment or wall-clock sleeps.

Windows verification against the final generated UI passed all 14 focused tests:
`archive_options` (3), `shared_toolbar` (5), `comparison_toolbar` (2), and
`small_window` (4). The command was
`scripts/cargo.ps1 test -p midnight-ui --test archive_options --test shared_toolbar --test comparison_toolbar --test small_window --locked --offline -- --test-threads=1`,
using argument-array forwarding in PowerShell. Its log is
`_build/qa/archive-scroll/windows-final.log`. The enlarged-font built-in and
wrapped-custom-extension renders in the adjacent `render/` directory were also
visually checked: the final controls and the fixed OK/Cancel footer are visible.
The strict Windows UI check also passed:
`scripts/cargo.ps1 clippy -p midnight-ui --all-targets --locked --offline -- -D warnings`.
Its log is `_build/qa/archive-scroll/windows-clippy.log`.

The same 14 focused tests passed on Linux against fresh bindings; see
`_build/goal/viewer-opening-toolbar/linux/main-controls-final.log`. Its enlarged
built-in-description render also shows the final checkbox above the fixed footer.
The headless Linux custom-extension snapshot did not display the CJK name, so it
establishes Unicode data/click/removal behavior rather than CJK glyph rendering.
The Windows snapshot verifies the wrapped Unicode label visually.
