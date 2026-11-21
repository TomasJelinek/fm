# Acceptance coverage: UI polish gate

The UI polish gate is the acceptance suite's strict, automatic check of how every
window, dialog, popup and state of Midnight Explorer looks and behaves. It renders each
surface with Slint's software renderer, reads the live item tree and the rendered pixels,
and fails on any finding. This record lists every check family, the red case that proves
it can fail, every surface and state with the checks that ran on it, the exemptions with
their reasons, and the product defects the gate found. The suite is described in
[ACCEPTANCE.md](../ACCEPTANCE.md) ("UI review checks"); the code is in
`crates/midnight-acceptance/tests/acceptance/scenarios/ui_*.rs`,
`tests/acceptance/review.rs` and `scenarios/options_visual.rs`.

Status: **PASS** (no flag in the run named below), **FAIL** (a product defect; the
finding is listed under "Product defects"), **PENDING** (the check runs at full strength
but the product change that satisfies it is still in progress; reason given), **SKIP**
(the environment cannot run it; reason given).

Last run: see "Runs" at the end. Statuses are from those runs; a surface marked
"rerun" had flags that the check refinements made after the run (occlusion, captions drawn
by siblings, header rows that scroll with their list, deliberate path elision) no longer
raise, and needs the next run to confirm.

## Scenarios

| Scenario | What it covers | Process |
| --- | --- | --- |
| `ui_polish_checks` | Red cases: every check flags a deliberately broken window and passes its corrected twin. | in process |
| `ui_review` | Default matrix: the main window's states, menus and popups, every embedded dialog and the tool windows, static and contrast checks. | in process |
| `ui_themes` | Every theme the default matrix leaves out: main window states, menus, popups, Copy, an error notice and Find. | in process |
| `ui_polish_dialogs` | Interactive checks of every dialog and tool window; hover and tooltips of the main window. | private child (`run_private`) |
| `ui_polish_jobs` | Copy job windows in every state, the decision windows and Jobs, at preferred and minimum size, with interactive checks. | in process |
| `ui_polish_viewers` | Viewer windows (text, source with a selection, image, Markdown, hex) and a comparator, dark and light. | staged child with plugins |
| `options_visual` | Every Options page in light, dark and a narrow touch layout. | in process |
| `ui_matrix_full` | The full matrix, on request only (`--all-themes` or `MIDNIGHT_ACCEPTANCE_UI_MATRIX=full`); `SKIP` otherwise. | in process |

`--only ui_` runs the gate (`options_visual` is selected by its own name). A flag fails
its scenario whatever `--strict-review` says: the gate scenarios return an error when
their outcome has a flag that is not an exemption (`ui_gate::verdict`). When every flag
belongs to a check marked pending, the scenario reports `PENDING`.

### Matrix

| Matrix | Combinations |
| --- | --- |
| Default (`ui_review`) | Every surface: Blue Desktop 1340×820, Dark Touch 960×600, Light Desktop 1600×1000, HighContrastDark Touch 1340×820, Blue Desktop and Dark Touch at the minimum size (320×200), all at scale 1. Colour scope at scale 2: Light Desktop 1340×820 and HighContrastLight Touch 960×600. |
| Themes (`ui_themes`) | The other 21 themes, Desktop 1340×820 ×1, colour scope. |
| Jobs (`ui_polish_jobs`) | Blue Desktop and Dark Touch; every window at its preferred and its minimum size. |
| Viewers (`ui_polish_viewers`) | Dark Desktop and Light Touch. |
| Full (`ui_matrix_full`) | Every theme × both densities at 1340×820 ×1, plus Blue, Dark, Light, HighContrastDark and HighContrastLight × both densities × {minimum, 960×600, 1340×820, 1600×1000} × {1, 2} (148 combinations, every surface). |

Tool windows are sized per combination from their own layout: the minimum size, three
quarters of the preferred size, the preferred size, or 1.4 times it. Scale 2 renders at
twice the physical size (the logical layout is unchanged).

## Check families

Each check is proven by a red case in `ui_polish_checks`: the broken window must be
flagged and its corrected twin must pass every check.

| ID | Check | Measures | Red case (broken → corrected) |
| --- | --- | --- | --- |
| U-K01 | internal close button in a framed window | An icon-only button labelled Close, Close dialog, Cancel dialog, Dismiss or Decide later in the top band of a tool window whose Window element is framed (no `no-frame`). | Case 1: framed window with a × in its header → the same window frameless. |
| U-K02 | overflow without scroll bar | A Flickable whose content is larger than its viewport shows no scroll bar thumb beside it and does not scroll when the wheel turns over it (the wheel probe scrolls it back). | Case 2: ten notes in a 100 px list, wheel swallowed → a thumb beside it; case 20: no thumb and wheel swallowed → no thumb but the wheel scrolls. |
| U-K03 | content cut off without scrolling | Text or a control reaches more than 1.5 px outside a clipping container that is not a scroll view. | Case 3: four notes in a 40 px clipped box → 120 px. |
| U-K04 | text clipped without ellipsis | A single-line text needs more width than its element and does not elide. | Case 4: `overflow: clip` → `overflow: elide`. |
| U-K05 | wrapped text cut off | Wrapped text needs more height than its element. | Case 6: 20 px high → 140 px. |
| U-K06 | elided text without full text | An elided text whose full text neither its own accessible label, an accessible ancestor's label or description, nor a tooltip provides. | Case 5: `accessible-role: none`, no tooltip → a tooltip with the text. |
| U-K07 | control without accessible name | An interactive control (button, check box, combo box, slider, spin box, tab, text field, switch, radio button) with an empty accessible label. | Case 7: empty label → "Apply". |
| U-K08 | icon-only button without tooltip | A button that shows no text and that no tooltip area covers. | Case 8: no tooltip → a tooltip. |
| U-K09 | overlapping text | The glyph areas of two texts overlap by more than 2 px both ways. | Case 9: two texts drawn over each other → side by side. |
| U-K10 | text outside the window | A text's glyph area reaches more than 1.5 px outside the window. | Case 10: a caption past the right edge → inside. |
| U-K11 | uneven button heights | Text buttons of one row differ in height by more than 1.5 px. | Case 11: 36/30/30 px → 30/30/30 px. |
| U-K12 | uneven button spacing | Gaps of at most 24 px between neighbouring buttons of one row differ by more than 1.5 px. | Case 11: 8 and 16 px gaps → 8 and 8 px. |
| U-K13 | dismiss button not last | In a dialog's footer row, Cancel or Close is followed by another button. | Case 11: Cancel first → Cancel last. |
| U-K14 | low text contrast | WCAG 2.2 AA in the rendered pixels: every visible text against each background colour that covers at least 15 % of its glyph area, 4.5:1, or 3:1 for large text (24 px, or 18.67 px bold) and symbols; selected text in text fields is measured with the selection colours. Text of disabled controls is exempt (WCAG 1.4.3). | Case 12: #9a9a9a on #808080 → #000000; case 13: selected text #b8b8b8 on #c8c8c8 → white on #1a5fb4. |
| U-K15 | surface shows no text | A window, dialog or popup without any visible text. | Case 14: blank → captioned. |
| U-K16 | control clipped by window | (`inspect::check_layout`) an interactive control outside the window. | Case 15: half outside → inside. |
| U-K17 | overlapping controls | (`inspect::check_layout`) two interactive controls overlap by more than 2 px, neither containing the other. | Case 16: overlapping → apart. |
| U-K18 | zero-size label | (`inspect::check_layout`) a visible non-empty text without width or height. | Case 17: width 0 → 140 px. |
| U-K19 | label elided to nothing | (`inspect::check_layout`) a visible text narrower than about one character while it needs more. | Case 18: 6 px → 100 px. |
| U-K20 | text does not fit its button | (`inspect::check_layout`) a button's caption needs more width than it has (paths exempt). | Case 19: 50 px → 140 px. |
| U-K21 | control not reachable with Tab | Tab from the dialog's first focus visits every enabled interactive control (one member of a tab or radio group reaches the group) before the cycle closes. | Interactive case 1: a button that cannot take the focus → it can. |
| U-K22 | Shift+Tab does not reverse Tab | Shift+Tab from the first stop visits the Tab cycle backwards. | Covered by the corrected twins of every interactive case (a wrong order fails them). |
| U-K23 | focus leaves the dialog | Tab moves the focus out of the dialog's popup or window. | Covered by the corrected twins. |
| U-K24 | focus not visible | A control looks the same focused and unfocused (pixels within 4 px of it), or Tab stops on something that is not a control. | Interactive case 2: no focus ring → a ring. |
| U-K25 | focus indicator below 3:1 | The changed pixels of the focused control reach less than 3:1 against their unfocused colour and against the surrounding colour (WCAG 1.4.11). | Interactive case 8: #cfcfcf ring on #d8d8d8 → #1a3fb4. |
| U-K26 | Escape does not close | Escape leaves the dialog open for 3 s. | Interactive case 3: Escape ignored → closes. |
| U-K27 | Enter has no effect | Enter in a freshly opened dialog changes nothing observable. | Interactive case 4: Enter ignored → applies. |
| U-K28 | button has no effect | Clicking a button or tab of a freshly opened dialog changes nothing observable: windows, notices, popups, visible texts, control states, the scratch folders and the panels' folders on disk, commands sent to the backend, file-chooser requests, jobs, the panels, or (where the window is still) its pixels outside the clicked and the focused control. Changes between two quiet samples before the click (progress) do not count. | Interactive case 5: Details does nothing → shows the details. |
| U-K29 | unexpected window or notice | After Escape, Cancel, Close, the header's close or Decide later, a window, notice or popup that was not open before the dialog; after a job ends or its window closes, a notice or another window. | Interactive case 6: Cancel opens a notice → only closes. |
| U-K30 | no hover feedback | Hovering a button, tab, check box, radio button, combo box or switch changes no pixel of it. | Interactive case 7: a button without hover state → with one. |
| U-K31 | tooltip does not show | Hovering an icon-only button that has a tooltip area shows no tooltip text within the area's delay plus 600 ms. | Recorded by the product sweep (the static check U-K08 has the red case). |

## Surfaces and states

Statuses per check family: S = static checks U-K01–K20 and the contrast check U-K14,
I = interactive checks U-K21–K31 (focus, Escape, Enter, buttons, extra surfaces, hover,
tooltips). Combinations are those of the scenario's matrix.

| ID | Surface and state | Scenario | S | I |
| --- | --- | --- | --- | --- |
| U-S01 | Main window, cursor on a file | `ui_review`, `ui_themes`, `ui_polish_dialogs` | FAIL U-D06, U-D09, U-D10, U-D12 | FAIL U-D06, U-D10 |
| U-S02 | Main window, marked rows and the cursor | `ui_review`, `ui_themes` | FAIL U-D09, U-D10, U-D12 | NOT YET WRITTEN (static only) |
| U-S03 | Main window, filename search | `ui_review`, `ui_themes` | FAIL U-D09, U-D10; rerun (occlusion fix) | NOT YET WRITTEN (static only) |
| U-S04 | Main menu (F10) | `ui_review`, `ui_themes` | FAIL U-D02 | NOT YET WRITTEN |
| U-S05 | User menu (F9) | `ui_review`, `ui_themes` | PASS | NOT YET WRITTEN |
| U-S06 | Volume choices (Alt+F1) | `ui_review`, `ui_themes` | PASS | NOT YET WRITTEN |
| U-S07 | Options, every page | `ui_review`, `options_visual`, `ui_polish_dialogs` (General) | FAIL U-D02, U-D19, U-D08 (minimum size) | FAIL U-D04, U-D05 |
| U-S08 | Copy (F5) | `ui_review`, `ui_themes`, `ui_polish_dialogs` | FAIL U-D02, U-D07 | FAIL U-D03, U-D05 |
| U-S09 | Move (F6) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02, U-D07 | FAIL U-D03, U-D05 |
| U-S10 | Delete to Trash (F8) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D03, U-D05 |
| U-S11 | Permanent delete (Shift+F8) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D03, U-D05 |
| U-S12 | Rename (F2) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02, U-D07 | FAIL U-D03, U-D05 |
| U-S13 | Create directory (F7) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D03; Enter rerun (U-D21) |
| U-S14 | New empty file (Shift+F4) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D03; Enter rerun (U-D21) |
| U-S15 | Compare directories (Ctrl+F10) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D04, U-D05, U-D15 |
| U-S16 | Panel filter (Ctrl+F12) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D04, U-D05 |
| U-S17 | Make File List (Ctrl+M) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D04, U-D05 |
| U-S18 | Attributes and its Times page (Ctrl+F2) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02, U-D19 | FAIL U-D04 |
| U-S19 | Hot Paths | `ui_review`, `ui_polish_dialogs` | FAIL U-D02, U-D11 | FAIL U-D04 |
| U-S20 | About, About and Licenses tabs, a license's detail view | `ui_review`, `ui_polish_dialogs` | FAIL U-D02, U-D08 (minimum size) | FAIL U-D04 |
| U-S21 | Drive information (Ctrl+F1) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D04 |
| U-S22 | Properties (Alt+Enter) | `ui_review`, `ui_polish_dialogs` | FAIL U-D02 | FAIL U-D04 |
| U-S23 | Error notice with a long path, details shown | `ui_review`, `ui_themes`, `ui_polish_dialogs` | FAIL U-D02, U-D08 (minimum size) | FAIL U-D04 |
| U-S24 | Find (Alt+F7) | `ui_review`, `ui_themes`, `ui_polish_dialogs` | FAIL U-D16; rerun (header scroll and caption fixes) | FAIL U-D16 |
| U-S25 | Checksums (Ctrl+F8) | `ui_review`, `ui_polish_dialogs` | PASS (after the segmented-button fix; rerun) | FAIL U-D15, U-D17 |
| U-S26 | Help (F1) | `ui_review`, `ui_polish_dialogs` | FAIL U-D01 | FAIL U-D01, U-D04 |
| U-S27 | Jobs, empty | `ui_review`, `ui_polish_dialogs` | PASS | FAIL U-D04, U-D14, U-D18 |
| U-S28 | Copy job paused, details shown | `ui_polish_jobs` | PASS | FAIL U-D04 |
| U-S29 | Copy job waiting for another job | `ui_polish_jobs` | FAIL U-D13 (minimum size, progress part) | not checked |
| U-S30 | Copy job running | `ui_polish_jobs` | PASS | not checked |
| U-S31 | Copy job asking whether to cancel | `ui_polish_jobs` | PASS | not checked |
| U-S32 | Copy job cancelled | `ui_polish_jobs` | PASS | PASS (no notice or extra window after the job ended or its window closed) |
| U-S33 | Copy job completed | `ui_polish_jobs` | closes itself when done | – |
| U-S34 | Copy job completed with errors (3 and 40), details shown | `ui_polish_jobs` | FAIL U-D13 (minimum size, outcome part) | FAIL U-D04 |
| U-S35 | Copy job with notes | `ui_polish_jobs` | NOT YET RUN (opened from Jobs since the last run) | – |
| U-S36 | Name conflict decision window | `ui_polish_jobs` | PASS (no own × any more) | FAIL U-D04 |
| U-S37 | Failure decision window (Retry, Skip, Skip all, Cancel job) | `ui_polish_jobs` | FAIL U-D13 (minimum size, decision window) | FAIL U-D04 |
| U-S38 | Jobs listing finished jobs | `ui_polish_jobs` | PASS | FAIL U-D04, U-D18 |
| U-S39 | Viewer: text, all selected | `ui_polish_viewers` | NOT YET RUN | – |
| U-S40 | Viewer: source, all selected | `ui_polish_viewers` | NOT YET RUN | – |
| U-S41 | Viewer: image | `ui_polish_viewers` | NOT YET RUN | – |
| U-S42 | Viewer: Markdown | `ui_polish_viewers` | NOT YET RUN | – |
| U-S43 | Viewer: hex, all selected | `ui_polish_viewers` | NOT YET RUN | – |
| U-S44 | Comparator: two texts with differences | `ui_polish_viewers` | NOT YET RUN | – |

Not covered yet: the Find window's other pages (advanced options, folders, syntax help,
details, results, Feed to panel), the Checksums window's Verify page and results, the
thumbnail view, inline trees, tabs and the main window's toolbar menus beyond F9, F10
and Alt+F1. The main window gets no Tab/Escape/Enter checks: Tab switches the panels
there by design.

## Exemptions

None. A finding the product cannot avoid would be listed here with its reason (and in
`ui_gate::EXEMPTIONS`, or a dialog's `skip` list for a button that must not be clicked).

## Pending checks

| Surface | Check | Reason |
| --- | --- | --- |
| Decision windows (`…/decision/…`) | U-K01 | The decision windows' own close button is being removed (backend change in progress). |

## Product defects

All reported to the coordinator with their shots (run folders below `_build/acceptance-ui/`).

| ID | Defect | Check | Evidence (run, shot) |
| --- | --- | --- | --- |
| U-D01 | The Help window (a framed native window) shows its own × "Close dialog". | U-K01 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-help.png` |
| U-D02 | The dialogs' header × (CloseButton) and the F10 menu's "Close menu" have no tooltip. | U-K08 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-copy.png` |
| U-D03 | The header × is not reachable with Tab in Copy, Move, Trash, Delete, Rename, Create dir and New file (it is in the other dialogs). | U-K21 | 20261120-144625 `ui_polish_dialogs/dialogs-copy.png` |
| U-D04 | Key-routing wrapper FocusScopes are Tab stops that show no focus (dialogs, Help, Jobs, job and decision windows). | U-K24 | 20261120-144625 `ui_polish_dialogs/dialogs-help.png`; 20261120-143647 `ui_polish_jobs/Blue-Desktop-job-paused.png` |
| U-D05 | Primary and destructive buttons have no hover state (Copy, Move, Rename, Move to Trash, Permanently delete, OK, Compare, Create file list, Options topics). | U-K30 | 20261120-144625 `ui_polish_dialogs/dialogs-copy.png` |
| U-D06 | Main-window controls without hover state: column sort headers, Edit folder path, the drive button, the status bar's free-space button, the folder error icon. | U-K30 | 20261120-144625 `ui_polish_dialogs/dialogs-main.png` |
| U-D07 | Selected text in text fields is #e6f2fb on #0078d4 (3.98:1) in Light and HighContrastLight. | U-K14 | 20261120-143600 `ui_review/Light-Desktop-Large-x2-rename.png` |
| U-D08 | At 320×200: Details on a notice pushes its buttons out and Escape no longer closes it; About's Close is outside its body; Options pages squeeze fields to nothing. | U-K03, U-K26 | 20261120-143600 `ui_review/Blue-Desktop-Min-x1-notice-details.png`, `…-about.png` |
| U-D09 | At 320×200 the function bar shows 4 px of "F7"; at Touch 960×600 the clock has zero width and the permissions button 37 px of its text. | U-K18, U-K19, U-K20 | 20261120-143600 `ui_review/Blue-Desktop-Min-x1-main.png`, `Dark-Touch-Small-x1-main.png` |
| U-D10 | The panel warning row elides a long folder name without tooltip or accessible full text; its "Retry folders in …" button shows no tooltip. | U-K06, U-K31 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-main.png` |
| U-D11 | Hot Paths: "Current folder: <path>" is clipped without an ellipsis. | U-K04 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-hot-paths.png` |
| U-D12 | The panel splitter is a button ("Panel splitter") without tooltip. | U-K08 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-main.png` |
| U-D13 | Job windows at their minimum size: the progress view's speed chart is squeezed and its label cut (progress part); the outcome view's footer squeezes "Copy details" to 53 of 75 px (outcome part); the failure decision window squeezes "Skip all this error type" to 120 of 166 px. | U-K03, U-K20 | 20261120-143647 `ui_polish_jobs/Blue-Desktop-job-waiting-min.png`, `Blue-Desktop-job-errors-few-min.png`, `Dark-Touch-decision-failure-few-min.png` |
| U-D14 | Jobs: Open and Copy details changed nothing observable for the selected finished Permanently delete job (to confirm). | U-K28 | 20261120-144625 `ui_polish_dialogs/dialogs-jobs.png` |
| U-D15 | Enter does nothing in Compare directories and in the Checksums window. | U-K27 | 20261120-144625 `ui_polish_dialogs/dialogs-compare-dirs.png`, `dialogs-checksums.png` |
| U-D16 | Find: the results' column sort headers are not reachable with Tab and have no hover state. | U-K21, U-K30 | 20261120-144625 `ui_polish_dialogs/dialogs-find.png` |
| U-D17 | Checksums: the Calculate / Verify file page buttons show no focus. | U-K24 | 20261120-144625 `ui_polish_dialogs/dialogs-checksums.png` |
| U-D18 | Jobs: the focused job list shows no focus. | U-K24 | 20261120-143647 `ui_polish_jobs/Blue-Desktop-jobs-list.png` |
| U-D19 | Icon-only buttons without tooltip: Attributes › Times date and time pickers, Options › Colors "Choose Foreground/Background" and "Discard changes". | U-K08 | 20261120-143600 `ui_review/Blue-Desktop-Normal-x1-times.png`, `…-options-08.png` |
| U-D20 | The command line's working folder is cut without ellipsis at Touch 960×600 and 320×200 (to confirm with the wheel probe). | U-K03 | 20261120-143600 `ui_review/Dark-Touch-Small-x1-main.png` |
| U-D21 | Create dir / New file: Enter with the empty name gives no feedback (the gate now types a name first; to confirm). | U-K27 | 20261120-144625 `ui_polish_dialogs/dialogs-mkdir.png` |

U-D01–U-D12 were sent in the first batch and U-D13–U-D21 in the second (21 November).

Fixed in source on 21 November, with `midnight-ui` tests that fail on the earlier
source; the gate itself has not been run again:

- U-D05 for primary and destructive action buttons (hover and pressed fills that keep
  each theme's text contrast, `action_button_states`). Hovering the Options topics did
  not reproduce: all nine topics that are not selected, and every drawn control of the
  General page, change on hover in Blue Desktop, Dark Touch and Light (`form_polish`
  keeps the check).
- U-D06 for the column sort headers, Edit folder path, the path's omission marker, the
  free-space button and the folder error icon (`main_window_polish`). The drive button
  did not reproduce in the harness.
- U-D09: the function bar shows whole keys only, and the status strip whole fields
  only. The zero-width clock was the rows' time column, which is now hidden with its
  column instead of drawn 0 px wide.
- U-D10: a shortened name in a row has a tooltip with the full name, and the "Retry
  folders in …" button shows its tooltip.
- U-D12: the panel splitter has a tooltip.
- U-D01: the framed Help window has no × of its own.
- U-D02: the dialogs' header × (and the F10 menu's Close menu) has a tooltip that
  names Escape.
- U-D03: Tab and Shift+Tab reach the header × in Copy, Move, Trash, Delete, Rename,
  Create dir and New file.
- U-D04: the key-routing wrapper FocusScopes of 15 dialogs and windows are no longer Tab
  stops, nor (since the second batch) those of the colour picker, shortcut
  replacement, Select by mask and the main menu.
- U-D07: the Rename, Create, New file, Copy and Move name fields and the conflict
  window's new-name field keep 4.5:1 selected-text contrast in all 26 themes, and so do
  Make File List's destination field and preview and the Compare directories
  tolerance field since the second batch. Since the third batch the Checksums Verify
  page's path field does too: it selected with Fluent's colours (in Blue black on
  #0078d4 instead of the theme's white on #315b86) and is now the themed field, with
  its label, tooltip, height, Enter and typing unchanged (`checksum_verify_field`).
- U-D08, the notice part: at 320×200 shown details scroll with the message, the buttons
  stay in the dialog and Escape closes it. About's Close stays in its body there
  (not reproducible, test added). Since the second batch the Options pages that fit
  their lists to the window (Comparators, Plugins, Keyboard) keep room for six rows at
  320×200 and scroll, instead of squeezing their lists to nothing. Third batch
  (`options_caption_fit`): at 320×200 with fonts of 20–24 px "Show commands containing",
  "No Custom Commands configured.", "Thumbnail size (longest side)" and "No matching
  built-in file types" were cut without "…" (on Windows they needed 296–359 px of
  286 px); they wrap now, as the pages' other captions do, and so do the other
  one-line captions of those pages, which wider fonts would cut too. The leftovers of
  run 20261121-045727 (`minimum_dialogs`, confirmed: About stayed open after the
  gate's walk, click the filter, Down, Enter, Escape; at 320×200 the Licenses tab and
  a library's licence were cut by the content area, 41 px high in Blue Desktop and
  11 px in Dark Touch, and the filter's placeholder needed 200 of 149 px; a notice's
  Copy details showed 61 of its 82 px caption in Dark Touch): a short About has a
  40 px title strip and tighter spacing, its Licenses tab and a library scroll as a
  whole with at least three list rows and four lines of licence, Down scrolls the list
  into view before it takes the keyboard (Slint 1.18.1 drops the focus of a control
  scrolled out of view at the next key, so Enter and Escape reached nothing), Back
  sits beside Close, and a narrow filter's placeholder says "Filter". Hot Paths keeps its
  current-folder line at its height (it was 0 px high there). A notice's Details and
  Copy details show their glyphs only where their captions do not fit beside Close,
  as a job window's actions do.
- U-D11 did not reproduce: the Hot Paths current folder has used middle elision with a
  tooltip since the 20 November integration (test added).
- U-D16: Find's result column headings are Tab stops after the result list, with a
  hover fill and a focus frame (at least 3.77:1 against the header in all 26 themes);
  Enter or Space sorts by the focused heading, which scrolls into view, and the result
  list shows a focus frame. Other keys on a focused heading still reach the list.
- U-D19: the Attributes › Times date and time picker buttons, the Options › Colors
  swatches and Discard changes have tooltips. The arrow cells inside the open calendar
  and time popups have none: those popups close themselves, and Slint 1.18.1 panics
  when a shown tooltip is destroyed inside a popup.
- U-D20, confirmed: the command line's working folder was cut without an ellipsis at
  Dark Touch 960×600, at 320×200 and with a long path at 1340×820. Its start is now drawn
  by the shared path label, which ends it with "..."; the full path stays in the
  tooltip and the accessible label.
- U-D21, confirmed: Enter with an empty name in Create dir, New file, Rename, Copy and
  Move did nothing. The dialog now stays open, moves the keyboard to the name field and
  says "Type the <field> first." under it (also its accessible description) until a
  name is typed.
- U-D13: a finished job's footer leaves out Copy details, then Jobs, when they would
  not fit whole beside the dismiss button (each caption measured in the actions' font;
  Pause and Details count with their longer captions, so toggling them moves no other
  action), and Jobs still offers Copy details: at 320×200 Blue Desktop shows Details,
  Jobs and Close, Dark Touch Details and Close. The waiting label wraps inside the
  speed chart. The failure decision window is never narrower than Retry, Skip and
  Skip all side by side (on Windows 323 px in Blue Desktop, 382 px in Dark Touch); a
  name conflict keeps 320 px. Third batch (`job_footer_glyphs`), confirmed on Windows
  at the window's minimum size (320×200): a running Copy's Details showed 67 of 84 px
  in Blue Desktop at 14 px, 47 of 90 at 16 px and 44 of 93 in Dark Touch; a paused
  one's 76 of 78 already at 12 px, and a job waiting for a decision showed 1 of
  Resolve's 83 px at 12 px; the question's Cancel operation showed 177 of 192 px at
  20 px. The gate did not flag it (it measures the caption against the button's
  visible part). Decision taken: the footer is not stacked; where the actions that
  stay (Pause or Resume, Details, Resolve, or the question's Cancel operation) cannot
  all show their captions beside the dismiss button, they show their glyphs only, each
  with its caption as its accessible label and on its tooltip's first line, Resolve
  with a glyph of its own; Copy details, then Jobs, still leave first. The job window
  is never narrower than Pause, Details and Resolve as glyphs beside Cancel operation
  (wider than 320 px in Dark Touch at 15 px), so they stay whole at its minimum size
  with any font.
- U-D14 did not reproduce: for a finished Permanently delete job, Open shows its
  window (brings it forward, or opens it again after it was closed) and Copy details
  copies the job's details (`jobs_actions`).
- U-D15: in Compare directories Enter compares from any option and from the tolerance
  field (whose own Enter compares, so input methods keep the key); Space still ticks
  an option, Close and the header × keep their Enter, and a held Enter or one during a
  comparison does nothing. In Checksums, Enter in the results or on the page runs
  Calculate or Verify under the footer button's conditions, but not from Details or
  while a save question waits; fields, buttons, the page chooser and the algorithm
  menu keep their own Enter.
- U-D17: the Checksums page buttons and results list show focus, at least 3:1 in Blue,
  Dark, Light, HighContrastDark and HighContrastLight. Every dialog tab (also in
  Options › Plugins and Attributes) shows focus with a 2 px frame.
- U-D18: the focused Jobs list shows a 2 px ring over its frame, at least 3:1 in the
  same themes.
- U-D05, third batch (`hover_feedback`), confirmed: an unchecked WrappingCheckBox and
  the Options › Colors swatches changed no pixel when hovered. A check box now answers
  the pointer on its box or caption with a 2 px active frame around the box (checked
  ones too), and a swatch with a 2 px active-frame ring around it; only the frame
  changes, and in all 26 themes the active frame stands out from the dialog surface
  at least as much as the resting frame. A swatch's own border showed no change in
  HighContrastDark (the foreground swatch has the frame's colour), hence the ring
  outside it.
- Path bar omission marker in Network, third batch (`network_path_omission`),
  confirmed: the marker's tooltip showed the location's own caption ("SERVER-0042"),
  which is all a Network path editor holds. It now names what it omits: the chain from
  the outermost location through the last omitted ancestor ("Network › Microsoft
  Windows Network › A rather long workgroup name 文档"). On native and archive paths it
  still shows the whole path. The last omitted part of each run now draws the marker,
  which keeps every layout as before (`path_elision`, `archive_path_bar`).
- WIN-3 of [windows.md](windows.md), confirmed (`path_bar_fit`): every drawn path
  component's caption is rounded up to a whole pixel while the bar measured joined
  captions, so where a long path just fitted, "Edit folder path" reached up to 7 px
  out of the bar's clip on Windows (6 px in the run). The fitting now keeps a pixel
  per component in reserve.
- WIN-7 (the find record's F-L04), confirmed (`find_compact_status`): in a Find window
  under 360 px high with text over 20 px the idle status was the last child of the
  scrolled query form and showed 0 of its 26–42 px. It now takes the query area's last
  line below the scrolled form, and the query area grows by that line.
- WIN-2, confirmed (`command_path_overlay`): the command line's scrolled working folder
  lay under the folder's start drawn over the strip (overlaps of 96×40, 21×22 and
  15×32 px at the flagged sizes), and at Dark Touch 320×200 that start showed 15 px of
  its text. The strip is now cut where the start ends (outside the strip, which keeps
  its place, size and scrolling, so the start takes the strip's wheel, drag and hint);
  the start is at least two characters wide, or left out where that and a character of
  the end do not fit.
- Jobs rows (windows.md, "Also seen"), confirmed: a row counted in the plural whatever
  the number ("1 files · 0 skipped · 1 errors"). Copy, Move and Delete rows, and the
  counts in a transfer's details, now use the singular for one ("1 file · 0 skipped ·
  1 error", "1 / 1 target · 1 error", "1 temporary source location needs attention";
  `job_presentation` unit test `job_rows_and_details_count_one_in_the_singular`).

The U-D01–U-D04, U-D07, U-D08 and U-D11 results come from `dialog_polish`, whose
tests failed on the earlier dialogs except About's and Hot Paths'. The second batch's
U-D16 result comes from `find_polish` and its U-D04, U-D05, U-D07, U-D08 and
U-D19–U-D21 results from `form_polish`: 14 of their 15 tests failed on the earlier
source; the Options topics check passes on both. The U-D13–U-D15, U-D17 and U-D18
results come from `tool_window_polish` and `native_job_windows` (`minimum_size`,
`jobs_actions`): 8 of their 9 tests failed on the earlier source; the U-D14 one
passes on both. The third batch's results come from `job_footer_glyphs`,
`checksum_verify_field`, `options_caption_fit`, `hover_feedback`,
`network_path_omission`, `path_bar_fit`, `find_compact_status`, `command_path_overlay`
and `minimum_dialogs`, and the `job_presentation` unit test above: 16 of their 20
tests failed on the earlier source; the other four guard what must stay (captions
where they fit, the Verify field's label, tooltip, height and keys, the hover looks'
contrast, the native and archive markers' tooltips). The third batch's tests ran on
Windows only (WSL was unavailable), so none of them ran on Linux. The first two
batches' tests are in the `midnight-ui` `integration` target, which passed in CI's
Linux check job: `dialog_polish` (10 tests) in runs 44851212423 (`985c8ec0`) and
44865587302 (`3d42cb1f`), and the second batch's `find_polish` (4), `form_polish`
(11), `tool_window_polish` (5) and `native_job_windows` `minimum_size` (3) and
`jobs_actions` (1) in run 44865587302. The layout tests measure with the platform's own
fonts, and the captions they check wrap, elide or shrink to glyphs by design rather
than by Windows metrics. The gate has not been run again on Linux since any batch
(the Windows gate runs are in [windows.md](windows.md#results)), so the surfaces
above keep their recorded results until it is.

## Runs

Host runs of `_build/coord-final/debug/deps/acceptance-<hash>` (a copy, so staged
children keep their executable), `--scale smoke`, plugins from a complete plugin build:

| Run | Scenario | Result | Time |
| --- | --- | --- | --- |
| 20261120-143219 | `ui_polish_checks` | PASS: 20 static and 8 interactive red cases flagged, corrected twins clean | 255 s |
| 20261120-152113 | `ui_polish_checks` (after the check refinements) | PASS | 277 s |
| 20261120-143600 | `ui_review` (former 7-combination matrix) | FAIL: 466 flags (U-D01–U-D13, U-D19, U-D20 and check false positives fixed since) | 1924 s, of which about 11 min were 60 s waits at 320×200 (now 15 s) |
| 20261120-143647 | `ui_polish_jobs` | FAIL: 45 flags (U-D04, U-D13, U-D18; path-label elision fixed since) | 249 s |
| 20261120-144625 | `ui_polish_dialogs` | FAIL: 115 flags (U-D02–U-D06, U-D10, U-D14–U-D17, U-D21) | 901 s |

Not run on Linux yet: `ui_themes`, `ui_polish_viewers`, `options_visual` in its gate
form, and `ui_matrix_full` (estimated 2.5 h). On Windows `ui_themes` and
`options_visual` passed, `ui_polish_viewers` failed (WIN-9) and `ui_matrix_full`
reported SKIP (it runs only with `--all-themes`)
([Windows results](windows.md#results)). Expected default gate time after the matrix
rebalance and fail-fast waits: about 45 minutes in all (`ui_review` 10–12 min,
`ui_themes` about 6 min, `ui_polish_dialogs` 15 min, `ui_polish_jobs` 4 min,
`ui_polish_viewers` 5 min, `ui_polish_checks` 4 min, `options_visual` 2 min).
