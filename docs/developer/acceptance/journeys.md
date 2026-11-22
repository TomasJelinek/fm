# Acceptance coverage: cross-feature journeys

The area records ([panels](panels.md), [transfers](transfers.md), [Find](find.md),
[viewers](viewers.md), [Options and keyboard](options-tools.md), [tools](tools.md),
[UI polish](ui-polish.md))
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
| `journeys_find_to_checksums` (staged) | Alt+F7 `needle` in `*.txt` with Search inside archives; Feed to panel; F3 on a result, Escape; Insert × 3, F5; Backspace; Ctrl+Shift+C by name across the panels (also from a subfolder) and between two copies; Ctrl+F8 on the copies, Calculate, Save manifest…, Verify. | Results are exactly three files in two folders and the ZIP member; the fed list holds the three files and counts the member, in the singular (J-1); the viewer shows the result's text and the list keeps its cursor; the copies are byte-identical in one folder while the list stays fed; identical verdicts for result/copy pairs, a different verdict for two copies; each copy's SHA-256 equals an independent digest of its original; the manifest is written beside the copies and verifies 3 of 3. | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` and on Windows in run 20261122-141148. Increment 2: PASS on Linux in run `20261122-050758` and on Windows in run 20261122-052221; Windows run 20261122-073649 failed once, Find's search inside `bundle.zip` having ended Incomplete (3 found, 7 examined), and its rerun 20261122-090231 passed (the journey now names Find's details when that happens). Earlier: PASS on Linux in run `20261121-144246` (7.1 s; the fed list's status says "1 archive member not shown", J-1 fixed); PASS on Windows in run 20261121-043913 ([Windows results](windows.md#results)) |
| `journeys_archive_roundtrip` (staged) | Enter `bundle.zip` › `bundle` › `nested` › `deeper`; Backspace × 2; F5 of the member folder `nested`; Backspace out; Enter `nested.zip` › `inner.zip`; F5 of `inner.txt`; Backspace out; expand the copy's inline branch; F2 inside it; F6 of the copy next to the archives; Ctrl+Shift+S there. Then `locked.zip` (a ZipCrypto, a WinZip AES-256 and a plain member, written beside the archives; a non-ASCII password): Enter; F5 of the ZipCrypto member, a wrong password and Enter, the right one and Enter, Enter in the Copy dialog; F3 on the AES member, Escape; Backspace; Enter; Insert on the plain and the ZipCrypto member, F5, Escape in the password dialog, Enter in the Copy dialog. | Listings equal the members with their byte sizes; the cursor returns to the member folder and to each archive left; the member tree arrives byte for byte (checked against the same member of the fixture's tar); the inner archive's member arrives; the branch shows the subfolder at depth 1, the renamed one stays at depth 1 with the cursor; the move leaves nothing behind and both panels follow; the measured size equals the disk and the member sizes listed inside the archive; the archives are unchanged. F5 of the encrypted member asks for the archive's password before the Copy dialog; the wrong password is refused in the dialog ("Wrong password — try again."); the right one opens the Copy dialog and the member arrives byte for byte; F3 on the AES member shows its text without asking (the archive keeps the password until it is closed); entered again, F5 asks again, and after Escape the Copy dialog opens, as the dialog says ("without it they are skipped"): the plain member arrives, the protected one is skipped with the reason "no password was entered" (Completed with errors: 1 file, 1 skipped) and is not written, and no password is sent; `locked.zip` is unchanged. | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` (20.4 s) and on Windows in run 20261122-141148 (25.0 s). Increment 2 (without the password step): PASS on Linux in run `20261122-091132` and on Windows in run 20261122-073649. Earlier: PASS on Linux in run `20261121-144246` (6.6 s) and at full scale in run `20261121-171419` (12.6 s); PASS on Windows in run 20261121-032850 ([Windows results](windows.md#results)) |
| `journeys_filtered_conflicts` | Ctrl+F12 Masks `*.txt`; Num + `*.txt\|echo*`; Ctrl+H; F5 onto three existing names answered Replace, Skip and Copy as by the file each question names; the job window; Ctrl+H; Jobs. | The filter lists the dot-name too; the mask marks four files; Ctrl+H drops the hidden mark, keeps the filter and hides dot-names in the other panel; one question per conflict; the job ends Completed with one skip, two files and one recovery copy; its window says "Files copied: 2", "1 skipped item was not copied.", "1 file was kept in a recovery folder; Details shows where." and no notice follows; replaced, kept, renamed and recovered bytes exact, nothing unmarked, hidden or filtered out copied; the target lists the new name with dot-names hidden, the recovery folder appears with Ctrl+H; the source keeps its filter and only the skipped file stays marked; Jobs shows "Completed with warnings" and "2 files · 1 skipped · 0 errors". | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` and on Windows in run 20261122-141148. Increment 2: PASS on Linux in run `20261122-050758` and on Windows in run 20261122-052221. Earlier: PASS on Linux in run `20261121-144246` (3.6 s; Jobs: "Completed with warnings", "2 files · 1 skipped · 0 errors"), with the scenario fix of 21 November (the job's window sized before it is read or rendered; run 20261120-152552 had failed on that). On Windows (where the recovery folder has no hidden attribute and is listed while hidden files are hidden) every check held in run 20261121-043911 ([windows.md](windows.md)) |
| `journeys_concurrent_jobs` | F5 of `many/` (Wait off), Pause; F5 of `small/` (Wait off); Shift+F8 of a tree in the same panel; Jobs; Resume. | The paused copy holds still; the second copy completes beside it (a strict check since its pending mark was removed on 21 November: "a job whose Wait for earlier transfers is off starts at once"); the delete asks no question and waits queued, deleting nothing, while the copy stays paused (a Delete holds the whole namespace, and every transfer keeps its namespace share until it ends, paused or not: `OPERATION_PROGRESS.md`, "Admission policy and waits"); Jobs lists the paused copy, the completed copy and "Permanently delete" queued; after Resume both copies equal their sources, the sources are unchanged, the completed copy's window closes, then the delete completes and the row goes, no notice. Until 21 November the journey expected the delete to complete beside the paused copy, which the documented design does not provide (Windows run, [windows.md](windows.md)). | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` and on Windows in run 20261122-141148. Increment 2: PASS on Linux in run `20261122-050758` and on Windows in run 20261122-052221. Earlier: PASS on Linux in run `20261121-144246` (6.8 s): the delete waited queued while the copy stayed paused, and the then pending check passed; every check held on Windows in run 20261121-042048, and with the check strict in run `20261121-192806` ([windows.md](windows.md)) |
| `journeys_options_restart` (staged) | Options: Appearance › Light, Colors › a `*.log` rule moved to the top, Keyboard › Copy (`files.copy-to`) from F5 to Ctrl+Alt+Y, OK; use; restart; use; Options again, Cancel. | One save holding the three choices; at once and after the restart: the panels' luma rises with the theme; `report.log` red, `notes.txt` not; F5 opens nothing, Ctrl+Alt+Y opens Copy, its dialog area is light; Enter copies and the copy is red in the other panel; F3 opens a viewer whose background is light. The default palette gives `readme.md`, `notes.txt` and `plan.docx` one colour (the documents' blue since f153478c) and `main.rs` another (the source cyan), neither the theme's text colour that `data.bin` keeps, at once and the same after the restart; after the restart Options lists its pages from the top in their order of use. | Wave 2 (41adbb86, with the palette of f153478c): PASS on Linux in run `20261122-142728` (31.7 s) and on Windows in run 20261122-141148 (29.0 s). Increment 2 (without the palette and page order): PASS on Linux in run `20261122-091132` and on Windows in run 20261122-073649. Earlier: PASS on Linux in run `20261121-144246` (13.4 s); PASS on Windows in run 20261121-032203 ([Windows results](windows.md#results)) |
| `journeys_links_readonly` | F5 of a deep tree (folder, file, dangling and enclosing links; mode-0555 folders with mode-0444 files) with the Links choice "Keep links as links", then "Copy the files links point to" (before the Links choice: Follow links cleared, then checked); Ctrl+Shift+S on the two copies; Shift+F8 of each copy. | Kept as links: the tree whole, every link a link with its target text, the copied protected folder still without owner write; copying the files links point to: the linked folder and file copied, the dangling and enclosing links kept with a note each ("copied as a link"); sizes equal the disk and differ by exactly the linked bytes; each delete completes without a question (Delete clears owner write protection inside the deleted tree), the row goes, no notice; the original tree (content, modes, times) and the link targets are unchanged. | With the Links choice: PASS on Linux in runs `20261122-142728` (9.3 s, wave 2) and `20261122-050758` (4.9 s); SKIP on Windows in runs 20261122-141148 and 20261122-052221 ("the journey checks Linux relative symbolic links and permission modes: Windows symbolic links need Developer Mode or administrator rights and folders are protected by ACLs (files_links covers junctions, kept and followed)"). Before the Links choice: PASS on Linux in run `20261121-144246` (2.3 s; the plain copy measures 25 B, the followed one 71 B); SKIP on Windows in run 20261121-032850 ([Windows results](windows.md#results)). |
| `journeys_keyboard_only` | End/Home at startup; F7; Enter; Shift+F4; Backspace; Ctrl+=; F5, Enter; F5 again onto the changed copies: Tab, Copy as, then Escape at the second question; F10, Down to Application, Tab, Space (Jobs); Up and Down to the waiting job, Tab, Space (Resolve); Skip; Tab; F2; Shift+F8, Enter; in `links/` F5 of a link to a folder (a directory junction on Windows), Tab to Links, Up, Enter; Tab; Ctrl+G; Alt+Left; Alt+F7, typed mask, Enter, Tab to the results, Home, Enter, Escape; Ctrl+F12 twice; Ctrl+U twice. | The list takes keys at startup; created folder and file on disk with the cursor following; Backspace back onto the folder; the mask marks exactly the two text files; the copy completes and the marks clear; every conflict question is answered by keys and Escape leaves one for later (the job waits); Jobs, opened by keys, selects the waiting job by Up and Down, and its Resolve brings the question back; the job ends with one skip, alpha's text under the typed name and both copies kept; the rename replaces the preselected stem; the delete removes the file; Tab, Up and Enter in the Copy dialog choose "Copy the files links point to": the folder's tree arrives and the link itself is unchanged; the typed path opens and history returns; Enter in the masks searches, Tab reaches the results, Enter goes to the file, Escape closes Find; the filter applies and clears by Enter in its expression; Ctrl+U swaps the panels' places and back, each keeping its folder; no pointer event, nothing left open. | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` (51.7 s; `links/to-target` a symbolic link) and on Windows in run 20261122-141148 (27.0 s; a directory junction). Increment 2 (without the conflicts, Jobs and Links steps): PASS on Linux in run `20261122-091132` and on Windows in run 20261122-073649. Earlier: PASS on Linux in run `20261121-144246` (5.6 s), with the journey fix of 21 November (Ctrl+U swaps the panels' places and each keeps its folder, as `files_navigation` checks, N-27; run 20261120-152552 had failed on that); every check held on Windows in run 20261121-042048 ([windows.md](windows.md)) |
| `journeys_pointer_only` (private process) | Double click into a folder and on `..`; a rubber band from the gutter; the footer's F5 cell, the dialog's Copy button, Replace · keep original; a drag onto a folder row of the other panel; the Size header twice and the Name header; the toolbar's Jobs and its Close button; a click on a row, the footer's F8 cell, Move to Trash; on the held Slint clock (`App::hold_clock`), a drag released 250 ms after its press and the same drag released 450 ms after it; in a folder of 60 rows, a wheel of 1.5 rows, the rest, a double click on the first fully visible row. | Location and cursor after the double clicks; the band marks exactly two files; the copy completes after one question with the bytes replaced, and its window stays with the kept original ("1 file was kept in a recovery folder; Details shows where.") until its Close button; the drag offers Move and moves the file into the folder; size order, reversed, name order; Jobs lists the copy as completed with warnings (the kept original) and the move as completed, and closes; the file is in the private Trash with its bytes; the 250 ms drop starts nothing (no dialog, no job, the folder stays) and the left panel's status line says "Drag cancelled: released within 0.4 s"; the 450 ms drop moves the folder; the wheel takes the list 1.5 rows down with a row cut at its top edge, every frame until the list rests has a row across that edge (and, a row and more down, the row's text in the band below it, not one plain colour), the list rests on a whole row (within 0.5 px) after 30 still frames, and a double click opens the first fully visible row; nothing left open. Until 21 November the journey expected the copy to complete without warnings and left its window open. | Wave 2 (41adbb86): PASS on Linux in run `20261122-142728` (45.1 s) and on Windows in run 20261122-141148 (25.8 s), with the same drops and wheel. Increment 2: PASS on Linux in run `20261122-050758` (33.3 s) and on Windows in run 20261122-052221 (29.0 s; F8's confirmation cancelled, no private Recycle Bin): both drops offered Move (feedback 4), the 250 ms one was cancelled with the note and the 450 ms one moved `folder/`; the wheel cut a row at 33.0 px, 70 frames were drawn to the top, the list rested at 44.0 px (22 px rows) and the double click opened `r01`. Before 22 November the quick drop was a pending check that could not be observed (the synthetic drag took 482–683 ms of real time): PENDING on Linux in run `20261121-164146` (with the Linux harness patch): every check held, the move "Completed" without warnings; the quick drop could not be observed (the synthetic drag took 603 ms). In run `20261121-144246` it failed only on the journey's own Trash check, which looked for `Trash/files/b-two.txt` while a trashed entry is named `b-two.txt-midnight-<unique>` with its path in the `.trashinfo` (harness defect; the patch finds the entry as `files_trash` does). On Windows run 20261121-043911 every check held except the move's state, "Completed with warnings" (finding WIN-6 in [windows.md](windows.md)), and the pending quick drop |

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
| An encrypted member's password asked once by F5, kept for F3 until the archive closes; without it the copy goes on and skips the protected member | viewers P2, P3 | `journeys_archive_roundtrip` |
| A panel filter, hidden files and a mask selection decide what F5 copies | panels F-01, H-02, S-05 | `journeys_filtered_conflicts` |
| Three different conflict answers in one job; its outcome text; Jobs history | transfers T-K01–K03, tools O-J01 | `journeys_filtered_conflicts` |
| The recovery folder a Replace creates follows Show hidden files | transfers T-K01, panels H-01 | `journeys_filtered_conflicts` |
| Completed roots deselected, skipped ones kept, under a filter and hidden files | panels S-15 | `journeys_filtered_conflicts` |
| A permanent delete while a copy is paused and another copy runs | transfers T-J01, T-R09 | `journeys_concurrent_jobs` |
| Jobs lists copies and a delete in their states together | transfers T-J03, tools O-J01 | `journeys_concurrent_jobs` |
| Theme, colour rule and key rebinding saved in one session, used at once and after a restart | options O-A01, O-L01, O-K08 | `journeys_options_restart` |
| A rebound key opens its dialog in the new theme and the job runs | options O-K08 | `journeys_options_restart` |
| A colour rule colours a file copied into the other panel | options O-L01 | `journeys_options_restart` |
| A viewer opened after a theme change and after a restart uses the saved theme | viewers X5 | `journeys_options_restart` |
| The default palette's document and source colours and the Options pages' order survive a restart | options O-L01, O-D03 | `journeys_options_restart` |
| Links kept as links, then the files they point to copied, on one tree with write-protected folders | transfers T-D09, T-C02 | `journeys_links_readonly` |
| Folder sizes of a copy with links kept and one with links followed | panels Z-02 | `journeys_links_readonly` |
| Shift+F8 of a copied tree holding write-protected folders, without a question | transfers T-R13 | `journeys_links_readonly` |
| A whole session by keys only, Find included | keyboard O-S01–S10 | `journeys_keyboard_only` |
| Conflicts answered by keys: Copy as, Escape to decide later, Jobs and Resolve by keys | transfers T-K03, T-K08 | `journeys_keyboard_only` |
| The Copy dialog's Links choice by keys; the files a link points to copied | transfers T-D09 | `journeys_keyboard_only` |
| A whole session by mouse only, drag and drop and the Trash included | transfers T-X04–X06, panels V-02 | `journeys_pointer_only` |
| A drop sooner than 0.4 s after the press is cancelled and says so; a later one moves | – | `journeys_pointer_only` (on the held Slint clock) |
| A wheel of 1.5 rows glides to a whole row with no blank band at the list's top; a double click then opens the first fully visible row | – | `journeys_pointer_only` |

## Findings

| Finding | Product bug | Check | Status |
| --- | --- | --- | --- |
| J-1 | The fed list's status counts one omitted archive member in the plural: "1 archive members not shown" (`crates/midnight-ui/src/find_feed.rs`, `status`, formats `" · {} archive members not shown"` for every count). Found by reading the code; fixed in source on 21 November ("1 archive member not shown"). | `journeys_find_to_checksums` › the fed list's count of left-out members is grammatical | Fixed: PASS on Linux in run `20261121-144246` and on Windows in run 20261121-043913 ([Windows results](windows.md#results)) |

Pending checks written (a failure of one reports `PENDING` instead of `FAIL`). Both
behaviors were implemented on 20 November (d9cafd45); the harness still reports these
checks as pending until they are promoted to ordinary checks:

- `journeys_concurrent_jobs`: a copy with "Wait for earlier transfers" off
  completes beside a paused one ("a job whose Wait for earlier transfers is off starts
  at once", as in the transfers record's T-J01). It held on Windows in run 20261121-042048
  and on Linux in run `20261121-144246`: the mark can be removed.
- `journeys_pointer_only`'s quick drop is a strict check since 22 November: the journey
  holds the Slint clock (`App::hold_clock`, which the 0.4 s rule follows) so that the
  real time a synthetic drag takes does not count; before, the drag took 482–683 ms of
  real time and the quick drop could not be observed.

## Host runs

Rerun on main be2d2dea, after the Copy dialog's polish (142aa74b) and the Panel filter's
shared edit line (27189334), smoke scale, debug test builds with `--only
harness_checks,startup,journeys`: Linux run `20261122-195747` passed all eleven
scenarios (198.6 s) and Windows run 20261122-173839 ten, `journeys_links_readonly`
reporting SKIP (251.5 s). `journeys_keyboard_only` filtered with Ctrl+F12 and Enter in
the new edit line and chose "Copy the files links point to" by Tab, Up and Enter in the
polished Copy dialog; the other journeys' steps held as below.

Runs of 22 November for wave 2, smoke scale, debug test builds of 41adbb86 with WP1's
wave 2 (`--only harness_checks,startup,journeys`): Linux run `20261122-142728` (WSL 2,
Ubuntu 24.04 on ext4, packages of b1b89343) passed all eleven scenarios; Windows run
20261122-141148 (release packages of 60e08043, run folders on an NTFS volume without 8.3
names) passed ten, and `journeys_links_readonly` reported SKIP. The red runs of the new
checks failed as each red case was meant to (Windows runs 20261122-121748,
20261122-122433 and 20261122-141533; [Windows
results](windows.md#journeys-on-22-november)). Increment 2's final runs: Linux
`20261122-091132` (all thirteen passed) and Windows 20261122-073649, where
`journeys_find_to_checksums` failed once on an incomplete Find inside `bundle.zip`
(rerun 20261122-090231 passed).

Runs of 22 November, smoke scale, debug test build of a901e438 with WP1's increment 2
(`--only harness_checks,startup,journeys,tools_jobs_copy,ui_polish_jobs`): Linux run
`20261122-050758` (WSL 2, Ubuntu 24.04 on ext4, packages of 05710878) and Windows run
20261122-052221 (release packages of 60e08043, run folders on an NTFS volume without 8.3
names). All eight journeys passed on Linux; on Windows seven passed and
`journeys_links_readonly` reported SKIP.

Linux run `20261121-144246` of commit 3f6fff8a (WSL 2, Ubuntu 24.04 on ext4, smoke
scale, debug test build, every plugin package built from that commit; report in
`_build/acceptance-linux/20261121-144246/`): all eight journeys ran, seven passed
(2.3–13.4 s each) and `journeys_pointer_only` failed on its own Trash check (harness
defect, above). Rerun `20261121-164146` with the Linux harness patch
(committed with this record):
`journeys_pointer_only` PENDING (only the unobservable quick drop).

Run 20261120-152552 (smoke scale, in-process journeys only, shared test binary built
at 16:24 from this checkout): `journeys_keyboard_only` FAIL (Ctrl+U, see above, 44.6 s),
`journeys_filtered_conflicts` FAIL (scenario defect, see above, 35.5 s). On Windows
`journeys_find_to_checksums`, `journeys_archive_roundtrip`, `journeys_concurrent_jobs`
and `journeys_options_restart` passed, `journeys_pointer_only` failed (WIN-6) and
`journeys_links_readonly` reported SKIP ([Windows results](windows.md#results)). Run
them on Linux with the newest shared test binary:

```text
MIDNIGHT_ACCEPTANCE_RUN_DIR=<checkout>/_build/acceptance-journeys \
MIDNIGHT_ACCEPTANCE_PLUGINS=<a complete plugin set, e.g. _build/debug> \
  _build/<target>/debug/deps/acceptance-<hash> journeys
```
