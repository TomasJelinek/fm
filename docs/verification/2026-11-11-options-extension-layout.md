# Options extension-field layout, 11 November 2026

The viewer extension editor wrapped into a fixed two-control-height TextEdit.
Its internal padding left an incomplete second line visible at common font
sizes. It now uses the standard single-line LineEdit, which follows the caret
horizontally and sizes its height from the actual text metrics. Home/End and
arrow keys expose long lists without losing any part of the extension string.
Pasted newlines become spaces; the existing backend separator parser and
validation, Reset, OK and Cancel behavior remain unchanged.

Options starts at 840×580 logical pixels instead of 740×520. Both initial exposed
geometry and DialogFrame desired dimensions use the new defaults. Existing
font scaling, minimum dimensions, available-area clamping and manual resizing
remain in place.

Targeted Windows debug/test checks passed:

| Integration suite | Passed | Execution time |
| --- | ---: | ---: |
| plugin_options | 9 | 0.37 s |
| options_reflow | 8 | 0.11 s |
| options_footer | 3 | 0.10 s |
| dialog_drag | 5 | 0.30 s |

The new extension regression retains the complete Unicode string, moves
Home → End → Home, verifies actual horizontal scrolling and checks that the
rendered caret remains inside the field horizontally and vertically. Cases use
14px and 22px desktop fonts and a 28px touch font at 1100×800, 760×650 and 1050×900.
The three software-rendered screenshots were inspected. Existing tests cover
invalid drafts, corrected multiline input, Reset/Cancel transactions, all Options
pages at 320×200, large fonts, footer actions, dragging and resizing.

Logs and rendered fixtures are under `_build/qa/options-extension-layout-20261111/`.
Full workspace suites were not rerun for this focused UI correction. No plugin
API, preferences schema or third-party dependency changed.

The fresh debug `me.exe` build passed (12.21 s with existing dependency artifacts).
Its SHA-256 is
`f1efab28020398a7a18c0dfc57932f3a9dd882c959aa0790020348889eb9d90c`.
A matching protected plugin runtime was deployed and registered. Startup smoke
with isolated QA directories exited successfully, produced `app.png` and wrote
no stderr. Rustfmt and Git whitespace checks passed; all changed handwritten
source files remain below 100,000 bytes and use LF line endings. A read-only
review found no issues in the extension-field or sizing changes.
