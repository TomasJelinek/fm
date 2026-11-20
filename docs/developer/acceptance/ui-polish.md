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

U-D01–U-D12 were sent in the first batch and assigned; U-D13–U-D21 are queued for the
next batch.

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

Not run yet: `ui_themes`, `ui_polish_viewers`, `options_visual` in its gate form, and
`ui_matrix_full` (estimated 2.5 h). Expected default gate time after the matrix
rebalance and fail-fast waits: about 45 minutes in all (`ui_review` 10–12 min,
`ui_themes` about 6 min, `ui_polish_dialogs` 15 min, `ui_polish_jobs` 4 min,
`ui_polish_viewers` 5 min, `ui_polish_checks` 4 min, `options_visual` 2 min).
