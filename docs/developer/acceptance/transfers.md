# Acceptance coverage: transfers, Delete and jobs

The inventory of user-visible Copy (F5), Move (F6), Delete (F8, Shift+F8), clipboard,
drag-and-drop and job-window behavior, with the acceptance scenario and check that
covers each item and its last verified status. The suite is described in
[ACCEPTANCE.md](../ACCEPTANCE.md); scenarios live in
`crates/midnight-acceptance/tests/acceptance/scenarios/`.

Status: **PASS** (the check passed in the run named below), **FAIL** (a product bug,
with its finding), **PENDING** (announced behavior that has not landed; the check is in
the suite at full strength and is reported, not failed), **SKIP** (the environment
cannot run it; reason given), **OPEN** (not covered yet).

Last runs: the Linux run `20261121-144246` of commit 3f6fff8a (WSL 2, Ubuntu 24.04 on
ext4, smoke scale, debug test build, every plugin package built from that commit by
`./build.sh`; every scenario of the suite), and its rerun `20261121-164146` of
`files_transfer_errors` and `files_trash` with the Linux harness fixes
(committed with this record; see "Harness"
below), and the full-scale run `20261121-171419` of the Copy, Move and job scenarios
(`copy_move_delete`, `files_copy_kinds`, `files_move_kinds`, `files_conflicts`,
`files_links`, `files_copy_options`, `files_transfer_jobs`, `files_transfer_errors`,
`files_clipboard_drag`, `files_archive_transfers`, `tools_jobs_copy`) with the same
fixes: the same results as at smoke scale (`files_transfer_errors` PENDING on T-E06
only, every other one PASS).
Reports: `_build/acceptance-linux/<run>/`. Statuses without a run name are from
`20261121-144246`. Earlier runs (host, plugins `_build/try-1505`): t-run5 and t-run6
(`_build/acceptance-fixes/t-runs/20261120-150424`). Status words: as above, plus **NOT
RUN** (the check exists but has not run since its last rework) and **NOT VERIFIED** (the
scenario stopped before reporting it).

The Links choice (b29220bb), acceptance phase: the Linux run `20261122-024836` (WSL 2,
Ubuntu 24.04 on ext4, smoke scale, commit 60e08043 with this record's suite; no plugin
packages) of `files_copy_options`, `files_links`, `files_transfer_errors`,
`files_conflicts`, `files_transfer_jobs` and `journeys_links_readonly` (all PASS), its red
runs `20261122-025521` (the default, note and reason expectations reversed: seven failures)
and `20261122-025746` (the Links keys swapped: each choice refused), and the Windows runs
`20261122-024919` and `20261122-025708` (Windows 11, NTFS, smoke scale, no plugin packages).

## Copy/Move dialog

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-D01 | F5 shows the selection and its source folder | `files_copy_options` | summary names `a.txt` and the source folder | PASS |
| T-D02 | The destination is the other panel's folder | `files_copy_options` | "Destination folder" value | PASS |
| T-D03 | A typed existing folder is the destination | `files_copy_kinds`, `files_transfer_jobs`, `files_transfer_errors` | files arrive in the typed folder | PASS |
| T-D04 | A missing folder is refused in the dialog | `files_copy_options` | dialog stays open, "Enter the full path of an existing destination folder.", no job | PASS: ACC-OPT-01 (a missing folder admitted as a job) no longer happens |
| T-D05 | A relative path is refused | `files_copy_options` | same | PASS |
| T-D06 | An empty destination cannot be confirmed | `files_copy_options` | Enter starts nothing | PASS |
| T-D07 | Escape copies nothing | `files_copy_options` | target folder empty | PASS |
| T-D08 | Exact modified time: offered for files, not for folders; kept to the nanosecond | `files_copy_options`, `files_copy_kinds` | check box state; times equal | PASS (both scenarios) |
| T-D09 | Copy offers one **Links** choice; on a fixed local folder it shows "Keep links as links", of exactly "Copy the files links point to" and "Keep links as links", with no reason or warning line (none read by a screen reader either); the former "Follow links" check boxes are gone | `files_copy_options` | the Links box's entries, value and description once the folder's link facts arrived; no former check box | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-D10 | Move offers the same Links choice (same default on a fixed local folder); its policy line names renames | `files_copy_options` | as T-D09 for F6; policy text | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-D11 | "Wait for other operations to finish" shown, unchecked | `files_copy_options`, `files_transfer_jobs` | check box state | PASS |
| T-D12 | Full SHA-256 verification (default) reads every byte back | `files_copy_options` | `verified_bytes` equals the size; policy text | PASS (2,097,152 of 2,097,152 bytes read back; at full scale 25,165,824 of 25,165,824) |
| T-D13 | Size-and-samples verification from Options › General | `files_copy_options` | policy text; at most three 64 KiB samples read back; bytes equal | PASS (196,608 bytes read back) |

## Copy and Move

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-C01 | Every fixture kind copied one at a time | `files_copy_kinds` | bytes, exact names, trees, rows | PASS |
| T-C02 | Copies get the source's permission bits masked by the umask, inside copied folders too | `files_copy_kinds` | modes of every copied entry | PASS |
| T-C03 | A 12-level tree in one job | `files_copy_kinds` | tree equal | PASS |
| T-C04 | Many small files (3,000 at full scale) in one job | `files_copy_options`, `files_transfer_jobs` | tree equal | PASS at smoke scale (400 files in 2.4 s) and at full scale in `20261121-171419` (3,000 files in 20.5 s, each synced and read back; job 1 of `files_transfer_jobs` ended 14.7 s after Resume). t-run5 took 59.5 s for 3,000 files |
| T-C05 | A multi-selection into a typed subfolder with exact times | `files_copy_kinds` | bytes and times | PASS |
| T-C06 | Every fixture kind moved one at a time; mode and times kept | `files_move_kinds` | facts equal; source gone | PASS |
| T-C07 | An owned write-protected folder (mode 0555, with contents) moves as it is, without a question, and keeps its mode | `files_move_kinds`, `files_transfer_errors` | no question; tree, folder modes and source gone | PASS (both scenarios) |
| T-C08 | Every entry is accounted for (moved or refused) | `files_move_kinds` | counts one at a time and in multi-selections | PASS |
| T-C09 | Copy to another filesystem | `files_transfer_errors` | bytes in a private folder on `$XDG_RUNTIME_DIR` | PASS (tmpfs `/run/user/1000`) |
| T-C10 | Move to another filesystem moves, or keeps the source untouched | `files_transfer_errors` | either outcome consistent | PASS: Failed, source kept ("Cross-volume Move requires a qualified verified-copy destination; the source was retained.") |
| T-C11 | Cross-volume Move's source removal clears owner write protection inside the moved tree | – | needs two qualified (ext) filesystems | SKIP: one ext filesystem on the test host |
| T-C12 | Archive members as a source: a file, a nested and an empty folder from a ZIP; the same tree from a tar.gz | `files_archive_transfers`, `viewers_archives` (V) | known bytes; the two copies equal | PASS |
| T-C13 | An archive as a target is refused; F6 of a member is refused | `files_archive_transfers` | archive bytes unchanged; refusal reported; F6 notice | PASS |
| T-C14 | Inline tree rows as a source, marked at different depths of two branches | `files_copy_options`, `files_tree_ops` (P), `files_trash` | each marked row copied as its own folder; nested row trashed | PASS (copy of marked tree rows and the nested row's Trash, T-R08) |

## Links

The Links choice of Copy and Move (`files_links`; its default and wording on other kinds
of destination in `files_links_volumes`). Linux uses the fixture's symbolic links
(`links/to-folder`, `to-file`, `dangling`, `to-self`, all relative); Windows uses
directory junctions to a folder, to a missing folder and to their own folder (file
symbolic links need Developer Mode or administrator rights).

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-L01 | Keep links as links (the default on a fixed local folder) recreates each link with the same target: Linux symbolic links with the same text, Windows junctions to the same folder; the job details name each kept link whose relative target leads outside the copied items | `files_links` | `read_link` of each copy; the notes ("… which leads outside the copied items") for `../` targets only | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-L02 | Copy the files links point to: a folder link arrives as the folder's tree, a file link as the file; a dangling link stays a link with its reason in the job details; a selected link to its own folder is followed once and the links met again inside that copy stay links, each with its reason (Linux and Windows alike); a folder holding only a link to itself copies once | `files_links` | trees, bytes, `read_link`, notes ("its target does not exist", "contains it") | PASS on Linux (`20261122-024836`). Windows (`20261122-024919`): the folder and dangling junctions PASS; the junction to its own folder was followed once, against the check's expectation, which was wrong (the product does so on Linux too, and its engine test asserts it); the corrected check has not run on Windows yet |
| T-L03 | The Links choice by keyboard (Tab to it, Up or Down, Enter from the box), by pointer (a click on the entry) and with the open list's keys (Enter in the list chooses the entry and never confirms the dialog) | `files_links` (each pass uses one way) | the dialog stays open with no job until confirmed; the choice shown when confirmed | PASS (Linux `20261122-024836`: keyboard, pointer and the list's keys; Windows `20261122-024919`: keyboard and pointer) |
| T-L04 | F6 with the offered choice (keep links) moves each link itself, the folder it points to unchanged | `files_links` | the moved link's target; source gone | PASS (Linux `20261122-024836`, Windows `20261122-024919`: all three junctions) |

## Conflicts

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-K01 | Replace keeps the original in a recovery folder | `files_conflicts` | recovery copy holds the original | PASS |
| T-K02 | Skip, Skip all (one question) | `files_conflicts` | target unchanged | PASS |
| T-K03 | Copy as another name | `files_conflicts` | new name holds the bytes | PASS |
| T-K04 | Replace all (one question) | `files_conflicts` | all replaced, originals kept | PASS |
| T-K05 | Merge folders, then Skip a child | `files_conflicts` | merged tree | PASS |
| T-K06 | Merge all folders (one question) | `files_transfer_errors` | both folders merged | PASS |
| T-K07 | Cancel job midway keeps the earlier copy | `files_conflicts` | earlier file kept, later not started | PASS |
| T-K08 | Decide later leaves the job waiting; Resolve brings the question back | `files_transfer_errors` | `WaitingForDecision`; Resolve; Skip | PASS |
| T-K09 | Copy into its own folder; folder into itself refused | `files_conflicts` | nothing changes | PASS |

## Failures

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-E01 | A read-only destination asks Retry/Skip/Skip all this error type/Cancel job; Retry works once it is writable | `files_transfer_errors` | buttons; file copied; no further question | PASS |
| T-E02 | Skip all this error type: one question for three files | `files_transfer_errors` | skipped 3, nothing written | PASS |
| T-E03 | Cancel job at an error | `files_transfer_errors` | Cancelled, nothing written | PASS |
| T-E04 | An unreadable file: Retry once it is readable | `files_transfer_errors` | file copied | PASS |
| T-E05 | An unreadable folder in a tree: Skip and continue | `files_transfer_errors` | rest of the tree copied | PASS |
| T-E06 | A Move out of a mode-0555 folder asks Retry/Skip/Skip all this error type/Cancel job; Retry once the folder is writable moves the file | `files_transfer_errors` | buttons; file moved, no further question (finding ACC-ERR-01 otherwise) | PENDING (also in `20261121-164146`): a Move out of a mode-0555 folder ends Failed without a question; the Retry part (ACC-ERR-01) waits for the question |
| T-E07 | The job window shows the error itself, no following dialog | `files_transfer_errors` | window text, no notice | PASS |
| T-E08 | Details shows more | `files_transfer_errors` | more texts after Details | PASS |
| T-E09 | An outcome lists only items with errors and counts notes (a skipped unreadable child folder, a link copied as a link) | `files_transfer_errors` | error path listed, no per-note sentence | PASS: "1 skipped item was not copied." and "1 link was copied as a link; Details says why." |
| T-E10 | An unreadable selected folder fails while the transfer is prepared; Skip leaves it out and copies the rest of the selection | `files_transfer_errors` | Skip offered; the other file copied | PASS in `20261121-164146`. In `20261121-144246` the check read the question's buttons before sizing its window, saw only Decide later and answered Cancel job (harness defect, fixed by the Linux harness patch) |

## Jobs and their windows

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-J01 | Wait off: a second job runs at once beside a paused one | `files_transfer_jobs` | job 2 completes while job 1 stays paused | PASS on Linux (also `20261122-024836`). **FAIL on Windows** (`20261122-024919`, `20261122-025708`), finding ACC-JOB-02: job 2 (`small/` to another folder) stays Preparing, waiting for SameFolders, for 20 s while job 1 (`many/` of the same folder) is paused; it passed at 3f6fff8a (Windows `20261121-192806`) |
| T-J02 | Wait on: a third job waits for the paused one and runs after it | `files_transfer_jobs` | `EarlierTransfers`, "Queued — waiting for earlier transfers", copies only after job 1 ends | PASS (job 1's 400 files completed 2.1 s after Resume) |
| T-J03 | The Jobs window lists waiting, completed and paused jobs | `files_transfer_jobs`, `tools_jobs_copy` | row states | PASS |
| T-J04 | Jobs' Cancel job stops a job at once, without the question | `files_transfer_jobs` | Cancelled; no "Cancel this operation?" | PASS |
| T-J05 | Pause holds the transfer, Resume continues | `tools_jobs_copy`, `files_transfer_jobs` | progress unchanged while paused | PASS (both scenarios) |
| T-J06 | Cancel operation asks at a checkpoint and keeps only whole files | `tools_jobs_copy`, `files_transfer_jobs` | every kept file is whole | PASS (66 whole files kept) |
| T-J07 | No notice after a cancel | `files_transfer_jobs`, `files_transfer_errors` | no main-window notice | PASS |
| T-J08 | Totals: files and bytes (a 1 GiB file paused mid-copy) | `files_transfer_jobs` | "Files: … · Bytes: …" | PASS ("Files: 0 of 1 · Bytes: 109.8 MiB of 1.0 GiB") |
| T-J09 | Time remaining | `files_transfer_jobs`, `tools_jobs_copy` | "Time remaining:" | PASS ("Time remaining: Paused") |
| T-J10 | The speed chart's scale | `files_transfer_jobs` | "Scale:" | PASS |
| T-J11 | The current file's progress under Details | `files_transfer_jobs` | "Current file written" | PASS ("Current file written: 115081216 / 1073741824 bytes") |
| T-J12 | A per-file bar for a slow file without Details | `files_transfer_jobs` | "Current file written" before Details | PASS: the slow file's own bar showed "52%" beside its name without Details |
| T-J13 | Copied/moved/deleted wording, never "published" | `files_transfer_jobs` | no "publish" in any job window, Jobs window or notice seen | PASS |
| T-J14 | A clean completion closes its window | `files_transfer_jobs`, `tools_jobs_copy` | window gone | PASS |
| T-J16 | Jobs: Clear finished without errors keeps a job with an error; Clear all finished empties the list | `files_transfer_jobs` | rows before and after each button | PASS (the copy with an error ended CompletedWithErrors and was kept) |
| T-J15 | No internal × in a framed window's header (the title bar closes it) | `files_transfer_jobs`, `files_transfer_errors` | no button in the header's right corner of the progress, Jobs and decision windows | PASS for progress, Jobs and decision windows |

## Clipboard and drag and drop

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-X01 | Ctrl+C puts the marked files on the clipboard to copy | `files_clipboard_drag` | clipboard paths and intent | PASS |
| T-X02 | Ctrl+V copies them into the focused panel; sources stay | `files_clipboard_drag` | bytes beside kept sources | PASS |
| T-X03 | Ctrl+X, Ctrl+V moves | `files_clipboard_drag` | source gone, facts equal | PASS |
| T-X04 | A plain drag on one volume offers and does Move | `files_clipboard_drag` | feedback 4; moved | PASS |
| T-X05 | Ctrl+drag offers and does Copy | `files_clipboard_drag` | feedback 3; copied | PASS |
| T-X06 | A drop onto a folder row lands inside it | `files_clipboard_drag` | tree in the folder | PASS |
| T-X07 | Ctrl+Shift drag is refused and starts nothing | `files_clipboard_drag` | feedback 5; no job | PASS |

## Delete

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-R01 | F8's confirmation is not the permanent one and names the Trash; Escape cancels | `files_delete` | dialog; file kept | PASS |
| T-R02 | F8 of a file: the Trash holds it with a well-formed `.trashinfo` | `files_trash` | bytes, `Path=`, `DeletionDate=` | PASS |
| T-R03 | F8 of a folder keeps its tree, modes and times | `files_trash` | tree equal | PASS |
| T-R04 | F8 of a multi-selection | `files_trash` | every file in the Trash | PASS |
| T-R05 | Awkward names: percent-encoded paths, 32-byte labels | `files_trash` | five names | PASS |
| T-R06 | A read-only file keeps its mode in the Trash | `files_trash` | mode 0444 | PASS |
| T-R07 | The Trash folders are private (0700) | `files_trash` | modes | PASS |
| T-R08 | F8 of a nested tree row | `files_trash` | tree in the Trash; row gone | PASS (the t-run6 hang did not recur) |
| T-R09 | Shift+F8 of every kind, links (targets kept), a 0444 file, a multi-selection, a 12-level tree | `files_delete` | entries gone | PASS (61 entries one at a time, 4 links with their targets intact, a Ctrl+A multi-selection and the 12-level tree) |
| T-R10 | A mode-0555 folder with contents (the deleted tree) is deleted, to the Trash and permanently | `files_trash`, `files_delete` | gone without a question | PASS in both |
| T-R11 | A file of a mode-0555 folder asks Retry/Skip/Skip all/Cancel; Retry works once writable | `files_trash` | question; file in the Trash | PASS. The question offers Retry, Skip, Skip all errors, Cancel job and Decide later |
| T-R12 | Two such files: Skip all keeps both and ends the job | `files_trash` | both kept | PASS in `20261121-164146`. In `20261121-144246` the check looked for "Skip all" or "Skip all this error type", not the Delete question's "Skip all errors", and stayed PENDING (harness defect, fixed by the Linux harness patch) |
| T-R13 | Shift+F8 of a tree holding a mode-0555 folder deletes it whole | `files_trash` | tree gone | PASS |
| T-R14 | The account's own Trash is neither used nor read | `files_delete`, `files_trash` | private `XDG_DATA_HOME` | PASS (staged private `XDG_DATA_HOME`) |

Not observed at smoke scale: in `files_trash` the permanent delete of `bulk/` completed
before its window could be used, so its pause, resume and cancel were not exercised
(both Linux runs).

## Harness

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-H01 | Every work folder is deletable after its scenario | runner | no entry without owner permissions | PASS |
| T-H02 | A kept run folder is deletable | runner | same, for the whole run folder | PASS |
| T-H03 | A start with `--left` and `--right`, one of them or neither, against the real settings store in a private profile, is ready within the 10 s settings deadline; a given path wins, the other panel restores its saved tabs, which stay saved | `startup_paths` | tabs, active tab, listings, profile afterwards | PASS |

Harness defects the Linux run found in these scenarios, fixed in
(all platforms; committed with this record):
`files_transfer_errors` read the preparation question's buttons before sizing its window
(T-E10), and `files_trash` did not accept the Delete question's "Skip all errors" (T-R12).
