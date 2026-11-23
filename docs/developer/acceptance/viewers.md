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
`Failures` with pending and skipped checks), `viewers_probe.rs` (scrollbar drags and the
frames shown during a drag, Slint pixel deltas, settling, the byte range of a text page,
how far rendered content moved, text colours at their glyphs' solid cores, WCAG contrast
and the product's minima, how sharp rendered text is, whether the HTML viewer can run
here, the layout review of content windows) and `viewers_fixtures.rs` (content the
scenarios generate in their own work folders, SVG feature and refusal documents,
two-frame GIF and WebP files, and the image decoder's licensed AVIF fixtures).

The harness sends Slint's own pointer, wheel and key events. Production converts a wheel
notch (a winit `LineDelta`, with the system's lines per notch), focus changes and the
reduce-motion setting only from native window events (`on_winit_window_event`), which
those events never reach. The acceptance-only native input seam (decision A1,
`midnight_viewer_ui::acceptance_seams`, compiled only with `midnight-acceptance`'s
`acceptance` feature) delivers such events to a viewer window through the same observer:
`viewers_probe` turns the wheel by native notches, moves the system focus away and back,
moves and clicks the pointer over an HTML page (whose browser reads the pointer only from
native events; C42), and overrides the system's wheel and reduce-motion settings, which a window reads when it
next gains the focus, as it reads the system's. The comparator has a seam of its own
(`midnight_compare_ui::acceptance_seams`, the same feature) that turns its wheel by native
notches (F15). Checks with Slint's own wheel event
measure that a Slint pixel delta moves the content exactly its pixels.

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

WP5a's scenarios (`viewers_files`, `viewers_controls`, `viewers_presentation`,
`viewers_scroll`, `viewers_selection`, `viewers_compare`, `viewers_combinations`) ran again
on 22 November with the checks of this record; the statuses of their rows are from these
runs (smoke scale, debug test builds, software rendering):

- Windows 11, the release plugin packages of main 05710878, test builds of main 5ccd0eb7
  with these scenarios: runs `20261122-104409` (`viewers_files`, `viewers_scroll`),
  `20261122-152615` (`viewers_presentation`) and `20261122-135339` (`viewers_controls`,
  `viewers_compare`, `viewers_combinations`, `viewers_selection`).
- Linux (WSL 2, Ubuntu 24.04), the debug plugin packages of main b1b89343, a test build of
  main 5ccd0eb7 with these scenarios in a shared Cargo target whose path packages were
  cleaned first: run `20261122-110817` (all seven). That machine has no writable cgroup-v2
  delegation for the HTML viewer's browser, which refuses to start there; every HTML check
  reports `SKIP` with the browser's own sentence.
- Red runs (Windows; each new check broken in a copy of the scenarios, by a wrong expected
  value or action, and seen failing with its own message): `20261122-080620` and `20261122-115842`.
- After the source highlighting fix (main d96dec97, test builds with these scenarios; the same
  packages, so the Source plugin's own part of the fix, 200d924c, is not in them): Windows runs
  `20261122-182535` (`viewers_scroll`) and `20261122-185939` (`viewers_files`, `viewers_presentation`,
  `viewers_selection`, `viewers_combinations`: the same results as before), Linux run `20261122-185841`
  (`viewers_scroll`).
- On main 200589ca (with link following, 821957a1, and the acceptance seam's frames for resumed
  glides, bc78af01), a Windows test build with these scenarios and the release packages of main
  05710878 with the Markdown viewer plugin rebuilt from main ad91634e, the same plugin as at
  200589ca (before 821957a1 only web, mail and fragment targets were links there): runs
  `20261122-212918` (`viewers_controls` with the links section) and `20261122-212204` (red: the same
  scenario with each new links check broken by a wrong expected value or outcome, each
  seen failing with its own message). `viewers_scroll` was not run again on this commit;
  the final run at the release commit covers it.

Settled since then:

- `viewers_combinations` › small viewer windows tests where a control is placed
  (`inspect::placed_rect`), so a `WrappingCheckBox` (Lines, Word wrap), whose accessible
  native check box lies under an opacity-0 layer over the drawn box, counts as inside
  the window; it passed all 6 checks in the Linux run `20261121-144246`.
- With Find open, a short window pushed the viewer's footer outside it (320×200 with
  touch spacing, 560×200 with 32 px fonts). Since 21 November the rows above the
  text give way to the Find bar there, and the Find bar's Close find button (or Aa in
  Text mode) brings them back
  (`ui_polish::rows_above_the_text_give_way_to_find_in_short_windows_and_come_back`).
- Selection colours: since aac18026 every palette has its own selected-text colour, which
  the viewer uses for every selected glyph and underline (4.5:1, 7:1 in the Contrast and
  12:1 in the High Contrast palettes, and an inactive pair). `viewers_selection` checks
  exactly that (S4–S8): the painted selection against the theme's selection colour, the
  solid cores of the selected glyphs right of the gutter against the theme's
  selected-text colour, the pair against the theme's minimum. The low readings of the
  earlier probe (2.35:1 in RosePineDawn and GraphiteContrast) were antialiased edge
  pixels and the gutter's numbers.

## Opening and routing (F3, Alt+F3)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| O1 | F3 opens the applicable installed viewer for every fixture file kind (40 files) | `viewers_files` › each fixture case | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O2 | Alt+F3 forces the plain-text viewer (Markdown source shown, not rendered) | `viewers_controls` › Alt+F3 plain text; `viewers_scroll` › CSV as plain text | PASS (Windows `20261122-135339`, `20261122-104409`; Linux `20261122-110817`) |
| O3 | Every extension the Source viewer claims (58, including `makefile`/`gnumakefile` whole names) opens in Source mode with syntax colours | `viewers_files` › generated Source extensions | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O4 | CSV and TSV show rainbow columns (text colour and four column hues) | `viewers_files` › `table.csv`, `table.tsv` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O5 | Markdown extensions `md`, `markdown`, `mdown`, `mkd` render | `viewers_files` › Markdown extensions | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O6 | HTML extensions `html`, `htm`, `xhtml` open in the HTML viewer (Chromium) with local stylesheet and image | `viewers_files` › `docs/page.html`, `plain.htm`, `page.xhtml` | PASS on Windows (`20261122-104409`); SKIP on Linux: no writable cgroup-v2 delegation for the browser there |
| O7 | Image extensions PNG, JPEG (`.jpg`, `.jpeg`), GIF, WebP, SVG, BMP, ICO, TIFF (`.tif`, `.tiff`) show their pixels | `viewers_files` › image cases | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O8 | AVIF images: the image decoder's licensed fixtures (libavif 1.3.0 test data, BSD-2-Clause, `plugins/image-decoder/tests/fixtures`): the quality-100 `draw_points_idat.avif` with its alpha item shows the pixels of its reference `draw_points.png` (both at a typed 800 % zoom, within 6 per channel for 99.5 % of the image's pixels); `white_1x1.avif` is white | `viewers_files` › AVIF fixtures | PASS (Windows `20261122-104409`, Linux `20261122-110817`): all 23,232 picture pixels within 6 of the PNG's (largest deviation 1); the 1×1 picture white |
| O9 | PDF opens with its page counter | `viewers_files` › `docs/report.pdf` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O10 | Binary files open in the hex view; one printable byte is text | `viewers_files` › `sizes/*` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O11 | Empty text and binary files open without content or error | `viewers_files` › `text/empty.txt`, `sizes/zero.bin` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O12 | Encodings: UTF-8, UTF-8 BOM (hidden), UTF-16 LE/BE with BOM, CRLF (no CR shown), no final newline | `viewers_files` › text cases | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O13 | Invalid UTF-8 and odd UTF-16 bytes are made visible (`\xFF`, `\uD800`) | `viewers_files` › `invalid-utf8.txt`, `odd-utf16.txt` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O14 | NUL bytes are never shown raw (hex view or `\x00`) | `viewers_files` › `nul.txt` | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O15 | Damaged images: truncated PNG, GIF, WebP, BMP, TIFF, zero-byte PNG, broken SVG are refused with "This image is damaged or incomplete" and the decoder's reason | `viewers_files` › damaged cases | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O16 | Damaged PDFs: garbage after the signature and empty files are refused; a truncated PDF or JPEG shows what can be decoded or is refused, never an error or hang | `viewers_files` › damaged cases | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O17 | Broken Markdown, HTML and JSON show as far as they go | `viewers_files` › `broken.*` | PASS (Windows `20261122-104409`, Linux `20261122-110817`); `broken.html` SKIP on Linux: no writable cgroup-v2 delegation for the browser there |
| O18 | Open latency recorded per file (flagged over 500 ms, failing over 10 s) and its spread over 15 repeated opens | `viewers_files` › latency notes, repeated opens | PASS (Windows `20261122-104409`, Linux `20261122-110817`): no open near the 10 s limit. Flagged for review on Windows (debug build on a shared machine): 33 opens over 500 ms, the first one 2.5 s, the HTML page 6.2 s |
| O19 | Every viewer kind's window passes the UI review's layout checks | `viewers_files` › layout review; `viewers_scroll` › layout review | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| O20 | The HTML viewer's AppArmor Install… offer when the sandbox is refused | `viewers_files` › the AppArmor offer | SKIP, with the reason in the scenario's message: Windows has no AppArmor; WSL has AppArmor off. It needs Ubuntu outside WSL that restricts unprivileged user namespaces and a stage outside the profile's `_build/*/viewers/...` pattern (a `viewers_stage` change) |
| O21 | SVG features drawn: a marker, a pattern, a mask, a clip path, a blur filter, a `data:` PNG and a `switch` fallback for `foreignObject` (each proven by the pixel count of its own colour on a 200 × 120 page at 1:1); a script and images linked to a local file (`linked.png`, magenta) and a URL neither run nor show (the page's colour covers all of it, no magenta) | `viewers_files` › `svg-*.svg` | PASS (Windows `20261122-104409`, Linux `20261122-110817`): every feature's colour in its pixel range; no magenta of the linked image |
| O22 | Hostile SVGs refused before drawing with the decoder's own sentence: no SVG namespace, XML entities (a DOCTYPE internal subset), nesting past 64 levels, a `use` that refers to itself | `viewers_files` › `svg-no-namespace.svg`, `svg-entities.svg`, `svg-deep.svg`, `svg-cycle.svg` | PASS (Windows `20261122-104409`, Linux `20261122-110817`): each refused with its sentence ("This SVG does not declare the SVG namespace …", "… declares XML entities …", "… more than 64 levels deep (line 1) …", "This SVG is invalid: the reference on line 2 leads back to itself.") |
| O23 | Two-frame GIF and WebP (100 ms frames, looping) show their first frame and hold still | `viewers_files` › `two-frames.gif`, `two-frames.webp` | PASS (Windows `20261122-104409`, Linux `20261122-110817`): the first frame's colour only, unchanged while still |

## Viewer controls

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| C1 | Status bar: the large log's page as "Bytes 0–N / length" with the file's exact length | `viewers_controls` › status bar | PASS (Windows `20261122-135339`, Linux `20261122-110817`): "Bytes 0–30242 / 1048600" for the 1,048,600-byte log |
| C2 | Find: Ctrl+F, Enter, F3, Shift+F3, "No further match." | `viewers_controls` › find | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| C3 | Find button, Next and Previous match buttons walk the matches in order (the highlight moves line by line, column by column) | `viewers_presentation` › Find bar | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C4 | Aa (case sensitive) and Whole words | `viewers_controls` › find; `viewers_presentation` › Find bar | PASS (Windows `20261122-135339`, `20261122-152615`; Linux `20261122-110817`) |
| C5 | Clear find text empties the query and removes the highlight | `viewers_presentation` › Find bar | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C6 | Escape closes the viewer while the search field has the focus | `viewers_presentation` › Find bar | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C7 | Find in the hex view matches source bytes; Whole words is not offered there | `viewers_presentation` › Find in the hex view | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C8 | Find in the HTML viewer (Chromium's search; the whole status "Long page · Offline · JavaScript on · Match 1 of 1", then "… · No matches": the page's title, then its policy); no Whole words; Reload reads the page again and the status returns to "Long page · Offline · JavaScript on" | `viewers_presentation` › HTML zoom, typed zoom, policy and Find | PASS on Windows (html-browser branch run `20261123-172442`, 0.5.1); SKIP on Linux (no cgroup-v2 delegation for the browser) |
| C9 | Wrap: Ctrl+W and the Wrap box; the horizontal scrollbar goes | `viewers_controls` › wrap | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| C10 | Font and presentation (Aa) row | `viewers_controls` › presentation row | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| C11 | Font size + button (taller rows) and font family picker (other glyphs) | `viewers_presentation` › font size and family | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C12 | Font choices are kept for the next text file | `viewers_presentation` › preferences kept | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C13 | Lines (source line numbers) off removes the gutter; kept for the next source file | `viewers_presentation` › Source and Text modes, Lines; preferences kept | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C14 | Markdown presets GitHub, Compact, Comfortable move the text | `viewers_presentation` › Markdown and Text modes, presets | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C15 | Mode picker: Text ↔ Binary | `viewers_controls` › text and binary modes | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| C16 | Mode picker: Source ↔ Text (syntax colours go and return), Markdown ↔ Text (raw markup) | `viewers_presentation` › modes | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C17 | Image zoom: Ctrl+1, Ctrl+Plus, Ctrl+Minus, Ctrl+Equal (one step of 1.25), Zoom in button, Ctrl+wheel, Fit (back to the fit zoom the picture opened with, away from the Ctrl+wheel zoom) | `viewers_controls` › image zoom | PASS (Windows `20261122-135339`, Linux `20261122-110817`): Ctrl+Plus 125.0 %, Ctrl+Equal one step, Ctrl+wheel 195.3 %, Fit 100.0 % again |
| C18 | Image 1:1 button, Ctrl+0 (Fit), dragging the picture | `viewers_presentation` › image 1:1, Fit and dragging | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C19 | PDF Paged and Continuous layouts | `viewers_controls` › PDF layout; `viewers_scroll` › PDF | PASS (Windows `20261122-135339`, `20261122-104409`; Linux `20261122-110817`) |
| C26 | The layout chosen last is the next PDF's: Continuous in one PDF opens the next Continuous, Paged there opens the first Paged again | `viewers_presentation` › the PDF layout kept for the next PDF | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C27 | Typed zoom: Ctrl+Y and a click on the percentage open a field with the shown zoom; "150", "62.5" and "150%" apply; "0" and "7000" apply 0.01 % and 6400 % with "The smallest zoom is 0.01%" and "The largest zoom is 6400%"; "abc" keeps the field open with "Type a zoom percentage from 0.01% to 6400%, for example 150.5" and changes nothing; Escape closes only the field and keeps the zoom; HTML takes 10 % to 1000 % the same way; a PDF at a typed 400 % | `viewers_presentation` › typed zoom, HTML zoom, typed zoom, policy and Find; `viewers_scroll` › PDF sharp text | PASS (Windows `20261122-152615`, Linux `20261122-110817`; the HTML part SKIP on Linux). The scenario makes the invariant number format the user's first: the machine's own may use a decimal comma |
| C28 | The typed zoom in a decimal-comma number format with space-grouped thousands (made the user's format as the Engine's read of the regional settings delivers it, `viewers_support::use_number_locale`): the field opens with the shown zoom in that format ("62,5" again after "62,5"); "62,5", a dot ("75.5") and grouped thousands ("1 000,5") apply; "0" gives "The smallest zoom is 0,01%" and "abc" "Type a zoom percentage from 0,01% to 6400%, for example 150,5" | `viewers_presentation` › the typed zoom in a decimal-comma number format | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C29 | Viewer choices after a restart on one profile: Wrap, Lines, the Markdown preset, the text font and size, Aa, the image's 1:1 zoom, the PDF's Continuous layout and the comparator's font (saved in the profile, then shown by the next start) | `viewers_presentation` › viewer preferences saved in the profile, after a restart | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C20 | PDF pages: Ctrl+PageDown, Ctrl+PageUp, the page buttons, the counter, nothing before the first or after the last page | `viewers_presentation` › PDF pages | PASS (Windows `20261122-152615`, Linux `20261122-110817`) |
| C21 | HTML zoom: buttons, Ctrl+Plus, Ctrl+0; Online unchecked and JavaScript checked and available offline (web viewer API 0.4); Back and Forward disabled without a page to go to, Reload enabled | `viewers_presentation` › HTML zoom, policy and Find | PASS on Windows (html-browser branch run `20261123-172442`); SKIP on Linux: no writable cgroup-v2 delegation for the browser there |
| C22 | Space and Backspace show the next and previous file; Alt+Right and Alt+Left too; wrapping at the ends | `viewers_controls` › next and previous file | PASS (Windows `20261122-135339`, Linux `20261122-110817`), Alt+Right/Alt+Left and the wrap from the first file to the last included |
| C30 | Space and Backspace load the file they open as it is at that moment: a file changed after the viewer opened shows its new content | `viewers_controls` › Space and Backspace load a file changed after the viewer opened | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| C31 | A folder whose order exceeds 8 MiB of paths (long names, 1.5 times the budget): from the first file Backspace shows the last file of the window the viewer keeps around it, not the folder's last file; from the last file Space shows the window's first file (README: navigation cycles within that window) | `viewers_controls` › Space and Backspace in a folder over 8 MiB of paths | PASS (Windows `20261122-135339`, Linux `20261122-110817`): 42,225 files with 120-character names, 12.0 MiB of order; the window from either end holds 28,149 files |
| C32 | Links in a Markdown document by keyboard: Tab focuses the first link in view, then each next one, with a focus ring around it and its target on the status line ("Link to https://example.com/manual", "Link to mailto:help@example.com?subject=Hello" without its `attach` field, "Link to #the-end in this document", a local path resolved against the document's folder, a refused link's target with the reason); Shift+Tab goes back, and from the first link off the links, with no ring left | `viewers_controls` › links: Tab and Shift+Tab through the links | PASS (Windows `20261122-212918`) |
| C33 | Enter follows the focused web link to the presenter's URL opening (`Command::OpenUrl`), which the harness's Engine refuses here, having no desktop service: the status line says "Could not open https://example.com/manual: …" and nothing is launched, so no browser starts. The desktop stand-in, which records the address (`Launch::OpenUrl`, checked by `harness_checks`), cannot be combined with the plugin packages yet: the panels then start without a folder | `viewers_controls` › links: Enter follows the focused web link | PASS (Windows `20261122-212918`) |
| C34 | The pointer over a link is a pointer (`link-hover`) and the status line says where the link leads; beside the link both are gone | `viewers_controls` › links: the pointer over a link | PASS (Windows `20261122-212918`) |
| C35 | A drag over a link's text selects it (Ctrl+C copies a part of "manual") and follows nothing: no launch, and the status line says no link was followed | `viewers_controls` › links: a drag over a link selects | PASS (Windows `20261122-212918`) |
| C36 | A click on a mail link with an `attach` field: the address handed on has no such field ("Could not open mailto:help@example.com?subject=Hello: …", refused here as in C33), and nothing is launched | `viewers_controls` › links: a click on a mail link | PASS (Windows `20261122-212918`) |
| C37 | Refused links, clicked, each with its reason on the status line and nothing launched: an `.invalid` host ("This link points to an address that nothing can reach, so it is not followed."), `javascript:` ("Links to javascript: addresses are not followed.") and a file URL naming another computer ("Links to files on other computers are not followed.") | `viewers_controls` › links: link 3, 8 and 9 is refused | PASS (Windows `20261122-212918`) |
| C38 | A click on `#the-end` shows its heading about 290 KB further in the same document, beyond the first page the viewer loads, in the same window; Ctrl+Home brings the links back | `viewers_controls` › links: #the-end beyond the loaded page | FAIL on Windows (`20261122-212918`): after the click the status line says "Cannot look for #the-end: Invalid or oversized viewer request" and the view stays at the top (Markdown plugin rebuilt from ad91634e, the other packages of 05710878; finding VIEW-LINK-SCAN below) |
| C39 | A link to a folder opens it in the active panel ("Opened … in the active panel.") | `viewers_controls` › links: a folder opens in the active panel | PASS (Windows `20261122-212918`) |
| C40 | A link to a program (`tool.exe`) shows its folder in the active panel with the cursor on it ("No viewer shows …; it is shown in the active panel."); nothing is run or launched | `viewers_controls` › links: a program is shown in the panel, never run | Pending the final run at the release commit. In `20261122-212918` the fixture was 14 bytes of text, which the policy opens in the viewer as designed (a local file a viewer claims, or whose first 4 KiB hold no NUL, opens in the same window; nothing ran); the fixture is now binary (an `MZ` header with NUL bytes), so the check exercises a file that is shown in the panel |
| C41 | A relative link to another Markdown file opens it in the same window; Alt+Shift+Left and Alt+Shift+Right, and the mouse's Back and Forward buttons, go back to the first document and forward again | `viewers_controls` › links: a relative link and the link history | Pending: not reached in `20261122-212918` (C40 left `tool.exe` in the window, so the relative link was not where Tab had found it); red seen in `20261122-212204` only as that same miss |
| C42 | Links in the HTML viewer, whose browser hands a followed link to the same policy (web viewer API 0.4.0 with `MIDNIGHT_WEB_VIEWER_LINKS=host-v1`, which the host sets): the pointer over a web link shows "Link to https://example.com/page", a click on the web and the mail link reaches the same URL opening (refused here as in C33, the mail address without its `attach` field); a click on the local `next.html` opens it in the window, styled by `../links-styles/next.css` one folder above and by its script offline, with Back enabled; the Back button returns to `page.html` with Forward enabled; a click on a fragment link scrolls the page to its block; a page saved "complete" (`Saved page ž_files`: a stylesheet that imports another, a `.js.download` script) renders with them; nothing is launched | `viewers_controls` › links: the HTML viewer | PASS on Windows (html-browser branch run `20261123-172442`, screenshots `html-next-styled`, `html-saved-page`). SKIP on Linux where the browser cannot start |
| C23 | Space and Backspace in the HTML viewer (Chromium arbitrates the physical keys) | none | SKIP: Chromium arbitrates the physical keys through native window events, which this harness cannot deliver |
| C24 | Escape closes the viewer | every viewer scenario (`Viewing::close`) | PASS (every viewer scenario of these runs) |
| C25 | The window's own close button: the platform's close request (`CloseRequested`) closes the viewer and the comparator | `viewers_controls` › next and previous file (the plain-text viewer closed so); `viewers_compare` › same text in two encodings | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |

## Selection and copy

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| S1 | Ctrl+A selects the whole file and the status line says so; Ctrl+C copies the whole file's text exactly, several pages of it (source without line numbers) | `viewers_selection` › each theme and sample | PENDING: whole-file selection replaced the page's rows after the recorded runs; the check compiles and waits for its first run |
| S2 | Markdown copies the whole document's rendered text (its first and last sections, no markup), the hex view the hex rows it shows | `viewers_selection` › Markdown, hex view | PENDING, as S1 |
| S3 | A mouse drag and a Shift+click select runs of the file's text | `viewers_selection` › mouse selection | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| S4 | Every theme's selection as users see it (Rust source and CSV columns): the selection is painted in the theme's selection colour, every selected glyph (its solid core, right of the line-number gutter) in the theme's selected-text colour, as the viewer window's Theme names them, and that pair keeps the product's minimum: 4.5:1, 7:1 in PaperContrast, SandstoneContrast, DeepOcean and GraphiteContrast, 12:1 in High Contrast Dark and Light | `viewers_selection` › every theme | PASS (Windows `20261122-135339`, Linux `20261122-110817`): 26 themes, each pair above its theme's minimum, the lowest NortonCommander's 5.60:1 (minimum 4.5); in High Contrast Dark and Light the selected text is the page's own colour (#000000 on #1aebff, 14.37:1; #ffffff on #37006e, 15.13:1) |
| S5 | The same for plain text, Markdown, the hex view, Python, JSON, XML, YAML, CSS and SQL in Blue, Dark and Light | `viewers_selection` › representative themes | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| S6 | Unselected text colours keep 5:1 against the page (7:1 in the Contrast and High Contrast palettes), right of the gutter: strictly for a colour covering at least 300 pixels (its cores are solid), flagged for review below that | `viewers_selection` › every theme and sample | PASS (Windows `20261122-135339`, Linux `20261122-110817`); no colour flagged |
| S7 | The strikethrough lines of selected struck Markdown words take the selected-text colour (the Markdown viewer colours links without underlining them, so no bundled viewer draws underlines) | `viewers_selection` › Markdown links and struck text | PASS (Windows `20261122-135339`, Linux `20261122-110817`): 24 strikethrough rows over the selection, all in the selected-text colour (#ffffff in Blue and Dark, #202020 in Light), and in the text colour before it |
| S8 | An inactive viewer window (the system focus moved away through the seam) paints its selection in the theme's dimmer inactive pair, every selected glyph core in the inactive selected-text colour, a pair that keeps the theme's minimum; the active pair returns with the focus (Rust source, every theme) | `viewers_selection` › every theme, Rust source | PASS (Windows `20261122-135339`, Linux `20261122-110817`): in every theme the inactive fill and text as the Theme names them, the lowest Nord's #eceff4 on #484d54 (7.39:1), High Contrast's #000000 on #c8c8c8 (12.55:1); the active pair back with the focus |

## Scrolling

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| R1 | A content taller than the window shows a scrollbar with a visible thumb | `viewers_scroll` › scrollbar shown | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| R2 | Scrollbar drags from 0 % to 100 % (every 1 %) land without an error in the large log, CSV, TSV, CSV as plain text, Markdown with tables | `viewers_scroll` › scrollbar drags land | PASS (Windows `20261122-104409`, Linux `20261122-110817`): every 1 % in all five |
| R3 | The same every 2 % in Rust source, the hex view, a PDF (Continuous) and an image at actual pixels | `viewers_scroll` › scrollbar drags land | PASS (Windows `20261122-104409`, Linux `20261122-110817`): 51 drags each |
| R4 | The horizontal scrollbar of a 200,000-character line | `viewers_scroll` › horizontal scrollbar | PASS (Windows `20261122-104409`, Linux `20261122-110817`) |
| R5 | A Slint pixel delta (the harness's synthetic wheel event, 60 px) moves the content exactly its pixels in every viewer kind: one down 60 px, one up −60 px, a burst of five 300 px, the same in every viewer. This is not the product's notch (R11) | `viewers_scroll` › a Slint pixel delta moves exactly its pixels, the same travel in every viewer | PASS (Windows `20261122-104409`, `20261122-182535`; Linux `20261122-110817`, `20261122-185841`): 60 px down, −60 px up and 300 px for five in all nine viewer kinds with a scrollbar (the HTML viewer: R6) |
| R6 | The HTML viewer and Slint's own wheel event | `viewers_scroll` › HTML Slint pixel delta | SKIP: viewer.slint drops Slint's own wheel event in the HTML mode, so a Slint pixel delta moves the page 0 px; the HTML viewer's native notch is checked in R11 |
| R11 | One native wheel notch (through the seam) at three lines per notch moves 100 px, one up −100 px, five 500 px, in every viewer kind, HTML included; on the large log six lines per notch move 200 px and one screen per notch a screen; a notch glides (three or more position changes) and with reduced motion moves in one step | `viewers_scroll` › one wheel notch moves 100 px; the system's wheel setting and reduced motion | FAIL in the runs on main 5ccd0eb7 and d96dec97 (Windows `20261122-182535`: TSV −15, CSV as plain text −15 px; Linux `20261122-185841`: CSV −42, TSV −29, CSV as plain text −16 px), because of VIEW-NOTCH-UP (below), a defect of the acceptance seam fixed on main ad91634e; not run again here (pending the final run at the release commit). Every other kind 100, −100 and 500 px, the HTML viewer included on Windows, whose glide fell short in two of five Windows runs (87, −72, 364 px in `20261122-065626`; 53, −53, 434 px in `20261122-161039`); a short HTML glide the right way is flagged for review (backlog VIEW-09), not failed. The large log on both systems: six lines per notch 200 px, one screen per notch 748 px, a notch glides over 13 positions, and with reduced motion moves in one step |
| R14 | Source highlighting while a person scrolls: in a C++ file of 150 rows (about 6 KB) twelve wheel notches down and up, eight PageDowns and PageUps and a scrollbar drag never show "Preparing source highlighting…" in any frame's status line, and after each gesture the rows show their keyword, string and comment tints; in one of 56,000 rows (about 2.2 MB) the rows shown after Ctrl+End are coloured within a second and the notice, if shown, is gone once they are | `viewers_scroll` › source highlighting | The small file PASS on main d96dec97 (Windows `20261122-182535`, Linux `20261122-185841`): no "Preparing source highlighting…" in any frame's status line, every tint after each gesture. The large file PASS on Linux (coloured 915 ms after Ctrl+End, no notice) and FAIL on Windows (coloured after 1,975 ms; no notice); both with plugin packages that predate the Source plugin's part of the fix. Red, the product before the fix (main 5ccd0eb7, runs `20261122-115842`, `20261122-161039`): the small file showed "Preparing source highlighting…" while scrolling, and the large file's rows were coloured after 1,808 and 2,080 ms |
| R12 | A PDF shows new frames while its scrollbar is dragged (at least two distinct frames that differ from the page before the drag, over 20 moves 33 ms apart), not only at the release | `viewers_scroll` › PDF, new frames while the scrollbar is dragged | PASS (Windows `20261122-104409`, `20261122-182535`; Linux `20261122-110817`, `20261122-185841`): 20 distinct frames during 20 moves 33 ms apart |
| R13 | PDF text is sharp at a typed 400 %: three quarters of its edges between the white page and solid ink are at most 2 px wide (rendered at the zoom, not an enlarged raster) | `viewers_scroll` › PDF, sharp text at a typed 400 % zoom | PASS (Windows `20261122-104409`, `20261122-182535`; Linux `20261122-110817`, `20261122-185841`): 19,486 text edges, three quarters at most 1 px wide |
| R7 | Down and Up move 28 px; PageDown and PageUp exactly one screen; Ctrl+End and Ctrl+Home reach the ends | `viewers_scroll` › keys | PASS in every viewer kind (Windows `20261122-104409`, `20261122-182535`; Linux `20261122-110817`, `20261122-185841`): Down 28 px, Up −28 px, PageDown and PageUp 712 or 748 px (one screen), twenty Down presses in a row 560 px, Ctrl+End 1.0, Ctrl+Home 0.0. The moves are measured on the pixels, also where a refill drew the text again at another sub-pixel phase (the large log's Slint pixel delta up −60.22 px, its notch up −100.63 px) |
| R8 | HTML keys: Down, PageDown, Ctrl+End, Ctrl+Home (back to the first view, pixel for pixel) | `viewers_scroll` › HTML keys | PASS on Windows (`20261122-104409`: Chromium Down 40 px, PageDown 623 of 712 px); SKIP on Linux: no writable cgroup-v2 delegation for the browser there |
| R9 | Large log: Ctrl+End to the last line, Ctrl+Home, PageDown, the wheel down and back to the first view (followed by the shown position and the pixels: the content's accessible text is the whole prepared page) | `viewers_controls` › keyboard and wheel scrolling | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| R10 | Kinetic (touchpad) coasting | none | SKIP: not written yet; the acceptance seam's touchpad reports (`NativeWheel::Pixels` with their phases) can drive it now |

VIEW-NOTCH-UP (Windows and Linux, 22 November, main 5ccd0eb7 and d96dec97) was a defect of the
acceptance seam, not of the viewer. After a scrollbar position the first notch up handed the text
camera over to a corrected surface (the position's page started inside a row, and the page read
above it showed that row whole), and the window kept the glide's remaining distance until its next
frame. The seam ended its frame pump 250 ms after its last input once nothing moved, also during
that wait, so the rest of the glide never got a frame: one notch up after a notch down rested after
15–42 px (CSV −29 and −15 px, TSV −42 px, CSV as plain text −15 px, the 1 MiB log −30 px, Rust
source −15 px). With winit's frames the application kept the whole travel. Main ad91634e has the
fix: the seam sends a frame after each of its inputs and whenever the window moves, as winit does
(bc78af01), and a scrollbar position's text page starts at a row, so that notch needs no corrected
surface (bcd61454). `viewers_scroll` was not run again for this record; the final run at the
release commit covers R11.

VIEW-LINK-SCAN (Windows, 22 November, main 200589ca, Markdown plugin of ad91634e): in a 288 KB Markdown file (`links.md` of `viewers_controls`: nine links, 4,000 filler paragraphs, then `## The end`) a click on `[the end](#the-end)` shows "Cannot look for #the-end: Invalid or oversized viewer request" and does not scroll; the heading lies beyond the first page (bytes 0–12233).

Test fixture corrected (C40, not a product finding): `tool.exe` of `viewers_controls` was a 14-byte text file, which the link policy rightly opens in the same viewer window (a local file a viewer claims or whose first 4 KiB hold no NUL); it is now binary, an `MZ` header with NUL bytes, as a program's first bytes are.

## File comparison (Ctrl+Shift+C)

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| F1 | Two marked files compared: identical, the same text in two encodings, different | `viewers_compare` › identical, encodings, three differences | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F2 | Difference navigation: N, P, F7, Shift+F7, Alt+Down, Alt+Up, Alt+Right, Alt+Left, Alt+Home, Alt+End, the Next and Previous difference buttons | `viewers_compare` › three differences | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F3 | Find in both files: Find… button, Ctrl+F, Enter, F3, Shift+F3, Shift+Enter, "Match N of M · File · line", Aa, Whole words, Clear Find | `viewers_compare` › Find in the compared files | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F4 | The text font chooser repaints the comparison | `viewers_compare` › Find in the compared files | PASS or SKIP per machine (passes when another fixed-pitch family renders differently): PASS on Windows `20261122-135339` and Linux `20261122-110817` |
| F5 | Scrolling as in the text viewer: an arrow moves 28 logical px, PageDown the whole view, a Slint pixel delta exactly its 60 px, the position bar | `viewers_compare` › scrolling the compared text | PASS (Windows `20261123-211009`: Down 28 px, PageDown and 12 Up 373 px); the earlier row-by-row check passed on Windows `20261122-135339` and Linux `20261122-110817` |
| F15 | One wheel notch in the comparator moves `notch_pixels_for` the system's lines per notch (100 px at three lines), as in the text viewer; an arrow moves 28 logical px and PageDown the whole view | `viewers_compare` | PASS (Windows `20261123-211009`, through the comparator's acceptance seam `midnight_compare_ui::acceptance_seams`: a notch moved 100 px) |
| F6 | Images: Side by side, Swipe, Onion skin; the divider and opacity sliders; dragging the divider; Zoom in and out, Ctrl+wheel, Fit; Close | `viewers_compare` › images | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F7 | Binary pairs in the hex view: summary, navigation keys and button | `viewers_compare` › binary files | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F8 | The Hex button of a text pair shows its bytes and back | `viewers_compare` › text over the former 4 MiB limit | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F9 | Large text: 3.5 MiB and 4.25 MiB logs compared as text | `viewers_compare` › large logs, over the former limit | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F10 | Text over 32 MiB, over 1,000,000 lines, or with a line over 16 KiB is compared as bytes with the reason | `viewers_compare` › over a text limit | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F11 | File A / File B chosen by hand: the resolved side prefilled, a missing path refused in the form (drafts kept), Browse…, Compare, Cancel | `viewers_compare` › File A and File B chosen by hand | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F12 | Archive members: Compare asks for File A and File B (members have no native paths) | `viewers_archives` › Compare files on archive members | PASS |
| F13 | Compare files in a list of search results is refused with the reason | `viewers_combinations` › Find results and a fed list | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| F14 | Escape, Close and the window's close request close the comparator | `viewers_compare` | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |

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
| T1 | Alt+5: every visible tile shows its own image, no flicker while still | `viewers_thumbnails` | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke): 9 of 9 tiles after 6.5 s and 7.6 s. PASS in the Linux run `20261122-140702` (WSL 2, smoke), after the fix (below): 9 of 9 tiles after 235 ms |
| T2 | End and the wheel scroll to tiles that load; Home shows cached tiles at once | `viewers_thumbnails` | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke). PASS in the Linux run `20261122-140702` (WSL 2, smoke), after the fix (below): End shows the last 11 tiles after 740 ms |
| T3 | PNG, JPEG, GIF, WebP and SVG thumbnails | `viewers_thumbnails` › image formats | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke). PASS in the Linux run `20261122-140702` (WSL 2, smoke), after the fix (below) |
| T4 | BMP, ICO and TIFF thumbnails | `viewers_thumbnails` › BMP, ICO and TIFF thumbnails | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke). PASS in the Linux run `20261122-140702` (WSL 2, smoke), after the fix (below) |
| T5 | A PDF thumbnail shows its first page | `viewers_thumbnails` › PDF first page | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke): a white page of 96 x 64 px with 67 dark text pixels. PASS in the Linux run `20261122-140702` (WSL 2, smoke), after the fix (below): the same page |
| T6 | AVIF: libavif's `draw_points_idat.avif` (the image decoder's licensed fixture) looks like its PNG twin | `viewers_thumbnails` › the AVIF thumbnail matches its PNG twin | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| T7 | The JPEG, GIF, WebP and SVG tiles show their images' own colours (grey blocks, palette, checkerboard, the SVG's fills), not a placeholder | `viewers_thumbnails` › the … tile's pixels | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| T8 | A password-protected PDF (RC4, `encode::pdf_encrypted`) keeps its placeholder | `viewers_thumbnails` › a password-protected PDF keeps its placeholder | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| T9 | Two-frame GIF and WebP tiles show the first frame | `viewers_thumbnails` › … shows its first frame | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| T10 | A program's tile shows its own icon (Windows: a copy of `explorer.exe` against a file named `.exe` without icon resources; the harness installs only the desktop service's icon reader, decision A3) | `viewers_thumbnails` › a program's own icon | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); SKIP in the Linux run `20261122-140702` (WSL 2, smoke): Linux tiles show type icons by design |

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
reason shows in the panel's status row instead of a modal notice (the fix below). Since
22 November `viewers_thumbnails` names THUMB-UNKNOWN-STORAGE in its failure when no
thumbnail appears on storage the platform reports as `Unknown`.

Fixed in source on 22 November, both ways (the owner's decision: a local block device
whose kind cannot be proven counts as local for previews; network storage and pending
classifications keep the rule). Rerun on Linux in `20261122-140702` (WSL 2, smoke, with the
packages of b1b89343): every thumbnail check passed, 9 of 9 tiles after 235 ms. The native filesystem plugins report a disk attached to the machine
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
| A11 | One folder of 100,001 files: its listing and Ctrl+Shift+S inside it | `viewers_archives` › flat.tar | PASS on Windows (runs `20261122-053621` and `20261122-114734`, smoke): listed in 805 and 674 ms, Ctrl+Shift+S 5,151 bytes in 1,056 and 1,168 ms; PASS in the Linux runs `20261121-144246` (647 ms, 995 ms) and `20261122-140702` (776 ms, 1,564 ms) |
| A12 | F5 copies a folder of 10,001 files out of a tar, then a Ctrl+A selection of all of them (more than the 4,096 entries a selection once stopped at), byte for byte within 360 s each | `viewers_archives` › F5 of the folder / of a selection of 10,001 … out of copy.tar | PASS on Windows (runs `20261122-053621` and `20261122-114734`, smoke): the folder in 108 and 117 s, the selection in 132 and 131 s; PASS in the Linux run `20261122-140702` (WSL 2, smoke): the folder in 120 s, the selection in 130 s |
| A13 | The same with the 100,001 files of `flat.tar`, within 40 minutes each, with the runner's opt-in stress sizes (`--stress`, `MIDNIGHT_ACCEPTANCE_STRESS=1`) | `viewers_archives` (stress sizes) | NOT RUN YET. Earlier runs at 600 s ran out of time: 59,456 files on Windows (21 November, WIN-8), about 48,470 in the Linux run `20261121-144246` (about 81 files/s) |
| A14 | A quick search, then F3: the search closes and the focused match opens, in a native folder and inside an archive | `viewers_archives` › quick search, then F3 | PASS on Windows (runs `20261122-053621` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |

## Archive passwords

The archives are written by `encode::zip_entries` (ZipCrypto and WinZip AES, checked
against independent readers, see ACCEPTANCE.md); the password is non-ASCII.

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| P1 | Encrypted rows end " · Encrypted" (and only they); the details name the method ("ZipCrypto", "AES-256", …) with its explanation | `viewers_passwords` › … listing and details | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| P2 | F3 on an encrypted member asks "<archive> is password-protected. Enter its password to view <member>."; an empty field gives "Type the password first.", a wrong password "Wrong password — try again."; Escape closes the dialog without a notice, opens nothing and gives the keyboard back to the member's row (U-D28, 08e418d0) | `viewers_passwords` › zipcrypto.zip: F3 asks; empty, wrong, Escape | PASS on Windows (run `20261122-204133`, smoke, at ad91634e). Red run `20261122-204037` (the old notice put back into the product) fails it: each of the three dismissals (Escape after F3, Cancel after F3, Escape at the outer password of an Enter) met the notice within the 500 ms the check watches. Earlier runs (`20261122-040925`, `20261122-114734`, the Linux run `20261122-140702`) checked the notice that the dismissal then gave; not rerun on Linux since |
| P3 | The right password (OK by pointer, Enter by keyboard) opens the viewer; the other members and Alt+F3 then open without asking; leaving and entering the archive asks again; Cancel closes the dialog the same way, without a notice | `viewers_passwords` › the password is asked once per archive | PASS on Windows (run `20261122-204133`, smoke, at ad91634e); the red run `20261122-204037` fails its Cancel. Earlier runs (`20261122-040925`, `20261122-114734`, the Linux run `20261122-140702`) checked the notice that the dismissal then gave; not rerun on Linux since |
| P4 | AES-256 (stored and deflated), AES-128 and AES-192 members; a plain member beside encrypted ones opens without asking | `viewers_passwords` › aes.zip, aes128.zip, aes192.zip | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| P5 | An AES archive inside a ZipCrypto archive: Enter asks for the outer password, F3 inside for the inner archive's own (the outer one is refused there); Escape at the outer password closes the dialog without a notice and keeps the panel | `viewers_passwords` › nested.zip | PASS on Windows (run `20261122-204133`, smoke, at ad91634e); the red run `20261122-204037` fails its Escape. Earlier runs (`20261122-040925`, `20261122-114734`, the Linux run `20261122-140702`) checked the notice that the dismissal then gave; not rerun on Linux since |
| P6 | Members no reader opens say why in their details and ask for nothing: Implode, an unsafe name (`../x`), a link, PKWARE strong encryption | `viewers_passwords` › odd.zip | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| P7 | A 7-Zip archive with encrypted names asks before it lists | `viewers_passwords` › names.7z | SKIP on Windows (runs `20261122-040925` and `20261122-114734`): no `7z` on that machine; SKIP in the Linux run `20261122-140702` (WSL 2, smoke): no `7z` there either |
| P8 | The password reaches neither the saved settings, the clipboard, the harness's event trace nor the staged child's console | `viewers_passwords` › no leaks | PASS on Windows (run `20261122-114734`, smoke): the trace kept as the passwords were answered holds `→ UnlockArchive { …, password: ArchivePassword(<redacted>) }` and no password. Run `20261122-040925` passed it too, but its event-trace part proved nothing then (red runs below); PASS in the Linux run `20261122-140702` (WSL 2, smoke), with the same unlock command in the trace |

## Office documents

The packages are written by `encode::ooxml` with the extra fields Office writes (growth
hints 0xA220, extended timestamps).

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| OF1 | Padded DOCX, PPTX and XLSX render; after Find the status reads exactly "Office preview · Offline · JavaScript off · Match 1 of 1" (before cee7ecd8 a Word document's reply could leave the bare count) | `viewers_office` › padded.… | PASS on Windows (run `20261122-114734`, smoke): exactly that status for each. In run `20261122-040925`, before cee7ecd8, the Word document's status was the bare "Match 1 of 1". In the Linux run `20261122-140702` (WSL 2, smoke) FAIL: the browser was refused, "Chromium requires a writable cgroup-v2 user delegation with memory and pids already enabled" (that run was not started in a delegated scope). Such a machine refusal is now a SKIP with its reason, as in `viewers_files`. PASS in the Linux run `20261122-194133` (WSL 2, smoke, at 42fd1d85, started in a delegated systemd user scope after the shared target's clean step): exactly that status for each |
| OF2 | A package whose parts are encrypted: "This presentation is password-protected, so the preview cannot open it." | `viewers_office` › locked.pptx | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| OF3 | An OLE compound file: "This document is password-protected or saved in the older binary Office format (.doc, .xls or .ppt), which the preview cannot open." | `viewers_office` › protected.docx | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| OF4 | With the HTML viewer disabled in the settings: "Office preview requires an installed and enabled Chromium HTML viewer." | `viewers_office` › without the HTML viewer | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| OF5 | Linux without the browser's system libraries: the viewer names them and the command that installs them (b8537464) | `viewers_office` › the HTML viewer names its missing libraries | Not run: the check runs only where `ldd` finds the browser's libraries missing, and the WSL machine has them since 22 November (`20261122-140702`) |

## Video

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| VD1 | Each video's own tile shows its colour, not the fallback icon | `viewers_video` › thumbnails | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke): 6,902–6,912 pixels of the first colour per tile in both; PASS in the Linux run `20261122-140702` (WSL 2, smoke): 6,912 pixels for plain.mp4 and h264.mp4 after 512 ms |
| VD2 | F3 opens the first frame; a click at 75 % of the position slider shows the later frames, Home on the slider the first again; the Play button plays to the end | `viewers_video` › F3 on … | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke) |
| VD3 | An H.264 + AAC movie through the system's decoders: Windows' encoders and Media Foundation; on Linux the host's `ffmpeg -c:v libx264` and the distribution's FFmpeg (GStreamer is the fallback when FFmpeg is missing, not exercised) | `viewers_video` › F3 on h264.mp4 | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); PASS in the Linux run `20261122-140702` (WSL 2, smoke): `ffmpeg -c:v libx264` made the movie, the distribution's FFmpeg played it |
| VD4 | NTFS-compressed copies (Windows) | `viewers_video` › F3 on packed.mp4, h264-packed.mp4 | PASS on Windows (runs `20261122-040925` and `20261122-114734`, smoke); SKIP in the Linux run `20261122-140702` (WSL 2, smoke) (a Windows attribute) |

Red runs of the checks written on 22 November (T6–T10, A12, A14, P1–P6, P8, OF1–OF4,
VD1–VD4; P7 and OF5 cannot run on that machine), on Windows at smoke scale. Each was built
with deliberately wrong expected values: 27 breaks, at least one per check. Run
`20261122-054316` (increment-1 code at 05710878) failed every broken check, each with its
measured value, except the event-trace part of P8. That part could not fail: the harness
keeps only the last 32 lines of its trace, and by the time the scenario looked, they no
longer held the unlock commands. Since then `submit_password` keeps the trace lines as
each password is answered, and P8 requires an unlock command among them. Run
`20261122-084359` (at 26c18895, `viewers_passwords` only) then failed all ten broken
password checks, P8's event trace among them. A lease fault let that run share the desktop
with another agent's run for its whole length. Each failure still shows its deliberately
wrong value, which contention cannot produce. OF1's exact status, expected since 5ccd0eb7,
has not been run red; a status that is not exactly that string fails it.

The Linux run `20261122-140702` (WSL 2, smoke) ran at 5ccd0eb7 plus increment 2 (the files
of 25f5c7b9), with the shared Linux packages of b1b89343 (filesystem ABI minor 4), after
the shared Cargo target's clean step of every path package; it was not started in a
delegated systemd user scope.

## Combinations

| ID | Inventory item | Scenario › check | Status |
| --- | --- | --- | --- |
| X1 | F3 on a result in the Find window | `viewers_combinations` › Find results and a fed list | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X2 | F3 in results fed to a panel; Space and Backspace follow the results | `viewers_combinations` › Find results and a fed list | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X3 | F3 in an expanded inline tree branch; Space follows the visible file order | `viewers_combinations` › an inline tree branch | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X4 | Four viewer windows at once, each with its own file; a fifth refused with the reason; closing one keeps the others | `viewers_combinations` › four viewer windows | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X5 | A theme previewed in Options recolours an open viewer, its syntax colours (glyph cores of at least 300 pixels, right of the gutter) keep 5:1 against the Light page; Cancel restores it | `viewers_combinations` › a theme previewed in Options | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X6 | A viewer moved to a display of twice the scale shows the same lines twice as tall | `viewers_combinations` › a viewer at twice the scale | PASS (Windows `20261122-135339`, Linux `20261122-110817`) |
| X7 | Viewers on archive members (and inner archives' members) | `viewers_archives` | PASS |
| X8 | Every theme's selection colours | `viewers_selection` | See S4–S8 |

## Not yet written

- The AppArmor Install… offer (O20: reported `SKIP` with the machine's reason), the Make
  File List viewer opened from Ctrl+M (P's area, hooked here), a viewer for every file
  reached from a Find feed of archive members.
- Alt+Left/Right cycling files after link history; the links section on Linux (no Linux
  packages with the Markdown plugin's link targets yet); the address a followed web or mail
  link hands to the desktop service (`Launch::OpenUrl`) in a viewer scenario: the desktop
  stand-in (`Config::desktop`) with the plugin packages starts without a folder in the panels.
- The SVG decoder's note naming what it left out ("Not shown: linked images, scripts.")
  is not shown by any viewer yet (its notice API is deferred), so O21 checks the pixels
  only.
