# Acceptance coverage: cross-feature journeys

The area records ([panels](panels.md), [transfers](transfers.md), [Find](find.md),
[viewers](viewers.md), [Options and tools](options-tools.md), [UI polish](ui-polish.md))
check each feature on its own. A journey checks the seams between them: a realistic
sequence of steps from different areas, through the real windows (keys, pointer and
wheel, never presenter calls), with the outcome checked on disk and on screen after
every step, so that a step that leaves state behind for the next one (marks, a filter,
a fed list, an open window, a saved setting, a recovery folder) is caught where a user
would meet it. The suite is described in [ACCEPTANCE.md](../ACCEPTANCE.md); the
scenarios are `crates/midnight-acceptance/tests/acceptance/scenarios/journeys_*.rs`,
their helpers `journeys_support.rs`.

Run them with `bash scripts/acceptance.sh --only journeys` (the staged ones need the
plugin packages, `MIDNIGHT_ACCEPTANCE_PLUGINS`). A journey notes every step it
completed (`step N: …` in the report's notes). It goes on after a failed check and
stops only when the next steps cannot run without the failed one; the stop names the
step.

Status: **PASS** (every check of the journey held in the run named below), **FAIL**
(a product bug; the finding is listed under "Findings"), **PENDING** (every check held
except checks of announced behavior that has not landed; reason given), **SKIP** (the
environment cannot run it; reason given).

## Journeys

| Journey | Steps | Checks | Status |
| --- | --- | --- | --- |
| `journeys_find_to_checksums` (staged) | Alt+F7 `needle` in `*.txt` with Search inside archives; Feed to panel; F3 on a result, Escape; Insert × 3, F5; Backspace; Ctrl+Shift+C by name across the panels (also from a subfolder) and between two copies; Ctrl+F8 on the copies, Calculate, Save manifest…, Verify. | Results are exactly three files in two folders and the ZIP member; the fed list holds the three files and counts the member, in the singular (J-1); the viewer shows the result's text and the list keeps its cursor; the copies are byte-identical in one folder while the list stays fed; identical verdicts for result/copy pairs, a different verdict for two copies; each copy's SHA-256 equals an independent digest of its original; the manifest is written beside the copies and verifies 3 of 3. | NOT YET RUN: written and registered; no host run yet |
| `journeys_archive_roundtrip` (staged) | Enter `bundle.zip` › `bundle` › `nested` › `deeper`; Backspace × 2; F5 of the member folder `nested`; Backspace out; Enter `nested.zip` › `inner.zip`; F5 of `inner.txt`; Backspace out; expand the copy's inline branch; F2 inside it; F6 of the copy next to the archives; Ctrl+Shift+S there. | Listings equal the members with their byte sizes; the cursor returns to the member folder and to each archive left; the member tree arrives byte for byte (checked against the same member of the fixture's tar); the inner archive's member arrives; the branch shows the subfolder at depth 1, the renamed one stays at depth 1 with the cursor; the move leaves nothing behind and both panels follow; the measured size equals the disk and the member sizes listed inside the archive; the archives are unchanged. | NOT YET RUN: written and registered; no host run yet |
| `journeys_filtered_conflicts` | Ctrl+F12 Masks `*.txt`; Num + `*.txt\|echo*`; Ctrl+H; F5 onto three existing names answered Replace, Skip and Copy as by the file each question names; the job window; Ctrl+H; Jobs. | The filter lists the dot-name too; the mask marks four files; Ctrl+H drops the hidden mark, keeps the filter and hides dot-names in the other panel; one question per conflict; the job ends Completed with one skip, two files and one recovery copy; its window says "Files copied: 2", "1 skipped item was not copied.", "1 file was kept in a recovery folder; Details shows where." and no notice follows; replaced, kept, renamed and recovered bytes exact, nothing unmarked, hidden or filtered out copied; the target lists the new name with dot-names hidden, the recovery folder appears with Ctrl+H; the source keeps its filter and only the skipped file stays marked; Jobs shows "Completed with warnings" and "2 files · 1 skipped · 0 errors". | FAIL (scenario defect, not a product bug): steps 1–3 held in run 20261120-152552, then the scenario rendered the job's Copy window before sizing it (`attempt to divide by zero` in the software renderer); fix: `app::layout_tool` the window before `screenshot_window` in step 5 |
| `journeys_concurrent_jobs` | F5 of `many/` (Wait off), Pause; F5 of `small/` (Wait off); Shift+F8 of a tree in the same panel; Jobs; Resume. | The paused copy holds still; the second copy completes beside it (pending: "a job whose Wait for other operations is off starts at once"); the delete completes without a question while the copy stays paused and the row goes; Jobs lists the paused copy, the completed copy and "Permanently delete" completed; after Resume both copies equal their sources, the sources are unchanged, the completed copy's window closes, no notice. | NOT YET RUN: written and registered; no host run yet |
| `journeys_options_restart` (staged) | Options: Appearance › Light, Colors › a `*.log` rule moved to the top, Keyboard › Copy (`files.copy-to`) from F5 to Ctrl+Alt+Y, OK; use; restart; use. | One save holding the three choices; at once and after the restart: the panels' luma rises with the theme; `report.log` red, `notes.txt` not; F5 opens nothing, Ctrl+Alt+Y opens Copy, its dialog area is light; Enter copies and the copy is red in the other panel; F3 opens a viewer whose background is light. | NOT YET RUN: written and registered; no host run yet |
| `journeys_links_readonly` | F5 of a deep tree (folder, file, dangling and enclosing links; mode-0555 folders with mode-0444 files) with Follow links cleared, then checked; Ctrl+Shift+S on the two copies; Shift+F8 of each copy. | Cleared: the tree whole, every link a link with its target text, the copied protected folder still without owner write; checked: the linked folder and file copied, the dangling and enclosing links kept with a note each ("copied as a link"); sizes equal the disk and differ by exactly the linked bytes; each delete completes without a question (Delete clears owner write protection inside the deleted tree), the row goes, no notice; the original tree (content, modes, times) and the link targets are unchanged. | NOT YET RUN: written and registered; no host run yet |
| `journeys_keyboard_only` | End/Home at startup; F7; Enter; Shift+F4; Backspace; Ctrl+=; F5, Enter; Tab; F2; Shift+F8, Enter; Tab; Ctrl+G; Alt+Left; Alt+F7, typed mask, Enter, Tab to the results, Home, Enter, Escape; Ctrl+F12 twice; Ctrl+U twice. | The list takes keys at startup; created folder and file on disk with the cursor following; Backspace back onto the folder; the mask marks exactly the two text files; the copy completes and the marks clear; the rename replaces the preselected stem; the delete removes the file; the typed path opens and history returns; Enter in the masks searches, Tab reaches the results, Enter goes to the file, Escape closes Find; the filter applies and clears by Enter in its expression; Ctrl+U swaps and back; no pointer event, nothing left open. | FAIL (not triaged): all nine steps ran in run 20261120-152552 and every check held except Ctrl+U, which did not swap the panels after the Ctrl+F12 steps (the left panel stayed on its folder for 30 s); to triage: harness focus after the filter dialog or a product bug |
| `journeys_pointer_only` (private process) | Double click into a folder and on `..`; a rubber band from the gutter; the footer's F5 cell, the dialog's Copy button, Replace · keep original; a drag onto a folder row of the other panel; the Size header twice and the Name header; the toolbar's Jobs and its Close button; a click on a row, the footer's F8 cell, Move to Trash; a drop within 400 ms of the press. | Location and cursor after the double clicks; the band marks exactly two files; the copy completes after one question with the bytes replaced; the drag offers Move and moves the file into the folder; size order, reversed, name order; Jobs lists the copy and the move as completed and closes; the file is in the private Trash with its bytes; the quick drop does nothing (pending: "a drop within 400 ms of the press is cancelled"); nothing left open. | NOT YET RUN: written and registered; no host run yet |

## Combinations inventory

Each combination below is a seam between two or more area records; "single area"
names where the parts are checked on their own.

| Combination | Single area | Journey |
| --- | --- | --- |
| Find results inside archives fed to a panel; members left out and counted | Find J11 | `journeys_find_to_checksums` |
| A viewer opened from a fed list leaves the list, its cursor and marks as they were | Find K9, viewers X2 | `journeys_find_to_checksums` |
| F5 of marked fed results; the other panel lists the copies while the list stays fed | Find K11 | `journeys_find_to_checksums` |
| Compare a file with its copy by name across the panels, from the root and a subfolder | viewers F1 | `journeys_find_to_checksums` |
| Checksums of copies equal the originals'; a manifest saved beside the copies verifies | tools O-T01 | `journeys_find_to_checksums` |
| An archive member folder copied out equals the same member of another archive format | transfers T-C12, viewers A5 | `journeys_archive_roundtrip` |
| A member of an archive inside an archive copied out; Backspace out keeps the cursor | viewers A6 | `journeys_archive_roundtrip` |
| A copied-out tree in the inline tree, renamed inside the tree, moved back by F6 | panels B-08, transfers T-C06 | `journeys_archive_roundtrip` |
| Folder sizes equal the member sizes the archive listing showed | panels Z-01 | `journeys_archive_roundtrip` |
| A panel filter, hidden files and a mask selection decide what F5 copies | panels F-01, H-02, S-05 | `journeys_filtered_conflicts` |
| Three different conflict answers in one job; its outcome text; Jobs history | transfers T-K01–K03, O-J01 | `journeys_filtered_conflicts` |
| The recovery folder a Replace creates follows Show hidden files | transfers T-K01, panels H-01 | `journeys_filtered_conflicts` |
| Completed roots deselected, skipped ones kept, under a filter and hidden files | panels S-15 | `journeys_filtered_conflicts` |
| A permanent delete while a copy is paused and another copy runs | transfers T-J01, T-R09 | `journeys_concurrent_jobs` |
| Jobs lists copies and a delete in their states together | transfers T-J03, tools O-J01 | `journeys_concurrent_jobs` |
| Theme, colour rule and key rebinding saved in one session, used at once and after a restart | options O-A01, O-L01, O-K08 | `journeys_options_restart` |
| A rebound key opens its dialog in the new theme and the job runs | options O-K08 | `journeys_options_restart` |
| A colour rule colours a file copied into the other panel | options O-L01 | `journeys_options_restart` |
| A viewer opened after a theme change and after a restart uses the saved theme | viewers X5 | `journeys_options_restart` |
| Follow links cleared and checked on one tree with write-protected folders | transfers T-D09, T-C02 | `journeys_links_readonly` |
| Folder sizes of a copy with links kept and one with links followed | panels Z-02 | `journeys_links_readonly` |
| Shift+F8 of a copied tree holding write-protected folders, without a question | transfers T-R13 | `journeys_links_readonly` |
| A whole session by keys only, Find included | keyboard O-S01–S10 | `journeys_keyboard_only` |
| A whole session by mouse only, drag and drop and the Trash included | transfers T-X04–X06, panels V-02 | `journeys_pointer_only` |
| A drop within 400 ms of the press is cancelled | – | `journeys_pointer_only` (pending) |

## Findings

| Finding | Product bug | Check | Status |
| --- | --- | --- | --- |
| J-1 | The fed list's status counts one omitted archive member in the plural: "1 archive members not shown" (`crates/midnight-ui/src/find_feed.rs`, `status`, formats `" · {} archive members not shown"` for every count). Found by reading the code; fixed in source on 21 November ("1 archive member not shown"); the check is written, not run yet. | `journeys_find_to_checksums` › the fed list's count of left-out members is grammatical | NOT YET RUN |

Pending checks written (they report `PENDING` while the behavior is missing):

- `journeys_concurrent_jobs`: a copy with "Wait for other operations to finish" off
  completes beside a paused one ("a job whose Wait for other operations is off starts at
  once", as in the transfers record's T-J01).
- `journeys_pointer_only`: a drop released within 400 ms of the press does nothing ("a
  drop within 400 ms of the press is cancelled").

## Host runs

Run 20261120-152552 (smoke scale, in-process journeys only, shared test binary built
at 16:24 from this checkout): `journeys_keyboard_only` FAIL (Ctrl+U, see above, 44.6 s),
`journeys_filtered_conflicts` FAIL (scenario defect, see above, 35.5 s). The other six
journeys compile and are registered but have not run yet. Run them with the newest shared test binary:

```text
MIDNIGHT_ACCEPTANCE_RUN_DIR=<checkout>/_build/acceptance-journeys \
MIDNIGHT_ACCEPTANCE_PLUGINS=<a complete plugin set, e.g. _build/debug> \
  _build/<target>/debug/deps/acceptance-<hash> journeys
```
