# Viewers, comparison, thumbnails and archives: acceptance coverage

This record lists every user-visible feature of the viewers (F3, Alt+F3), the file
comparator (Ctrl+Shift+C), directory comparison (Ctrl+F10), the Thumbnails view (Alt+5)
and archive browsing (the inventory), the acceptance scenario and check that exercise
it through the real windows, and the status of that check in the latest recorded run.
The inventory follows the code (`crates/midnight-viewer-ui`, `midnight-viewer-engine`,
`midnight-compare-ui`, `midnight-compare-engine`, `midnight-thumbnail-host`,
`midnight-web-viewer-host`, `plugins/midnight-viewer-*`, `plugins/midnight-fs-archive`)
and the documentation ([KEYBOARD_SHORTCUTS](../../KEYBOARD_SHORTCUTS.md),
[VIEWER_PRESENTATION](../VIEWER_PRESENTATION.md), [VIEWER_PREFERENCES](../VIEWER_PREFERENCES.md),
[VIEWER_SCROLLING](../VIEWER_SCROLLING.md), [SCROLLING](../SCROLLING.md),
[FILE_COMPARISON](../FILE_COMPARISON.md), [COMPARE_DIRECTORIES](../COMPARE_DIRECTORIES.md),
[ARCHIVE_FORMATS](../../ARCHIVE_FORMATS.md)). The suite itself is described in
[ACCEPTANCE.md](../ACCEPTANCE.md).

Statuses:

- **PASS**: the check ran and held.
- **FAIL**: the check ran and found a product bug (named by its finding).
- **PENDING**: the check waits for announced behavior, for the reason given.
- **SKIP**: the acceptance environment cannot provide what the check needs.

Run these scenarios with `bash scripts/acceptance.sh --only viewers,tools_compare_dirs`.
Every `viewers_*` scenario runs in a staged child process with the installed plugin
packages (`MIDNIGHT_ACCEPTANCE_PLUGINS`, see ACCEPTANCE.md). Their helpers are
`tests/acceptance/viewers_support.rs` (composition, opening and closing windows,
`Failures` with pending and skipped checks), `viewers_probe.rs` (scrollbar drags,
wheel notches, settling, the byte range of a text page, how far rendered content moved,
text colours and WCAG contrast, the layout review of content windows) and
`viewers_fixtures.rs` (content the scenarios generate in their own work folders).

Latest recorded runs (debug test build, software rendering, plugins from a current
complete plugin set): `viewers_files`, `viewers_compare`, `viewers_thumbnails`,
`viewers_archives`, `tools_compare_dirs` and `viewers_combinations` in one run;
`viewers_scroll` alone; `viewers_selection`, `viewers_presentation` and
`viewers_combinations` (with small windows) in a third run. Items marked "built, not
run" were added after the runs: they compile but have no result yet.

Checked in source on 21 November, without a new acceptance run:

- The Source viewer's Lines check box lies inside 480×320 and 320×200 windows, at both
  densities and fonts of 12 to 32 px (`midnight-viewer-ui` test
  `ui_polish::source_toolbar_controls_stay_inside_small_windows_and_take_clicks`).
  `viewers_combinations` › compact reports it outside every window, 1000×720 included,
  because its visibility probe drops anything under an opacity-0 layer, and a
  `WrappingCheckBox` keeps its accessible native check box in such a layer over the
  drawn box (Word wrap too). The probe needs fixing, not the toolbar.
- With Find open, a short window pushed the viewer's footer outside it (320×200 with
  touch spacing, 560×200 with 32 px fonts). Since 21 November the rows above the
  text give way to the Find bar there, and the Find bar's Close find button (or Aa in
  Text mode) brings them back
  (`ui_polish::rows_above_the_text_give_way_to_find_in_short_windows_and_come_back`).
- S4, S5 and X8 are refuted: the ink the renderer draws for selected text keeps at
  least 4.50:1 against the selection for every theme (26) and syntax role (12, CSV
  columns included), 312 checks (`selection_contrast::selected_text_of_every_syntax_role_reaches_aa_contrast_in_every_theme`).
  The low readings were antialiased edge pixels, blends of that ink with the
  selection: the probe should take the highest-contrast pixel of large glyphs.

## Opening and routing (F3, Alt+F3)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| O1 | F3 opens the applicable installed viewer for every fixture file kind (40 files) | `viewers_files` › each fixture case | PASS |
| O2 | Alt+F3 forces the plain-text viewer (Markdown source shown, not rendered) | `viewers_controls` › Alt+F3 plain text; `viewers_scroll` › CSV as plain text | PASS |
| O3 | Every extension the Source viewer claims (58, including `makefile`/`gnumakefile` whole names) opens in Source mode with syntax colours | `viewers_files` › generated Source extensions | PASS |
| O4 | CSV and TSV show rainbow columns (text colour and four column hues) | `viewers_files` › `table.csv`, `table.tsv` | PASS |
| O5 | Markdown extensions `md`, `markdown`, `mdown`, `mkd` render | `viewers_files` › Markdown extensions | PASS |
| O6 | HTML extensions `html`, `htm`, `xhtml` open in the HTML viewer (Chromium) with local stylesheet and image | `viewers_files` › `docs/page.html`, `plain.htm`, `page.xhtml` | PASS |
| O7 | Image extensions PNG, JPEG (`.jpg`, `.jpeg`), GIF, WebP, SVG, BMP, ICO, TIFF (`.tif`, `.tiff`) show their pixels | `viewers_files` › image cases | PASS |
| O8 | AVIF images | none | SKIP: no AVIF encoder in the suite or on this machine to make a fixture |
| O9 | PDF opens with its page counter | `viewers_files` › `docs/report.pdf` | PASS |
| O10 | Binary files open in the hex view; one printable byte is text | `viewers_files` › `sizes/*` | PASS |
| O11 | Empty text and binary files open without content or error | `viewers_files` › `text/empty.txt`, `sizes/zero.bin` | PASS |
| O12 | Encodings: UTF-8, UTF-8 BOM (hidden), UTF-16 LE/BE with BOM, CRLF (no CR shown), no final newline | `viewers_files` › text cases | PASS |
| O13 | Invalid UTF-8 and odd UTF-16 bytes are made visible (`\xFF`, `\uD800`) | `viewers_files` › `invalid-utf8.txt`, `odd-utf16.txt` | PASS |
| O14 | NUL bytes are never shown raw (hex view or `\x00`) | `viewers_files` › `nul.txt` | PASS |
| O15 | Damaged images: truncated PNG, GIF, WebP, BMP, TIFF, zero-byte PNG, broken SVG are refused with "This image is damaged or incomplete" and the decoder's reason | `viewers_files` › damaged cases | PASS |
| O16 | Damaged PDFs: garbage after the signature and empty files are refused; a truncated PDF or JPEG shows what can be decoded or is refused, never an error or hang | `viewers_files` › damaged cases | PASS |
| O17 | Broken Markdown, HTML and JSON show as far as they go | `viewers_files` › `broken.*` | PASS |
| O18 | Open latency recorded per file (flagged over 500 ms, failing over 10 s) and its spread over 15 repeated opens | `viewers_files` › latency notes, repeated opens | PASS (slow-open flags only: first open and HTML 0.8–1.3 s) |
| O19 | Every viewer kind's window passes the UI review's layout checks | `viewers_files` › layout review; `viewers_scroll` › layout review | PASS |
| O20 | The HTML viewer's AppArmor Install… offer when the sandbox is refused | none | NOT YET WRITTEN: needs a stage outside the AppArmor profile's `_build/*/viewers/...` pattern (a `viewers_stage` change) |

## Viewer controls

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| C1 | Status bar (bytes, line and column) | `viewers_controls` › status bar | PASS |
| C2 | Find: Ctrl+F, Enter, F3, Shift+F3, "No further match." | `viewers_controls` › find | PASS |
| C3 | Find button, Next and Previous match buttons walk the matches in order (the highlight moves line by line, column by column) | `viewers_presentation` › Find bar | NOT YET RELIABLE: the highlight measurement still mislocates the 3rd and 4th match ((1219, 0)); fix the measurement before reading it as a product result |
| C4 | Aa (case sensitive) and Whole words | `viewers_controls` › find; `viewers_presentation` › Find bar | PASS |
| C5 | Clear find text empties the query and removes the highlight | `viewers_presentation` › Find bar | PASS |
| C6 | Escape closes the viewer while the search field has the focus | `viewers_presentation` › Find bar | PASS |
| C7 | Find in the hex view matches source bytes; Whole words is not offered there | `viewers_presentation` › Find in the hex view | PASS |
| C8 | Find in the HTML viewer (Chromium's search, "Match 1 of 1", "No matches"); no Whole words | `viewers_presentation` › HTML zoom, policy and Find | FAIL (minor): "1 matches" and never "Match 1 of 1" (worker_web.rs FindResult), assigned; check accepts it and flags the text |
| C9 | Wrap: Ctrl+W and the Wrap box; the horizontal scrollbar goes | `viewers_controls` › wrap | PASS |
| C10 | Font and presentation (Aa) row | `viewers_controls` › presentation row | PASS |
| C11 | Font size + button (taller rows) and font family picker (other glyphs) | `viewers_presentation` › font size and family | PASS |
| C12 | Font choices are kept for the next text file | `viewers_presentation` › preferences kept | PASS |
| C13 | Lines (source line numbers) off removes the gutter; kept for the next source file | `viewers_presentation` › Source and Text modes, Lines; preferences kept | PASS |
| C14 | Markdown presets GitHub, Compact, Comfortable move the text | `viewers_presentation` › Markdown and Text modes, presets | PASS |
| C15 | Mode picker: Text ↔ Binary | `viewers_controls` › text and binary modes | PASS |
| C16 | Mode picker: Source ↔ Text (syntax colours go and return), Markdown ↔ Text (raw markup) | `viewers_presentation` › modes | PASS |
| C17 | Image zoom: Ctrl+1, Ctrl+Plus, Ctrl+Minus, Zoom in button, Ctrl+wheel, Fit | `viewers_controls` › image zoom | PASS |
| C18 | Image 1:1 button, Ctrl+0 (Fit), dragging the picture | `viewers_presentation` › image 1:1, Fit and dragging | PASS |
| C19 | PDF Paged and Continuous layouts | `viewers_controls` › PDF layout; `viewers_scroll` › PDF | PASS |
| C20 | PDF pages: Ctrl+PageDown, Ctrl+PageUp, the page buttons, the counter, nothing before the first or after the last page | `viewers_presentation` › PDF pages | PASS |
| C21 | HTML zoom: buttons, Ctrl+Plus, Ctrl+0; Online and JavaScript boxes (unchecked; JavaScript needs Online) | `viewers_presentation` › HTML zoom, policy and Find | PASS |
| C22 | Space and Backspace show the next and previous file; Alt+Right and Alt+Left too; wrapping at the ends | `viewers_controls` › next and previous file | PASS for Space/Backspace; Alt+Right/Alt+Left and wrapping NOT YET VERIFIED (built, not run) |
| C23 | Space and Backspace in the HTML viewer (Chromium arbitrates the physical keys) | none | SKIP: Chromium arbitrates the physical keys through native window events, which this harness cannot deliver |
| C24 | Escape closes the viewer | every viewer scenario (`Viewing::close`) | PASS |
| C25 | The window's own close button | none | SKIP: the window decorations are native; a headless software window has none |

## Selection and copy

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| S1 | Ctrl+A selects the page's rows; Ctrl+C copies them (source without line numbers, as a prefix of the file) | `viewers_selection` › each theme and sample | PASS (Rust and CSV in every theme; plain text) |
| S2 | Markdown copies rendered text, the hex view its hex rows | `viewers_selection` › Markdown, hex view | PASS |
| S3 | A mouse drag and a Shift+click select runs of the file's text | `viewers_selection` › mouse selection | PASS |
| S4 | Selected text keeps 4.5:1 contrast against the selection in every theme (Rust source and CSV columns) | `viewers_selection` › every theme | REFUTED in source on 21 November (see above; the probe read antialiased pixels). Reported: Rust source in RosePineDawn 2.24:1 (#bb99aa on #985069) and GraphiteContrast 2.07:1 (#d28f5f on #ffdbc4); every other theme ≥ 4.5:1. Evidence: run 20261120-151137, shots/viewers_selection/ |
| S5 | The same for plain text, Markdown, the hex view, Python, JSON, XML, YAML, CSS and SQL in Blue, Dark and Light | `viewers_selection` › representative themes | REFUTED in source on 21 November (see above). Reported: numbers, strings and types of Python, JSON, YAML, CSS and SQL at 3.50–4.45:1 in Blue, Dark and Light (for example Light Python #865f43 on #cccccc 3.50:1). The measurement takes each colour group's most frequent far colour; thin glyphs may still bias it low, so compare with `readable_rgb` before filing |
| S6 | Unselected syntax colours below 3:1 against the background are flagged for review | `viewers_selection` › flags | PASS (flags only) |

## Scrolling

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| R1 | A content taller than the window shows a scrollbar with a visible thumb | `viewers_scroll` › scrollbar shown | PASS |
| R2 | Scrollbar drags from 0 % to 100 % (every 1 %) land without an error in the large log, CSV, TSV, CSV as plain text, Markdown with tables | `viewers_scroll` › scrollbar drags land | PASS (every 1 % in all five; the reported CSV drag failures are gone) |
| R3 | The same every 2 % in Rust source, the hex view, a PDF (Continuous) and an image at actual pixels | `viewers_scroll` › scrollbar drags land | FAIL for the PDF: 3–8 of 51 drags stop 2–4 % short of the release point (image_camera.rs Camera::receive drops the waiting target when a new surface arrives), assigned; Rust, hex and image PASS. Evidence: run 20261120-143743 |
| R4 | The horizontal scrollbar of a 200,000-character line | `viewers_scroll` › horizontal scrollbar | PASS |
| R5 | One wheel notch moves 60 px in every viewer kind; one up −60 px; a burst of five 300 px | `viewers_scroll` › wheel, same wheel travel | FAIL for Rust source: a burst of five notches moves 256 px, not 300 (motion past the retained raster guard is lost), assigned; one notch is 60 px in every viewer kind (PASS). Evidence: run 20261120-143743 |
| R6 | The HTML viewer's wheel travel per notch | `viewers_scroll` › HTML wheel | SKIP: the HTML viewer takes the wheel as a native window event; viewer.slint drops Slint's wheel event in web mode, so the harness moves the page 0 px |
| R7 | Down and Up move 28 px; PageDown and PageUp exactly one screen; Ctrl+End and Ctrl+Home reach the ends | `viewers_scroll` › keys | FAIL: CSV and TSV PageUp composite and Rust PageUp lose arrow presses after the page step (same cause as R5); image Ctrl+End/Ctrl+Home do nothing (assigned); CSV as plain text: a notch up across the page top moves 32 of 60 px and shows a row fragment (assigned). Log, Markdown, hex, PDF PASS. The held-key check (20 × Down = 560 px) is built, not run |
| R8 | HTML keys: Down, PageDown, Ctrl+End, Ctrl+Home (back to the first view, pixel for pixel) | `viewers_scroll` › HTML keys | PASS (Chromium: Down 40 px, PageDown 623 of 712 px) |
| R9 | Large log: Ctrl+End to the last line, Ctrl+Home, PageDown, the wheel | `viewers_controls` › keyboard and wheel scrolling | PASS |
| R10 | Kinetic (touchpad) coasting | none | SKIP: synthetic PointerScrolled events carry no touchpad phases and the harness delivers no native window events |

## File comparison (Ctrl+Shift+C)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| F1 | Two marked files compared: identical, the same text in two encodings, different | `viewers_compare` › identical, encodings, three differences | PASS |
| F2 | Difference navigation: N, P, F7, Shift+F7, Alt+Down, Alt+Up, Alt+Right, Alt+Left, Alt+Home, Alt+End, the Next and Previous difference buttons | `viewers_compare` › three differences | PASS |
| F3 | Find in both files: Find… button, Ctrl+F, Enter, F3, Shift+F3, Shift+Enter, "Match N of M · File · line", Aa, Whole words, Clear Find | `viewers_compare` › Find in the compared files | PASS |
| F4 | The text font chooser repaints the comparison | `viewers_compare` › Find in the compared files | PASS or SKIP per machine (passes when another fixed-pitch family renders differently) |
| F5 | Scrolling: an arrow moves one row, PageDown the visible rows, a wheel notch 60 px, the position bar | `viewers_compare` › scrolling the compared text | PASS (row 20 px, PageDown 35 rows, notch 60 px, position bar) |
| F6 | Images: Side by side, Swipe, Onion skin; the divider and opacity sliders; dragging the divider; Zoom in and out, Ctrl+wheel, Fit; Close | `viewers_compare` › images | PASS |
| F7 | Binary pairs in the hex view: summary, navigation keys and button | `viewers_compare` › binary files | PASS |
| F8 | The Hex button of a text pair shows its bytes and back | `viewers_compare` › text over the former 4 MiB limit | PASS |
| F9 | Large text: 3.5 MiB and 4.25 MiB logs compared as text | `viewers_compare` › large logs, over the former limit | PASS |
| F10 | Text over 32 MiB, over 1,000,000 lines, or with a line over 16 KiB is compared as bytes with the reason | `viewers_compare` › over a text limit | PASS |
| F11 | File A / File B chosen by hand: the resolved side prefilled, a missing path refused in the form (drafts kept), Browse…, Compare, Cancel | `viewers_compare` › File A and File B chosen by hand | PASS |
| F12 | Archive members: Compare asks for File A and File B (members have no native paths) | `viewers_archives` › Compare files on archive members | PASS |
| F13 | Compare files in a list of search results is refused with the reason | `viewers_combinations` › Find results and a fed list | PASS |
| F14 | Escape and Close close the comparator | `viewers_compare` | PASS |

## Directory comparison (Ctrl+F10)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| D1 | Size, time, tolerance 0/1/2, exact case, unmatched folders, Compare contents, Include subfolders | `tools_compare_dirs` | PASS |
| D2 | The dialog closes at Compare; no result window; the footer counts; the reopened dialog's summary | `tools_compare_dirs` | PASS |
| D3 | Invalid tolerances are reported in the dialog | `tools_compare_dirs` | PASS |
| D4 | Unreadable metadata and a dangling link count as unknown on both sides | `tools_compare_dirs` | PASS |
| D5 | A long content comparison: progress on the status line, F7 refused with the reason, progress and Cancel in the reopened dialog, Cancel selects nothing, the next comparison runs | `tools_compare_dirs` › running comparison | PASS |

## Thumbnails (Alt+5)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| T1 | Alt+5: every visible tile shows its own image, no flicker while still | `viewers_thumbnails` | PASS |
| T2 | End and the wheel scroll to tiles that load; Home shows cached tiles at once | `viewers_thumbnails` | PASS |
| T3 | PNG, JPEG, GIF, WebP and SVG thumbnails | `viewers_thumbnails` › image formats | PASS |
| T4 | BMP, ICO and TIFF thumbnails | `viewers_thumbnails` › BMP, ICO and TIFF thumbnails | PASS |
| T5 | A PDF thumbnail shows its first page | `viewers_thumbnails` › PDF first page | PASS |
| T6 | AVIF thumbnails | none | SKIP: no AVIF encoder to make a fixture |

## Archives

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| A1 | ZIP (stored), tar, tar.gz and an empty ZIP list their members at every level | `viewers_archives` › fixture archives | PASS |
| A2 | `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`, `.tbz2`, `.tbz`, `.tar.zst`, `.tzst`, 7-Zip and a deflated ZIP list their members | `viewers_archives` › generated archives | PASS |
| A3 | Raw `.gz`, `.xz`, `.bz2` and `.zst` streams list one member with the payload | `viewers_archives` › raw streams | PASS |
| A4 | F3 on members; Space shows the next member | `viewers_archives` › copy out and view inside, generated archives | PASS |
| A5 | F5 copies a member out byte for byte | `viewers_archives` | PASS |
| A6 | A ZIP inside a ZIP; 7-Zip ⊃ tar.gz ⊃ ZIP: Enter at each level, the path bar, F3 and F5 inside, Backspace out with the cursor on each archive | `viewers_archives` › nested.zip, 7-Zip ⊃ tar.gz ⊃ ZIP | PASS |
| A7 | The four-level limit: Enter on a fifth level explains it | `viewers_archives` › four-level limit | PASS |
| A8 | Damaged: a text file named `.zip`, a truncated ZIP, garbage after a 7-Zip signature, a corrupted tar.gz, a truncated 7-Zip give a clear error | `viewers_archives` › damaged cases | PASS |
| A9 | A member with a wrong CRC: the listing works, F3 and F5 fail clearly and leave no file | `viewers_archives` › crc.zip | PASS |
| A10 | 100,100 entries; 8.9 MB of names | `viewers_archives` › many.tar, long-names.tar | PASS |
| A11 | One folder of 100,001 files | `viewers_archives` › flat.tar | PASS |

## Combinations

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| X1 | F3 on a result in the Find window | `viewers_combinations` › Find results and a fed list | PASS |
| X2 | F3 in results fed to a panel; Space and Backspace follow the results | `viewers_combinations` › Find results and a fed list | PASS |
| X3 | F3 in an expanded inline tree branch; Space follows the visible file order | `viewers_combinations` › an inline tree branch | PASS |
| X4 | Four viewer windows at once, each with its own file; a fifth refused with the reason; closing one keeps the others | `viewers_combinations` › four viewer windows | PASS |
| X5 | A theme previewed in Options recolours an open viewer; Cancel restores it | `viewers_combinations` › a theme previewed in Options | PASS |
| X6 | A viewer moved to a display of twice the scale shows the same lines twice as tall | `viewers_combinations` › a viewer at twice the scale | PASS |
| X7 | Viewers on archive members (and inner archives' members) | `viewers_archives` | PASS |
| X8 | Every theme's selection colours | `viewers_selection` | REFUTED in source on 21 November (S4, S5). Reported: RosePineDawn and GraphiteContrast (S4) and syntax tokens in Blue, Dark and Light (S5); every other theme PASS |

## Not yet written

- Copy-out of the 100,001-file folder (F5, bytes and a time budget), Ctrl+Shift+S on it
  inside the archive (exact 5,151 bytes), and F5 of a 100,001-member selection
  (PENDING: copy-out of more than 4,096 entries is being enabled): written in
  `viewers_archives`, not run yet.
- The AppArmor Install… offer (O20), the Make File List viewer opened from Ctrl+M (P's
  area, hooked here), a viewer for every file reached from a Find feed of archive members.
