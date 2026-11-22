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

Latest recorded run: the Linux run `20261121-144246` of commit 3f6fff8a (WSL 2,
Ubuntu 24.04 on ext4, debug test build, software rendering, smoke scale, every plugin
package built from that commit by `./build.sh`; every scenario of the suite; report in
`_build/acceptance-linux/20261121-144246/`, staged children's own reports under its
`staged/`). Statuses are from that run unless another is named. Two environment limits
of that machine, not product results:

- **No HTML viewer.** The distribution lacks the HTML viewer's system libraries
  (`libnss3`, `libnspr4`, `libasound2t64`; [SYSTEM_REQUIREMENTS](../../SYSTEM_REQUIREMENTS.md)
  names them), so `midnight-viewer-html` exits at load (status 127) and every HTML case
  reports "Chromium transport stopped: failed to fill whole buffer" (`viewers_files`
  4 cases, `viewers_scroll`, `viewers_presentation`). `scripts/setup-linux.sh --install`
  did not install them then. Since 22 November it installs every library of the HTML
  viewer's Chromium, and the viewer names missing libraries and the command that
  installs them instead of that transport error (unit-tested; no acceptance run since).
  The run used a delegated systemd user scope, so the browser's cgroup-v2 delegation was
  available; AppArmor is off in WSL.
- **No `zstd`, `7z` or `zip`.** `viewers_archives` skipped the formats made with them.

Every `viewers_*` scenario ran again at full scale in run `20261121-171419` (with the
Linux harness patch, which for them only widens the layout checks' path exemption): the
same results (the 8 MiB log opened
in 99 ms and the 24 MiB binary in 81 ms; the folder copy-out out of `flat.tar` again ran
out of its budget, at about 52,390 files).

Earlier runs (debug test build, plugins from a complete plugin set of 20 November):
`viewers_files`, `viewers_compare`, `viewers_thumbnails`, `viewers_archives`,
`tools_compare_dirs` and `viewers_combinations` in one run; `viewers_scroll` alone;
`viewers_selection`, `viewers_presentation` and `viewers_combinations` in a third run.

Checked in source on 21 November, without a new acceptance run:

- The Source viewer's Lines check box lies inside 480×320 and 320×200 windows, at both
  densities and fonts of 12 to 32 px (`midnight-viewer-ui` test
  `ui_polish::source_toolbar_controls_stay_inside_small_windows_and_take_clicks`).
  `viewers_combinations` › compact reports it outside every window, 1000×720 included,
  because its visibility probe drops anything under an opacity-0 layer, and a
  `WrappingCheckBox` keeps its accessible native check box in such a layer over the
  drawn box (Word wrap too). The probe needs fixing, not the toolbar. In the Linux run
  `20261121-144246` `viewers_combinations` passed all 6 checks, its 480×320 and 320×200
  viewer windows included.
- With Find open, a short window pushed the viewer's footer outside it (320×200 with
  touch spacing, 560×200 with 32 px fonts). Since 21 November the rows above the
  text give way to the Find bar there, and the Find bar's Close find button (or Aa in
  Text mode) brings them back
  (`ui_polish::rows_above_the_text_give_way_to_find_in_short_windows_and_come_back`).
- S4, S5 and X8 are refuted: the ink the renderer draws for selected text keeps at
  least 4.50:1 against the selection for every theme (26) and syntax role (12, CSV
  columns included), 312 checks (`selection_contrast::selected_text_of_every_syntax_role_reaches_aa_contrast_in_every_theme`).
  The low readings were antialiased edge pixels, blends of that ink with the
  selection: the probe should take the highest-contrast pixel of large glyphs. The Linux
  run `20261121-144246`, with the Source viewer built from the current source, reports
  the same two themes (RosePineDawn #bd9ead on #985069 and GraphiteContrast #cb834e on
  #ffdbc4, both 2.35:1). A pixel count of its screenshots agrees with the analysis: the
  reported colours cover only 28 and 10 scattered edge pixels, while the glyph cores of
  line 4 are #faf4ed (code, 5.2:1) and #e1e5ec (comments, 4.51:1) in RosePineDawn, and
  #1b181b (code, 13.6:1), #3b65a5 (comments, 4.51:1) and #407119 (strings, 4.50:1) in
  GraphiteContrast. The probe still needs that change; note how close to 4.5:1 the
  comment and string inks are.

## Opening and routing (F3, Alt+F3)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| O1 | F3 opens the applicable installed viewer for every fixture file kind (40 files) | `viewers_files` › each fixture case | PASS |
| O2 | Alt+F3 forces the plain-text viewer (Markdown source shown, not rendered) | `viewers_controls` › Alt+F3 plain text; `viewers_scroll` › CSV as plain text | PASS |
| O3 | Every extension the Source viewer claims (58, including `makefile`/`gnumakefile` whole names) opens in Source mode with syntax colours | `viewers_files` › generated Source extensions | PASS |
| O4 | CSV and TSV show rainbow columns (text colour and four column hues) | `viewers_files` › `table.csv`, `table.tsv` | PASS |
| O5 | Markdown extensions `md`, `markdown`, `mdown`, `mkd` render | `viewers_files` › Markdown extensions | PASS |
| O6 | HTML extensions `html`, `htm`, `xhtml` open in the HTML viewer (Chromium) with local stylesheet and image | `viewers_files` › `docs/page.html`, `plain.htm`, `page.xhtml` | SKIP in the Linux run `20261121-144246`: the machine lacks the HTML viewer's system libraries (see above; the four cases failed with "Chromium transport stopped"). Last PASS on 20 November |
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
| O17 | Broken Markdown, HTML and JSON show as far as they go | `viewers_files` › `broken.*` | PASS for Markdown and JSON; `broken.html` SKIP (no HTML viewer on this machine, see above) |
| O18 | Open latency recorded per file (flagged over 500 ms, failing over 10 s) and its spread over 15 repeated opens | `viewers_files` › latency notes, repeated opens | PASS: no slow-open flag; 65–467 ms (the first open, `text/plain.txt`, 467 ms) |
| O19 | Every viewer kind's window passes the UI review's layout checks | `viewers_files` › layout review; `viewers_scroll` › layout review | PASS |
| O20 | The HTML viewer's AppArmor Install… offer when the sandbox is refused | none | NOT YET WRITTEN: needs a stage outside the AppArmor profile's `_build/*/viewers/...` pattern (a `viewers_stage` change) |

## Viewer controls

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| C1 | Status bar (bytes, line and column) | `viewers_controls` › status bar | PASS |
| C2 | Find: Ctrl+F, Enter, F3, Shift+F3, "No further match." | `viewers_controls` › find | PASS |
| C3 | Find button, Next and Previous match buttons walk the matches in order (the highlight moves line by line, column by column) | `viewers_presentation` › Find bar | PASS with the corrected measurement: matches at (0, 0), (25, 16), (0, 32), (8, 48); Shift+F3 back to (0, 32), Previous to (25, 16) |
| C4 | Aa (case sensitive) and Whole words | `viewers_controls` › find; `viewers_presentation` › Find bar | PASS |
| C5 | Clear find text empties the query and removes the highlight | `viewers_presentation` › Find bar | PASS |
| C6 | Escape closes the viewer while the search field has the focus | `viewers_presentation` › Find bar | PASS |
| C7 | Find in the hex view matches source bytes; Whole words is not offered there | `viewers_presentation` › Find in the hex view | PASS |
| C8 | Find in the HTML viewer (Chromium's search, "Match 1 of 1", "No matches"); no Whole words | `viewers_presentation` › HTML zoom, policy and Find | FAIL (minor, 20 November): "1 matches" and never "Match 1 of 1" (worker_web.rs FindResult), assigned; check accepts it and flags the text. Not run in the Linux run `20261121-144246` (no HTML viewer on that machine) |
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
| C21 | HTML zoom: buttons, Ctrl+Plus, Ctrl+0; Online and JavaScript boxes (unchecked; JavaScript needs Online) | `viewers_presentation` › HTML zoom, policy and Find | PASS on 20 November; not run in the Linux run `20261121-144246` (no HTML viewer on that machine) |
| C22 | Space and Backspace show the next and previous file; Alt+Right and Alt+Left too; wrapping at the ends | `viewers_controls` › next and previous file | PASS, Alt+Right/Alt+Left and the wrap from the first file to the last included |
| C23 | Space and Backspace in the HTML viewer (Chromium arbitrates the physical keys) | none | SKIP: Chromium arbitrates the physical keys through native window events, which this harness cannot deliver |
| C24 | Escape closes the viewer | every viewer scenario (`Viewing::close`) | PASS |
| C25 | The window's own close button | none | SKIP: the window decorations are native; a headless software window has none |

## Selection and copy

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| S1 | Ctrl+A selects the page's rows; Ctrl+C copies them (source without line numbers, as a prefix of the file) | `viewers_selection` › each theme and sample | PASS (Rust and CSV in every theme; plain text) |
| S2 | Markdown copies rendered text, the hex view its hex rows | `viewers_selection` › Markdown, hex view | PASS |
| S3 | A mouse drag and a Shift+click select runs of the file's text | `viewers_selection` › mouse selection | PASS |
| S4 | Selected text keeps 4.5:1 contrast against the selection in every theme (Rust source and CSV columns) | `viewers_selection` › every theme | REFUTED in source on 21 November (see above; the probe read antialiased pixels). Reported: Rust source in RosePineDawn and GraphiteContrast, 2.35:1 each in the Linux run `20261121-144246` (2.24:1 and 2.07:1 in run 20261120-151137); every other theme ≥ 4.5:1. The scenario fails on these two readings until the probe changes |
| S5 | The same for plain text, Markdown, the hex view, Python, JSON, XML, YAML, CSS and SQL in Blue, Dark and Light | `viewers_selection` › representative themes | PASS in the Linux run `20261121-144246` (no reading below 4.5:1). Run 20261120-151137 had reported numbers, strings and types of Python, JSON, YAML, CSS and SQL at 3.50–4.45:1 (for example Light Python #865f43 on #cccccc 3.50:1), refuted in source on 21 November (see above) |
| S6 | Unselected syntax colours below 3:1 against the background are flagged for review | `viewers_selection` › flags | PASS (flags only) |

## Scrolling

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| R1 | A content taller than the window shows a scrollbar with a visible thumb | `viewers_scroll` › scrollbar shown | PASS |
| R2 | Scrollbar drags from 0 % to 100 % (every 1 %) land without an error in the large log, CSV, TSV, CSV as plain text, Markdown with tables | `viewers_scroll` › scrollbar drags land | PASS (every 1 % in all five; the reported CSV drag failures are gone) |
| R3 | The same every 2 % in Rust source, the hex view, a PDF (Continuous) and an image at actual pixels | `viewers_scroll` › scrollbar drags land | PASS in all four (51 drags each; landing median 0.79–0.96 s). On 20 November 3–8 of the PDF's 51 drags stopped 2–4 % short (run 20261120-143743) |
| R4 | The horizontal scrollbar of a 200,000-character line | `viewers_scroll` › horizontal scrollbar | PASS |
| R5 | One wheel notch moves 60 px in every viewer kind; one up −60 px; a burst of five 300 px | `viewers_scroll` › wheel, same wheel travel | PASS: 60 px per notch and the burst in every viewer kind, Rust source included. On 20 November a burst of five moved 256 px in Rust source (run 20261120-143743) |
| R6 | The HTML viewer's wheel travel per notch | `viewers_scroll` › HTML wheel | SKIP: the HTML viewer takes the wheel as a native window event; viewer.slint drops Slint's wheel event in web mode, so the harness moves the page 0 px |
| R7 | Down and Up move 28 px; PageDown and PageUp exactly one screen; Ctrl+End and Ctrl+Home reach the ends | `viewers_scroll` › keys | PASS in every viewer kind (Down 28 px, PageDown 712 or 748 px = one screen, Ctrl+End 1.0, Ctrl+Home 0.0; the image's Ctrl+End and Ctrl+Home included), the held-key check too. The 20 November failures (CSV/TSV/Rust PageUp, the image's Ctrl+End/Home, CSV as plain text) did not recur |
| R8 | HTML keys: Down, PageDown, Ctrl+End, Ctrl+Home (back to the first view, pixel for pixel) | `viewers_scroll` › HTML keys | PASS on 20 November (Chromium: Down 40 px, PageDown 623 of 712 px); not run in the Linux run `20261121-144246` (no HTML viewer on that machine: "open: long.html: Chromium transport stopped", the scenario's only failure) |
| R9 | Large log: Ctrl+End to the last line, Ctrl+Home, PageDown, the wheel | `viewers_controls` › keyboard and wheel scrolling | PASS |
| R10 | Kinetic (touchpad) coasting | none | SKIP: synthetic PointerScrolled events carry no touchpad phases and the harness delivers no native window events |

## File comparison (Ctrl+Shift+C)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| F1 | Two marked files compared: identical, the same text in two encodings, different | `viewers_compare` › identical, encodings, three differences | PASS |
| F2 | Difference navigation: N, P, F7, Shift+F7, Alt+Down, Alt+Up, Alt+Right, Alt+Left, Alt+Home, Alt+End, the Next and Previous difference buttons | `viewers_compare` › three differences | PASS |
| F3 | Find in both files: Find… button, Ctrl+F, Enter, F3, Shift+F3, Shift+Enter, "Match N of M · File · line", Aa, Whole words, Clear Find | `viewers_compare` › Find in the compared files | PASS |
| F4 | The text font chooser repaints the comparison | `viewers_compare` › Find in the compared files | PASS or SKIP per machine (passes when another fixed-pitch family renders differently): PASS in the Linux run (System monospace → Ubuntu Mono repaints; DejaVu Sans Mono renders alike) |
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
| T1 | Alt+5: every visible tile shows its own image, no flicker while still | `viewers_thumbnails` | PASS on Windows (run `20261122-040925`, smoke): 9 of 9 tiles after 6.5 s. Linux FAIL: THUMB-UNKNOWN-STORAGE (below), fixed in source on 22 November, rerun pending |
| T2 | End and the wheel scroll to tiles that load; Home shows cached tiles at once | `viewers_thumbnails` | PASS on Windows (run `20261122-040925`, smoke). Linux FAIL: THUMB-UNKNOWN-STORAGE, fixed in source on 22 November, rerun pending |
| T3 | PNG, JPEG, GIF, WebP and SVG thumbnails | `viewers_thumbnails` › image formats | PASS on Windows (run `20261122-040925`, smoke). Linux FAIL: THUMB-UNKNOWN-STORAGE, fixed in source on 22 November, rerun pending |
| T4 | BMP, ICO and TIFF thumbnails | `viewers_thumbnails` › BMP, ICO and TIFF thumbnails | PASS on Windows (run `20261122-040925`, smoke). Linux FAIL: THUMB-UNKNOWN-STORAGE, fixed in source on 22 November, rerun pending |
| T5 | A PDF thumbnail shows its first page | `viewers_thumbnails` › PDF first page | PASS on Windows (run `20261122-040925`, smoke): a white page of 96 x 64 px with 67 dark text pixels. Linux FAIL: THUMB-UNKNOWN-STORAGE, fixed in source on 22 November, rerun pending |
| T6 | AVIF: libavif's `draw_points_idat.avif` (the image decoder's licensed fixture) looks like its PNG twin | `viewers_thumbnails` › the AVIF thumbnail matches its PNG twin | PASS on Windows (run `20261122-040925`, smoke) |
| T7 | The JPEG, GIF, WebP and SVG tiles show their images' own colours (grey blocks, palette, checkerboard, the SVG's fills), not a placeholder | `viewers_thumbnails` › the … tile's pixels | PASS on Windows (run `20261122-040925`, smoke) |
| T8 | A password-protected PDF (RC4, `encode::pdf_encrypted`) keeps its placeholder | `viewers_thumbnails` › a password-protected PDF keeps its placeholder | PASS on Windows (run `20261122-040925`, smoke) |
| T9 | Two-frame GIF and WebP tiles show the first frame | `viewers_thumbnails` › … shows its first frame | PASS on Windows (run `20261122-040925`, smoke) |
| T10 | A program's tile shows its own icon (Windows: a copy of `explorer.exe` against a file named `.exe` without icon resources; the harness installs only the desktop service's icon reader, decision A3) | `viewers_thumbnails` › a program's own icon | PASS on Windows (run `20261122-040925`, smoke); Linux tiles show type icons by design (reported SKIP) |

THUMB-UNKNOWN-STORAGE (Linux run `20261121-144246`, again in `20261121-164146`): in the
Thumbnails view no tile ever showed a thumbnail (`viewers_thumbnails`: 7 of 11 checks
failed, no notice), on a local folder of the WSL machine. A harness diagnostic showed
that the presenter asked for no thumbnail at all (`thumbnail_targets` returned 0 rows).
The machine's disk is a Hyper-V virtual disk, which the Linux storage classification
reports as `Unknown` by design (`midnight-platform-linux/src/storage_kind.rs`,
`classify_physical`: virtual, VMware, QEMU, cloud and other disks cannot prove local
storage), and `panel_thumbnails_allowed` (`midnight-ui/src/thumbnail_presentation.rs`)
allows thumbnails on `Unknown` storage only when Options › Previews' network previews are
on (off by default), because "Unknown must not start a read before a delayed network
classification". On a virtual machine's or cloud machine's local disk `Unknown` is the
final classification, not a transient one, so thumbnails never appear there by default
and nothing says why. With network previews turned on in the harness's seeded
preferences (a temporary experiment, run `20261121-170455`) every check passed in 20.5 s
on the same machine. Decided on 22 November (A2): a local block device of unknown kind
counts as local storage (exclusive I/O, previews allowed), and a withheld preview's
reason shows in the panel's status row instead of a modal notice; that fix is not on
main at 05710878. Since 22 November `viewers_thumbnails` names THUMB-UNKNOWN-STORAGE in its
failure when no thumbnail appears on storage the platform reports as `Unknown`.

Fixed in source on 22 November, both ways (the owner's decision: a local block device
whose kind cannot be proven counts as local for previews; network storage and pending
classifications keep the rule). Not yet rerun on Linux, so T1–T5 stay FAIL there until
an acceptance run passes them. The native filesystem plugins report a disk attached to the machine
whose media they cannot prove, the WSL disk among them, as `StorageKind::Local`
(filesystem ABI minor 4; `classify_physical` in both Linux classifiers,
`classify_descriptor` in both Windows ones); the presenter tells a pending
classification (`None`) from a final one and admits `Local` without network previews,
with exclusive I/O (`thumbnail_availability.rs`). A Thumbnails view withheld by a
setting (network storage, storage not known to be local, archive members with archive
previews off) names the reason and the option in its panel's status row, never in a
dialog. Tests: the Linux classifiers' `virtual_machine_and_removable_disks_are_local_of_unknown_media`
and `local_disks_of_unknown_media_stay_local_through_stacks` (run in WSL when the fix was
made, where WSL's own disks then classified as `Local` instead of `Unknown`), the presenter's
`a_local_disk_of_unknown_media_admits_previews_without_network_permission`,
`network_and_unclassified_storage_withhold_previews_and_say_which_option_allows_them`
and the window test `storage_reasons_and_preview_failures_are_status_rows_that_leave_the_keyboard_alone`.

## Archives

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| A1 | ZIP (stored), tar, tar.gz and an empty ZIP list their members at every level | `viewers_archives` › fixture archives | PASS |
| A2 | `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`, `.tbz2`, `.tbz`, `.tar.zst`, `.tzst`, 7-Zip and a deflated ZIP list their members | `viewers_archives` › generated archives | PASS for `.tgz`, `.tar.xz`, `.txz`, `.tar.bz2`, `.tbz2` and `.tbz`; `.tar.zst`, `.tzst`, 7-Zip and the deflated ZIP SKIP in the Linux run (no `zstd`, `7z` or `zip` on that machine; PASS on 20 November) |
| A3 | Raw `.gz`, `.xz`, `.bz2` and `.zst` streams list one member with the payload | `viewers_archives` › raw streams | PASS for `.gz`, `.xz` and `.bz2`; `.zst` SKIP in the Linux run (no `zstd`) |
| A4 | F3 on members; Space shows the next member | `viewers_archives` › copy out and view inside, generated archives | PASS |
| A5 | F5 copies a member out byte for byte | `viewers_archives` | PASS |
| A6 | A ZIP inside a ZIP; 7-Zip ⊃ tar.gz ⊃ ZIP: Enter at each level, the path bar, F3 and F5 inside, Backspace out with the cursor on each archive | `viewers_archives` › nested.zip, 7-Zip ⊃ tar.gz ⊃ ZIP | PASS for nested.zip; 7-Zip ⊃ tar.gz ⊃ ZIP SKIP in the Linux run (no `7z`; PASS on 20 November) |
| A7 | The four-level limit: Enter on a fifth level explains it | `viewers_archives` › four-level limit | PASS |
| A8 | Damaged: a text file named `.zip`, a truncated ZIP, garbage after a 7-Zip signature, a corrupted tar.gz, a truncated 7-Zip give a clear error | `viewers_archives` › damaged cases | PASS |
| A9 | A member with a wrong CRC: the listing works, F3 and F5 fail clearly and leave no file | `viewers_archives` › crc.zip | PASS |
| A10 | 100,100 entries; 8.9 MB of names | `viewers_archives` › many.tar, long-names.tar | PASS |
| A11 | One folder of 100,001 files: its listing and Ctrl+Shift+S inside it | `viewers_archives` › flat.tar | PASS for its listing (647 ms) and Ctrl+Shift+S (5,151 bytes in 995 ms) in the Linux run `20261121-144246` |
| A12 | F5 copies a folder of 10,001 files out of a tar, then a Ctrl+A selection of all of them (more than the 4,096 entries a selection once stopped at), byte for byte within 360 s each | `viewers_archives` › F5 of the folder / of a selection of 10,001 … out of copy.tar | NOT RUN YET (written 22 November) |
| A13 | The same with the 100,001 files of `flat.tar`, within 40 minutes each, with the runner's opt-in stress sizes (`--stress`, `MIDNIGHT_ACCEPTANCE_STRESS=1`) | `viewers_archives` (stress sizes) | NOT RUN YET. Earlier runs at 600 s ran out of time: 59,456 files on Windows (21 November, WIN-8), about 48,470 in the Linux run `20261121-144246` (about 81 files/s) |
| A14 | A quick search, then F3: the search closes and the focused match opens, in a native folder and inside an archive | `viewers_archives` › quick search, then F3 | NOT RUN YET (written 22 November) |

## Archive passwords

The archives are written by `encode::zip_entries` (ZipCrypto and WinZip AES, checked
against independent readers, see ACCEPTANCE.md); the password is non-ASCII.

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| P1 | Encrypted rows end " · Encrypted" (and only they); the details name the method ("ZipCrypto", "AES-256", …) with its explanation | `viewers_passwords` › … listing and details | PASS on Windows (run `20261122-040925`, smoke) |
| P2 | F3 on an encrypted member asks "<archive> is password-protected. Enter its password to view <member>."; an empty field gives "Type the password first.", a wrong password "Wrong password — try again."; Escape opens nothing and the notice says why | `viewers_passwords` › zipcrypto.zip: F3 asks; empty, wrong, Escape | PASS on Windows (run `20261122-040925`, smoke) |
| P3 | The right password (OK by pointer, Enter by keyboard) opens the viewer; the other members and Alt+F3 then open without asking; leaving and entering the archive asks again; Cancel opens nothing | `viewers_passwords` › the password is asked once per archive | PASS on Windows (run `20261122-040925`, smoke) |
| P4 | AES-256 (stored and deflated), AES-128 and AES-192 members; a plain member beside encrypted ones opens without asking | `viewers_passwords` › aes.zip, aes128.zip, aes192.zip | PASS on Windows (run `20261122-040925`, smoke) |
| P5 | An AES archive inside a ZipCrypto archive: Enter asks for the outer password, F3 inside for the inner archive's own (the outer one is refused there); Escape at the outer password keeps the panel | `viewers_passwords` › nested.zip | PASS on Windows (run `20261122-040925`, smoke) |
| P6 | Members no reader opens say why in their details and ask for nothing: Implode, an unsafe name (`../x`), a link, PKWARE strong encryption | `viewers_passwords` › odd.zip | PASS on Windows (run `20261122-040925`, smoke) |
| P7 | A 7-Zip archive with encrypted names asks before it lists | `viewers_passwords` › names.7z | SKIP on Windows (run `20261122-040925`): no `7z` on that machine |
| P8 | The password reaches neither the saved settings, the clipboard, the harness's event trace nor the staged child's console | `viewers_passwords` › no leaks | PASS on Windows (run `20261122-040925`, smoke) |

## Office documents

The packages are written by `encode::ooxml` with the extra fields Office writes (growth
hints 0xA220, extended timestamps).

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| OF1 | Padded DOCX, PPTX and XLSX render; Find reports "Match 1 of 1" | `viewers_office` › padded.… | PASS on Windows (run `20261122-040925`, smoke): "Match 1 of 1" for each |
| OF2 | A package whose parts are encrypted: "This presentation is password-protected, so the preview cannot open it." | `viewers_office` › locked.pptx | PASS on Windows (run `20261122-040925`, smoke) |
| OF3 | An OLE compound file: "This document is password-protected or saved in the older binary Office format (.doc, .xls or .ppt), which the preview cannot open." | `viewers_office` › protected.docx | PASS on Windows (run `20261122-040925`, smoke) |
| OF4 | With the HTML viewer disabled in the settings: "Office preview requires an installed and enabled Chromium HTML viewer." | `viewers_office` › without the HTML viewer | PASS on Windows (run `20261122-040925`, smoke) |
| OF5 | Linux without the browser's system libraries: the viewer names them and the command that installs them (b8537464) | `viewers_office` › the HTML viewer names its missing libraries | NOT RUN YET (Linux only, where libraries are missing) |

## Video

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| VD1 | Each video's own tile shows its colour, not the fallback icon | `viewers_video` › thumbnails | PASS on Windows (run `20261122-040925`, smoke): 6,902–6,912 pixels of the first colour per tile |
| VD2 | F3 opens the first frame; a click at 75 % of the position slider shows the later frames, Home on the slider the first again; the Play button plays to the end | `viewers_video` › F3 on … | PASS on Windows (run `20261122-040925`, smoke) |
| VD3 | An H.264 + AAC movie through the system's decoders: Windows' encoders and Media Foundation; on Linux the host's `ffmpeg -c:v libx264` and the distribution's FFmpeg (GStreamer is the fallback when FFmpeg is missing, not exercised) | `viewers_video` › F3 on h264.mp4 | PASS on Windows (run `20261122-040925`, smoke) |
| VD4 | NTFS-compressed copies (Windows) | `viewers_video` › F3 on packed.mp4, h264-packed.mp4 | PASS on Windows (run `20261122-040925`, smoke); SKIP on Linux |

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
| X8 | Every theme's selection colours | `viewers_selection` | REFUTED in source on 21 November (S4, S5). Reported in the Linux run `20261121-144246`: RosePineDawn and GraphiteContrast Rust source (S4), 2.35:1 each, which the pixel count above attributes to edge pixels; every other theme and sample PASS |

## Not yet written

- The AppArmor Install… offer (O20), the Make File List viewer opened from Ctrl+M (P's
  area, hooked here), a viewer for every file reached from a Find feed of archive members.
