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
| `ui_polish_dialogs` | Interactive checks of every dialog and tool window, every Options page, the colour picker, the Replace shortcut question, Select by mask and Change name case; hover and tooltips of the main window. | private child (`run_private`) |
| `ui_polish_jobs` | Copy job windows in every state, the decision windows and Jobs, at preferred and minimum size, with interactive checks. | in process |
| `ui_polish_viewers` | Viewer windows (text, source with a selection, image, Markdown, hex), a comparator and the archive password dialog, dark and light. | staged child with plugins |
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
| Full (`ui_matrix_full`) | Every theme × both densities at 1340×820 ×1, plus Blue, Dark, Light, HighContrastDark and HighContrastLight × both densities × {minimum, 960×600, 1340×820, 1600×1000} × {1, 2} (122 combinations: 52 plus 70, every surface). |

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
| U-K14 | low text contrast | WCAG 2.2 AA in the rendered pixels: every visible text against each background colour that covers at least 15 % of its glyph area, 4.5:1, or 3:1 for large text (24 px, or 18.67 px bold) and symbols; selected text in text fields is measured with the selection colours, and on its selection it needs the palette's own minimum at any size, as the product's palettes promise it (`text_minimum` in `crates/midnight-ui/tests/themes.rs`): 7:1 in the contrast palettes (Paper Contrast, Sandstone Contrast, Deep Ocean, Graphite Contrast), 12:1 in High Contrast Dark and Light, 4.5:1 otherwise (`ui_gate::selected_text_minimum`, read from the application's theme at each capture). Text of disabled controls is exempt (WCAG 1.4.3). | Case 12: #9a9a9a on #808080 → #000000; case 13: selected text #b8b8b8 on #c8c8c8 → white on #1a5fb4; case 21: selected white on #1a5fb4 (6.3:1) passes at 4.5:1 (the control) and is flagged at 7:1 and 12:1 → white on #001a40 (17:1). |
| U-K15 | surface shows no text | A window, dialog or popup without any visible text. | Case 14: blank → captioned. |
| U-K16 | control clipped by window | (`inspect::check_layout`) an interactive control outside the window. | Case 15: half outside → inside. |
| U-K17 | overlapping controls | (`inspect::check_layout`) two interactive controls overlap by more than 2 px, neither containing the other. | Case 16: overlapping → apart. |
| U-K18 | zero-size label | (`inspect::check_layout`) a visible non-empty text without width or height. | Case 17: width 0 → 140 px. |
| U-K19 | label elided to nothing | (`inspect::check_layout`) a visible text narrower than about one character while it needs more. | Case 18: 6 px → 100 px. |
| U-K20 | text does not fit its button | (`inspect::check_layout`) a button's caption needs more width than it has (paths exempt). | Case 19: 50 px → 140 px. |
| U-K21 | control not reachable with Tab | Tab from the dialog's first focus visits every enabled interactive control (one member of a tab or radio group reaches the group) before the cycle closes. | Interactive case 1: a button that cannot take the focus → it can. |
| U-K22 | Shift+Tab does not reverse Tab | Shift+Tab from the first stop visits the Tab cycle backwards. | Interactive case 9: Shift+Tab moves forwards like Tab → reverses it. |
| U-K23 | focus leaves the dialog | Tab moves the focus out of the dialog's popup or window. | Interactive case 10: Tab from the last button moves the keyboard into another popup → stays in the dialog. |
| U-K24 | focus not visible | A control looks the same focused and unfocused (pixels within 4 px of it), or Tab stops on something that is not a control. | Interactive case 2: no focus ring → a ring. |
| U-K25 | focus indicator below 3:1 | The changed pixels of the focused control reach less than 3:1 against their unfocused colour and against the surrounding colour (WCAG 1.4.11). | Interactive case 8: #cfcfcf ring on #d8d8d8 → #1a3fb4. |
| U-K26 | Escape does not close | Escape leaves the dialog open for 3 s. | Interactive case 3: Escape ignored → closes. |
| U-K27 | Enter has no effect | Enter in a freshly opened dialog changes nothing observable. | Interactive case 4: Enter ignored → applies. |
| U-K28 | button has no effect | Clicking a button or tab of a freshly opened dialog changes nothing observable: windows, notices, popups, visible texts, control states, the scratch folders and the panels' folders on disk, commands sent to the backend (not the harness's own acknowledgement of a visible listing, `PaneCommitted`, which follows any refresh), file-chooser requests, jobs, the panels, or (where the window is still) its pixels outside the clicked and the focused control. Changes between two quiet samples before the click (progress) do not count. | Interactive case 5: Details does nothing → shows the details. |
| U-K29 | unexpected window or notice | After Escape, Cancel, Close, the header's close or Decide later, a window, notice or popup that was not open before the dialog; after a job ends or its window closes, a notice or another window. | Interactive case 6: Cancel opens a notice → only closes. |
| U-K30 | no hover feedback | Hovering a button, tab, check box, radio button, combo box or switch changes no pixel of it, judged against two frames taken 180 ms apart just before it is hovered: pixels that change between those two (a caret, progress) do not count. | Interactive case 7: a button without hover state → with one. |
| U-K31 | tooltip does not show | Hovering an icon-only button that has a tooltip area shows no tooltip text within the area's delay plus 600 ms (the texts of custom tooltip content and the styled text of a `Tooltip { text: … }` count). | Interactive case 11: an icon-only button whose tooltip area shows no text → one whose tooltip names its action. |

## Surfaces and states

Statuses per check family: S = static checks U-K01–K20 and the contrast check U-K14,
I = interactive checks U-K21–K31 (focus, Escape, Enter, buttons, extra surfaces, hover,
tooltips). Combinations are those of the scenario's matrix.

| ID | Surface and state | Scenario | S | I |
| --- | --- | --- | --- | --- |
| U-S01 | Main window, cursor on a file | `ui_review`, `ui_themes`, `ui_polish_dialogs` | PASS (Linux; the "/review" path segment flagged at Dark Touch 320×200 was a gate defect, fixed by the Linux harness patch, run `20261121-170455`) | PASS |
| U-S02 | Main window, marked rows and the cursor | `ui_review`, `ui_themes` | PASS (Linux; the same path-segment gate defect as U-S01) | NOT YET WRITTEN (static only) |
| U-S03 | Main window, filename search | `ui_review`, `ui_themes` | PASS (Linux) | NOT YET WRITTEN (static only) |
| U-S04 | Main menu (F10) | `ui_review`, `ui_themes` | PASS (Linux) | NOT YET WRITTEN |
| U-S05 | User menu (F9) | `ui_review`, `ui_themes` | PASS | NOT YET WRITTEN |
| U-S06 | Volume choices (Alt+F1) | `ui_review`, `ui_themes` | PASS | NOT YET WRITTEN |
| U-S07 | Options, every page | `ui_review`, `options_visual`, `ui_polish_dialogs` (General) | PASS (Linux) | PASS (Linux) |
| U-S08 | Copy (F5) | `ui_review`, `ui_themes`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S09 | Move (F6) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S10 | Delete to Trash (F8) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S11 | Permanent delete (Shift+F8) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S12 | Rename (F2) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S13 | Create directory (F7) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S14 | New empty file (Shift+F4) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S15 | Compare directories (Ctrl+F10) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S16 | Panel filter (Ctrl+F12) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S17 | Make File List (Ctrl+M) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S18 | Attributes and its Times page (Ctrl+F2) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S19 | Hot Paths | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S20 | About, About and Licenses tabs, a license's detail view | `ui_review`, `ui_polish_dialogs` | PASS (Linux, 320×200 included) | PASS (Linux) |
| U-S21 | Drive information (Ctrl+F1) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S22 | Properties (Alt+Enter) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S23 | Error notice with a long path, details shown | `ui_review`, `ui_themes`, `ui_polish_dialogs` | PASS (Linux, 320×200 included) | PASS (Linux) |
| U-S24 | Find (Alt+F7) | `ui_review`, `ui_themes`, `ui_polish_dialogs` | PASS (Windows and Linux, 320×200 included: U-D22 and the "Path" heading flag of the 21 November runs are gone) | PASS (Linux, Windows) |
| U-S25 | Checksums (Ctrl+F8) | `ui_review`, `ui_polish_dialogs` | FAIL U-D27 (Dark Touch 320×200: the empty-list text overlaps the column headings; Windows and Linux); U-D22 is gone on Windows | PASS (Linux, Windows) |
| U-S26 | Help (F1) | `ui_review`, `ui_polish_dialogs` | PASS (Linux) | PASS (Linux) |
| U-S27 | Jobs, empty | `ui_review`, `ui_polish_dialogs` | PASS | PASS (Linux, run `20261121-164146` with the Linux harness patch; in run `20261121-144246` the gate clicked the disabled Clear all finished, a gate defect) |
| U-S28 | Copy job paused, details shown | `ui_polish_jobs` | PASS | PASS (Linux) |
| U-S29 | Copy job waiting for another job | `ui_polish_jobs` | PASS (Linux, minimum size included) | not checked |
| U-S30 | Copy job running | `ui_polish_jobs` | PASS | not checked |
| U-S31 | Copy job asking whether to cancel | `ui_polish_jobs` | PASS (Linux: Blue Desktop in run `20261121-144246`; Dark Touch not reached, see "Runs") | not checked |
| U-S32 | Copy job cancelled | `ui_polish_jobs` | PASS (Linux: Blue Desktop; Dark Touch not reached) | PASS (no notice or extra window after the job ended or its window closed) |
| U-S33 | Copy job completed | `ui_polish_jobs` | closes itself when done | – |
| U-S34 | Copy job completed with errors (3 and 40), details shown | `ui_polish_jobs` | PASS (Linux, minimum size included) | PASS (Linux) |
| U-S35 | Copy job with notes | `ui_polish_jobs` | PASS (Linux; Windows with a dangling directory junction, Links chosen in the open list with the keys) | – |
| U-S36 | Name conflict decision window | `ui_polish_jobs` | PASS (no own × any more) | PASS (Linux) |
| U-S37 | Failure decision window (Retry, Skip, Skip all, Cancel job) | `ui_polish_jobs` | PASS (Linux, minimum size included) | PASS (Linux) |
| U-S38 | Jobs listing finished jobs | `ui_polish_jobs` | PASS | PASS (Linux) |
| U-S39 | Viewer: text, all selected | `ui_polish_viewers` | PASS (Linux) | – |
| U-S40 | Viewer: source, all selected | `ui_polish_viewers` | PASS (Linux) | – |
| U-S41 | Viewer: image | `ui_polish_viewers` | PASS (Linux; WIN-9's zoom buttons have their tooltips) | – |
| U-S42 | Viewer: Markdown | `ui_polish_viewers` | PASS (Linux) | – |
| U-S43 | Viewer: hex, all selected | `ui_polish_viewers` | PASS (Linux) | – |
| U-S44 | Comparator: two texts with differences | `ui_polish_viewers` | PASS (Linux) | – |
| U-S45 | Main menu (F10), each other category (Files, Selection, Panels and navigation, Tools, Application) | `ui_review`, `ui_themes` | PASS (Windows, Linux) | – |
| U-S46 | The left panel's View and Sort menus | `ui_review`, `ui_themes` | FAIL U-D27 (Dark Touch 320×200: the Sort menu reaches below the window); PASS at every other combination (Windows, Linux) | – |
| U-S47 | Copy (F5) with its Links list open | `ui_review` | PASS (Windows, 320×200 included; Linux) | – |
| U-S48 | Attributes › Times: the Modified calendar and time picker | `ui_review` | FAIL U-D27 (Dark Touch 320×200: a "Now" caption elided under them); the arrows are exempt (see "Exemptions"); PASS at every other combination (Windows, Linux) | – |
| U-S49 | Change name case (Ctrl+F7) | `ui_review`, `ui_polish_dialogs` | PASS (Windows, Linux) | PASS (Windows, Linux) |
| U-S50 | Select by mask (Ctrl+=) | `ui_review`, `ui_polish_dialogs` | FAIL U-D27 (Dark Touch 320×200: its footer is cut); PASS at every other combination (Windows, Linux) | PASS (Windows, Linux) |
| U-S51 | Options › Colors: the colour picker | `ui_review`, `ui_polish_dialogs` | FAIL U-D23 (its swatches have no tooltip; Windows, Linux) | FAIL U-D23 (no tooltip, no hover feedback on the swatches; Windows, Linux) |
| U-S52 | Options › Keyboard: the Replace shortcut question and the Restore all defaults confirmation | `ui_review`, `ui_polish_dialogs` (Replace shortcut) | PASS (Windows, Linux; with the gate fix for surfaces wholly under another one) | PASS (Windows, Linux) |
| U-S53 | Find and Checksums: the More… list of a footer too narrow for its actions (at the windows' minimum size) | `ui_review` | PASS (Windows, Linux) | – |
| U-S54 | Custom Commands: the toolbar (every main-window capture now has two seeded commands) and, in a compact window, its "Custom commands" popup | `ui_review`, `ui_themes` | PASS for the toolbar (Windows, Linux); the popup SKIP: no size of the default matrix shows its button (see "Not covered yet") | – |
| U-S55 | Options, every page interactively (General's surface also clicks the page list and the footer; the other pages skip them, as the same controls) | `ui_polish_dialogs` | – | FAIL U-D25 (Archives), U-D26 (Keyboard), U-D24 (General: raised in both Linux runs and in Windows run `20261122-060309`, not in Windows runs `20261122-092401` and `20261122-124508`); PASS on Appearance, Colors, Previews, Editors, Comparators, Custom Commands and Plugins (Windows, Linux) |
| U-S56 | Archive password dialog: F3 on the encrypted member of a ZipCrypto archive (the gate never types its password) | `ui_polish_viewers` | PASS (Windows, Linux) | FAIL U-D28 (a notice after every dismissal; Windows, Linux) |

Not covered yet: the Find window's other pages (advanced options, folders, syntax help,
details, results, Feed to panel), the Checksums window's Verify page and results, the
thumbnail view, inline trees, tabs, the toolbar's other menus (Back/Forward history,
Hot Paths, drives), the Links reason and warning lines (they need a FAT, removable or
network destination), interactive checks of the calendar and time picker (each day
cell is a button: clicking all of them is left to `panels_times`), and the compact
window's Custom commands popup (no size of the default matrix shows its button: the
wider windows show the commands in the toolbar, and at 320×200 the toolbar clips the
button away, which the sweep records as SKIP). The main window gets
no Tab/Escape/Enter checks: Tab switches the panels there by design.

## Exemptions

A finding the product cannot avoid is listed here with its reason (and in
`ui_gate::EXEMPTIONS`, or a dialog's `skip` list for a button that must not be clicked).
An exempt finding is recorded as a note ("exempt (reason): …"), not as a flag.

| Surface | Check | Finding | Reason |
| --- | --- | --- | --- |
| Attributes › Times › the calendar (`times-calendar`) | U-K08 | "Previous month" and "Next month" show no text and have no tooltip | These popups close themselves, and Slint 1.18.1 panics when a shown tooltip is destroyed inside a popup (U-D19); the arrows' accessible names say what they do. |
| Attributes › Times › the time picker (`times-clock`) | U-K08 | "Increase …" and "Decrease …" (each column's arrows) show no text and have no tooltip | The same as the calendar's arrows. |

## Pending checks

None (`ui_gate::PENDING` is empty). The decision windows' U-K01 mark was removed on
22 November: their own close button is gone (no U-K01 flag on Linux in run
`20261121-144246`), so a × of their own fails the gate again.

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
| U-D22 | Found on Linux (run `20261121-144246`, again in `20261121-164146` and `20261121-170455`): at the minimum size (320×200) a footer action sits half outside the footer's scroller, under the dismiss button, although `DialogFooter` says a dialog leaves out an optional action that would not fit "so that no button sits half outside the scroller": Checksums' "Save…" shows 36 of its 42 px caption (Blue Desktop; `checksum-window.slint` keeps it below 460 px as "Save…"), and Find's View shows a sliver beside Close, which the gate reads as uneven spacing ("Find\|Go to file 8px", "Go to file\|Close 19px", Dark Touch; at Blue Desktop "Vi" shows). Not raised since be086db9 (fitted footers with More…): Windows runs `20261122-060309` and `20261122-092401`, Linux run `20261122-073609`. | U-K20, U-K12 | `_build/acceptance-linux/20261121-144246/shots/ui_review/Blue-Desktop-Min-x1-checksums.png`, `Dark-Touch-Min-x1-find.png`, `Blue-Desktop-Min-x1-find.png` |
| U-D23 | Options › Colors, the colour picker: its eight palette swatches (accessible names "RGB 0, 0, 0" … "RGB 168, 85, 247") show no text and have no tooltip, and change no pixel when hovered. The picker is a popup: tooltips there meet the Slint 1.18.1 constraint noted under U-D19, the hover state does not. | U-K08, U-K30 | Windows `20261122-060309`, Linux `20261122-073609` `ui_review/Blue-Desktop-Normal-x1-options-08-picker.png` (every default combination), `ui_polish_dialogs/dialogs-options-colour-picker.png` |
| U-D24 | Options: "Reset to defaults" is enabled and changes nothing observable while every page already shows its defaults (the class of finding O-4 in [options-tools.md](options-tools.md#findings)). | U-K28 | Windows `20261122-060309`, Linux `20261122-073609` `ui_polish_dialogs/dialogs-options.png` |
| U-D25 | Options › Archives: Tab from the page never focuses part of the footer: "Import configuration…" and "OK" on Windows, also "Reset to defaults" and "Export configuration…" on Linux (the other pages' Tab cycles reach them). | U-K21 | Windows `20261122-060309`, Linux `20261122-073609` `ui_polish_dialogs/dialogs-options-archives.png` |
| U-D26 | Options › Keyboard: the command list (619×146 at 1340×820) is a Tab stop without a role that shows no focus; "Restore all defaults" (and on Linux "Restore command") looks the same focused and unfocused; and Tab from the page's stops moves the keyboard focus out of the Options dialog. | U-K24, U-K23 | Windows `20261122-060309`, Linux `20261122-073609` `ui_polish_dialogs/dialogs-options-keyboard.png` |
| U-D27 | At the minimum size (320×200) in Dark Touch: the left panel's Sort menu reaches 4 px below the window ("Reverse order" at 164 px, 40 px high); Select by mask ("Select files") cuts its footer (Cancel reaches 29 px outside the dialog on Windows, 24 px on Linux; the dialog does not scroll); Checksums' empty-list text ("Choose the checksums in the list above, then press Calculate.") overlaps its column headings "File / folder" and "Bytes read"; on Linux, Attributes › Times scrolled to Modified shows 11 of the 33 px of a "Now" caption under the open calendar and time picker. | U-K16, U-K03, U-K09, U-K19 | Windows `20261122-060309` `ui_review/Dark-Touch-Min-x1-panel-sort-menu.png`, `Dark-Touch-Min-x1-select-mask.png`, `Dark-Touch-Min-x1-checksums.png`; Linux `20261122-073609` the same shots and `Dark-Touch-Min-x1-times-calendar.png`, `…-times-clock.png` |
| U-D28 | The archive password dialog: after Escape, Cancel or the header's Cancel dialog, a notice opens ("Archive member: secret.txt is password-protected; it was not opened, since no password was entered"), although a dismissal opens no further window or notice; since dbddf711 the product reports background failures in status rows instead of notices. | U-K29 | Linux `20261122-123625`, Windows `20261122-124508` `ui_polish_viewers/Dark-Desktop-password.png`, `Light-Touch-password.png` |

U-D23–U-D28 were found on 22 November; U-D23–U-D27 are assigned: U-D23–U-D26 to the Options work, U-D27 to the UI layout work; U-D28 is reported. The gate keeps failing on them (no exemption) until their fixes land and a rerun confirms them.

U-D01–U-D12 were sent in the first batch and U-D13–U-D21 in the second (21 November).
The Linux gate runs of 21 November (below) raised none of U-D01–U-D21, nor WIN-2, WIN-3
or WIN-9 of [windows.md](windows.md): the fixes listed next hold on Linux at every
combination of the default matrix and in `ui_themes`, `ui_polish_dialogs`,
`ui_polish_jobs`, `ui_polish_viewers` and `options_visual`.

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
than by Windows metrics. The gate ran again on Linux on 21 November with all three batches in the build
(commit 3f6fff8a; "Runs" below; the Windows gate runs are in
[windows.md](windows.md#results)), and the surfaces above carry those results.

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

Linux runs of 21 November: commit 3f6fff8a, WSL 2 (Ubuntu 24.04), smoke scale, debug
test build, every plugin package built from that commit, the whole suite in one run
(reports in `_build/acceptance-linux/<run>/`). Runs `20261121-164146` and
`20261121-170455` add the Linux harness patch (committed with this record); `20261121-170455` also seeded network previews on for its
thumbnail experiment, which no gate scenario uses.

| Run | Scenario | Result | Time |
| --- | --- | --- | --- |
| 20261121-144246 | `ui_polish_checks` | PASS: every red case flagged, corrected twins clean | 189 s |
| 20261121-144246 | `ui_review` | FAIL: 5 flags, all at 320×200: U-D22 (2: Checksums' "Save…", Find's footer spacing), the Find results' "Path" heading cut where the table scrolls sideways (gate defect; the harness owner's pending change exempts it), and the "/review" path segment at Dark Touch on `main` and `main-marked` (gate defect: its path exemption missed the segment's bidirectional isolates) | 250 s |
| 20261121-144246 | `ui_themes` | PASS (210 screenshots) | 271 s |
| 20261121-144246 | `ui_polish_dialogs` | FAIL: 1 flag, Jobs' Clear all finished "changes nothing observable": a gate defect (with every finished job clean, Clear finished without errors empties the list, and the reopened Jobs rightly disables Clear all finished, which the gate still clicked) | 609 s |
| 20261121-144246 | `ui_polish_jobs` | FAIL: 1 flag, "job step failed: Dark-Touch/long copy": Slint panicked (`i-slint-core` 1.18.1 `window.rs:781`, `component()` of a dropped component) when the harness used the copy's window after the 1–2 MiB copy had ended and closed it; a harness race, not a product result | 173 s |
| 20261121-144246 | `ui_polish_viewers` | PASS (WIN-9 fixed) | 33 s |
| 20261121-144246 | `options_visual` | PASS | 29 s |
| 20261121-144246 | `ui_matrix_full` | SKIP: runs with `--all-themes` only | 0 s |
| 20261121-164146 | `ui_polish_dialogs` | PASS (21 surfaces checked interactively) | 615 s |
| 20261121-164146 | `ui_polish_jobs` | PASS, but the cancel question and the cancelled window were not reached in either look: the copy ended first (the harness's long copy needs a larger file) | 183 s |
| 20261121-170455 | `ui_review` | FAIL: 3 flags (U-D22 twice, the "Path" heading); the path-segment flags are gone | 262 s |

On Windows `ui_themes` and `options_visual` passed, `ui_polish_viewers` failed (WIN-9)
and `ui_matrix_full` reported SKIP ([Windows results](windows.md#results)).
`ui_matrix_full` (estimated 2.5 h) has not run on either platform.

Runs of 22 November with this record's checks (the new surfaces, the palette's selected
text minimum, interactive red cases 9–11, the Windows job with notes): Windows 11 at
scale 1, `--scale smoke`, the debug acceptance binary through `scripts\acceptance.ps1
--test-binary`, plugin packages of the release build of d7ff8860; Linux in WSL 2 (Ubuntu
24.04), `scripts/acceptance.sh debug`, the shared debug plugin packages of the same base.
Every failure below is a product defect of the table above; none is a gate defect.

| Run | Base | Scenario | Result | Time |
| --- | --- | --- | --- | --- |
| `20261122-033749` (Windows) | 05710878 | `ui_polish_checks` | FAIL: one red case not proven: interactive case 11's corrected twin was flagged (the check could not read a text-mode tooltip's styled text); fixed | 370 s |
| `20261122-033749` (Windows) | 05710878 | `ui_polish_jobs` | PASS; the job with notes (a dangling junction) ended Completed with 1 note in both looks | 256 s |
| `20261122-050833` (Windows) | 05710878 | `ui_polish_checks` | PASS: every static and interactive red case flagged (case 21 at 7:1 and 12:1, not at 4.5:1; interactive cases 9, 10 and 11), corrected twins clean | 361 s |
| `20261122-050833` (Windows) | 05710878 | `options_visual` | PASS | 43 s |
| `20261122-060309` (Windows) | 05710878 | `ui_review` | FAIL: 54 flags: U-D23 (32), U-D27 (4), and two gate defects fixed since (12 × "surface shows no text" for Options under its full-window colour picker and Replace shortcut question; 6 steps at 320×200 that did not scroll the Times pickers and Copy's Links box into view, or clicked a Custom commands button the toolbar clips away) | 499 s |
| `20261122-060309` (Windows) | 05710878 | `ui_themes` | PASS (356 screenshots; selected text at each palette's minimum) | 557 s |
| `20261122-060309` (Windows) | 05710878 | `ui_polish_dialogs` | FAIL: 23 flags: U-D23 (16), U-D24, U-D25, U-D26 (3), and the covered-surface gate defect (2) | 1790 s |
| `20261122-060309` (Windows) | 05710878 | `ui_polish_jobs` | PASS | 270 s |
| `20261122-073609` (Linux) | 05710878 | `ui_review` | FAIL: 38 flags: U-D23 (32), U-D27 (6) | 434 s |
| `20261122-073609` (Linux) | 05710878 | `ui_themes`, `ui_polish_checks`, `ui_polish_jobs`, `options_visual` | PASS | 546, 377, 355, 35 s |
| `20261122-073609` (Linux) | 05710878 | `ui_polish_dialogs` | FAIL: 22 flags: U-D23 (16), U-D24, U-D25, U-D26 (4) | 2220 s |
| `20261122-092401` (Windows) | 26c18895 | `ui_review` | FAIL: 38 flags: U-D23 (32), U-D27 (6); no gate defect left | 518 s |
| `20261122-092401` (Windows) | 26c18895 | `ui_themes`, `ui_polish_checks`, `ui_polish_jobs` (Links chosen with `links`), `options_visual` | PASS | 520, 353, 295, 37 s |
| `20261122-092401` (Windows) | 26c18895 | `ui_polish_dialogs` | FAIL: 20 flags: U-D23 (16), U-D25, U-D26 (3); U-D24 not raised | 1819 s |
| `20261122-123625` (Linux) | 5ccd0eb7 | `ui_review` | FAIL: 38 flags: U-D23 (32), U-D27 (6) | 692 s |
| `20261122-123625` (Linux) | 5ccd0eb7 | `ui_themes`, `ui_polish_checks`, `ui_polish_jobs`, `options_visual` | PASS | 571, 427, 252, 78 s |
| `20261122-123625` (Linux) | 5ccd0eb7 | `ui_polish_dialogs` | FAIL: 22 flags: U-D23 (16), U-D24, U-D25, U-D26 (4) | 1759 s |
| `20261122-123625` (Linux) | 5ccd0eb7 | `ui_polish_viewers` | FAIL: 6 flags, U-D28 (three dismissals in each look); the viewer windows and the password dialog's static checks pass | 166 s |
| `20261122-124508` (Windows) | 5ccd0eb7 | `ui_review` | FAIL: 38 flags: U-D23 (32), U-D27 (6) | 525 s |
| `20261122-124508` (Windows) | 5ccd0eb7 | `ui_themes`, `ui_polish_checks`, `ui_polish_jobs`, `options_visual` | PASS | 484, 381, 292, 44 s |
| `20261122-124508` (Windows) | 5ccd0eb7 | `ui_polish_dialogs` | FAIL: 20 flags: U-D23 (16), U-D25, U-D26 (3); U-D24 not raised | 1782 s |
| `20261122-124508` (Windows) | 5ccd0eb7 | `ui_polish_viewers` | FAIL: 6 flags, U-D28, as on Linux | 128 s |
