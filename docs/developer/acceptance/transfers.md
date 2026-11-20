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

Last runs (host, plugins `_build/try-1505`): t-run5 (startup_paths, files_move_kinds,
files_copy_options, files_transfer_jobs, files_transfer_errors, files_trash, tools_jobs_copy;
folder removed) and t-run6 (files_copy_options, files_transfer_jobs, files_transfer_errors,
files_trash; `_build/acceptance-fixes/t-runs/20261120-150424`). Status words: as above,
plus **NOT RUN** (the check exists but has not run since its last rework) and **NOT
VERIFIED** (the scenario stopped before reporting it).

## Copy/Move dialog

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-D01 | F5 shows the selection and its source folder | `files_copy_options` | summary names `a.txt` and the source folder | PASS (t-run5) |
| T-D02 | The destination is the other panel's folder | `files_copy_options` | "Destination folder" value | PASS (t-run5) |
| T-D03 | A typed existing folder is the destination | `files_copy_kinds`, `files_transfer_jobs`, `files_transfer_errors` | files arrive in the typed folder | PASS (t-run5) |
| T-D04 | A missing folder is refused in the dialog | `files_copy_options` | dialog stays open, "Enter the full path of an existing destination folder.", no job | FAIL: ACC-OPT-01, a missing folder is admitted as a job (routed to the UI agent) |
| T-D05 | A relative path is refused | `files_copy_options` | same | PASS (t-run5) |
| T-D06 | An empty destination cannot be confirmed | `files_copy_options` | Enter starts nothing | PASS (t-run5) |
| T-D07 | Escape copies nothing | `files_copy_options` | target folder empty | PASS (t-run5) |
| T-D08 | Exact modified time: offered for files, not for folders; kept to the nanosecond | `files_copy_options`, `files_copy_kinds` | check box state; times equal | PASS (t-run5) (dialog part; `files_copy_kinds` not run) |
| T-D09 | "Follow links" (Linux Copy) shown checked; cleared keeps links as links; checked follows | `files_copy_options`, `files_links` | check box; link targets and trees | PASS (t-run5) (dialog part; `files_links` not run) |
| T-D10 | Move has no link option; its policy line names renames | `files_copy_options` | no "Follow links"; policy text | PASS (t-run5) |
| T-D11 | "Wait for other operations to finish" shown, unchecked | `files_copy_options`, `files_transfer_jobs` | check box state | PASS (t-run5) |
| T-D12 | Full SHA-256 verification (default) reads every byte back | `files_copy_options` | `verified_bytes` equals the size; policy text | PASS (t-run5) |
| T-D13 | Size-and-samples verification from Options › General | `files_copy_options` | policy text; at most three 64 KiB samples read back; bytes equal | PASS (t-run5) |

## Copy and Move

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-C01 | Every fixture kind copied one at a time | `files_copy_kinds` | bytes, exact names, trees, rows | NOT RUN since the rework (check written) |
| T-C02 | Copies get the source's permission bits masked by the umask, inside copied folders too | `files_copy_kinds` | modes of every copied entry | NOT RUN since the rework (check written) |
| T-C03 | A 12-level tree in one job | `files_copy_kinds` | tree equal | NOT RUN since the rework (check written) |
| T-C04 | Many small files (3,000 at full scale) in one job | `files_copy_options`, `files_transfer_jobs` | tree equal | PASS (t-run5); 59.5 s for 3,000 files (t-run6 hit the old 60 s limit, now 5 min) |
| T-C05 | A multi-selection into a typed subfolder with exact times | `files_copy_kinds` | bytes and times | NOT RUN since the rework (check written) |
| T-C06 | Every fixture kind moved one at a time; mode and times kept | `files_move_kinds` | facts equal; source gone | PASS (t-run5) |
| T-C07 | An owned write-protected folder (mode 0555, with contents) moves as it is, without a question, and keeps its mode | `files_move_kinds`, `files_transfer_errors` | no question; tree, folder modes and source gone | PASS (t-run5) (`files_move_kinds`), PASS (t-run6) (`files_transfer_errors`) |
| T-C08 | Every entry is accounted for (moved or refused) | `files_move_kinds` | counts one at a time and in multi-selections | PASS (t-run5) |
| T-C09 | Copy to another filesystem | `files_transfer_errors` | bytes in a private folder on `$XDG_RUNTIME_DIR` | PASS (t-run6) |
| T-C10 | Move to another filesystem moves, or keeps the source untouched | `files_transfer_errors` | either outcome consistent | PASS (t-run6): Failed, source kept ("Cross-volume Move requires a qualified verified-copy destination") |
| T-C11 | Cross-volume Move's source removal clears owner write protection inside the moved tree | – | needs two qualified (ext) filesystems | SKIP: one ext filesystem on the test host |
| T-C12 | Archive members as a source: a file, a nested and an empty folder from a ZIP; the same tree from a tar.gz | `files_archive_transfers`, `viewers_archives` (V) | known bytes; the two copies equal | NOT RUN since the rework (check written) |
| T-C13 | An archive as a target is refused; F6 of a member is refused | `files_archive_transfers` | archive bytes unchanged; refusal reported; F6 notice | NOT RUN since the rework (check written) |
| T-C14 | Inline tree rows as a source, marked at different depths of two branches | `files_copy_options`, `files_tree_ops` (P), `files_trash` | each marked row copied as its own folder; nested row trashed | PASS (t-run5) (copy of marked tree rows); Trash part: see T-R08 |

## Conflicts

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-K01 | Replace keeps the original in a recovery folder | `files_conflicts` | recovery copy holds the original | NOT RUN since the rework (check written) |
| T-K02 | Skip, Skip all (one question) | `files_conflicts` | target unchanged | NOT RUN since the rework (check written) |
| T-K03 | Copy as another name | `files_conflicts` | new name holds the bytes | NOT RUN since the rework (check written) |
| T-K04 | Replace all (one question) | `files_conflicts` | all replaced, originals kept | NOT RUN since the rework (check written) |
| T-K05 | Merge folders, then Skip a child | `files_conflicts` | merged tree | NOT RUN since the rework (check written) |
| T-K06 | Merge all folders (one question) | `files_transfer_errors` | both folders merged | PASS (t-run6) |
| T-K07 | Cancel job midway keeps the earlier copy | `files_conflicts` | earlier file kept, later not started | NOT RUN since the rework (check written) |
| T-K08 | Decide later leaves the job waiting; Resolve brings the question back | `files_transfer_errors` | `WaitingForDecision`; Resolve; Skip | PASS (t-run6) |
| T-K09 | Copy into its own folder; folder into itself refused | `files_conflicts` | nothing changes | NOT RUN since the rework (check written) |

## Failures

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-E01 | A read-only destination asks Retry/Skip/Skip all this error type/Cancel job; Retry works once it is writable | `files_transfer_errors` | buttons; file copied; no further question | PASS (t-run6) |
| T-E02 | Skip all this error type: one question for three files | `files_transfer_errors` | skipped 3, nothing written | PASS (t-run6) |
| T-E03 | Cancel job at an error | `files_transfer_errors` | Cancelled, nothing written | PASS (t-run6) |
| T-E04 | An unreadable file: Retry once it is readable | `files_transfer_errors` | file copied | PASS (t-run6) |
| T-E05 | An unreadable folder in a tree: Skip and continue | `files_transfer_errors` | rest of the tree copied | PASS (t-run6) (its pending mark passes: remove it) |
| T-E06 | A Move out of a mode-0555 folder asks Retry/Skip/Skip all this error type/Cancel job; Retry once the folder is writable moves the file | `files_transfer_errors` | buttons; file moved, no further question (finding ACC-ERR-01 otherwise) | PENDING: a Move out of a mode-0555 folder ends Failed without a question (t-run6); the Retry part (ACC-ERR-01) waits for the question |
| T-E07 | The job window shows the error itself, no following dialog | `files_transfer_errors` | window text, no notice | PASS (t-run6) |
| T-E08 | Details shows more | `files_transfer_errors` | more texts after Details | PASS (t-run6) |
| T-E09 | An outcome lists only items with errors and counts notes (a skipped unreadable child folder, a link copied as a link) | `files_transfer_errors` | error path listed, no per-note sentence | counts PASS (t-run6); the check wrongly called the skipped folder's error row informational: fixed, not rerun |
| T-E10 | An unreadable selected folder fails while the transfer is prepared; Skip leaves it out and copies the rest of the selection | `files_transfer_errors` | Skip offered; the other file copied | PENDING: preparation offers only Retry, Cancel job and Decide later (routed to the backend agent) |

## Jobs and their windows

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-J01 | Wait off: a second job runs at once beside a paused one | `files_transfer_jobs` | job 2 completes while job 1 stays paused | PENDING (t-run6 reading was confounded by a harness bug, now fixed: rerun) |
| T-J02 | Wait on: a third job waits for the paused one and runs after it | `files_transfer_jobs` | `EarlierTransfers`, "Queued — waiting for earlier transfers", copies only after job 1 ends | NOT RUN since a harness fix (t-run6 mixed up job IDs when jobs reported side by side) |
| T-J03 | The Jobs window lists waiting, completed and paused jobs | `files_transfer_jobs`, `tools_jobs_copy` | row states | PASS (t-run6) |
| T-J04 | Jobs' Cancel job stops a job at once, without the question | `files_transfer_jobs` | Cancelled; no "Cancel this operation?" | PASS (t-run6) |
| T-J05 | Pause holds the transfer, Resume continues | `tools_jobs_copy`, `files_transfer_jobs` | progress unchanged while paused | PASS (t-run5) (`tools_jobs_copy`) |
| T-J06 | Cancel operation asks at a checkpoint and keeps only whole files | `tools_jobs_copy`, `files_transfer_jobs` | every kept file is whole | PASS (t-run5), PASS (t-run6) |
| T-J07 | No notice after a cancel | `files_transfer_jobs`, `files_transfer_errors` | no main-window notice | PASS (t-run6) |
| T-J08 | Totals: files and bytes (a 1 GiB file paused mid-copy) | `files_transfer_jobs` | "Files: … · Bytes: …" | PASS (t-run6) |
| T-J09 | Time remaining | `files_transfer_jobs`, `tools_jobs_copy` | "Time remaining:" | PASS (t-run6) |
| T-J10 | The speed chart's scale | `files_transfer_jobs` | "Scale:" | PASS (t-run6) |
| T-J11 | The current file's progress under Details | `files_transfer_jobs` | "Current file written" | PASS (t-run6) |
| T-J12 | A per-file bar for a slow file without Details | `files_transfer_jobs` | "Current file written" before Details | PENDING: no per-file progress before Details |
| T-J13 | Copied/moved/deleted wording, never "published" | `files_transfer_jobs` | no "publish" in any job window, Jobs window or notice seen | PASS (t-run6) |
| T-J14 | A clean completion closes its window | `files_transfer_jobs`, `tools_jobs_copy` | window gone | PASS (t-run5) |
| T-J16 | Jobs: Clear finished without errors keeps a job with an error; Clear all finished empties the list | `files_transfer_jobs` | rows before and after each button | NOT RUN since a fix (t-run6 had no job with an error, so Clear all finished was rightly disabled) |
| T-J15 | No internal × in a framed window's header (the title bar closes it) | `files_transfer_jobs`, `files_transfer_errors` | no button in the header's right corner of the progress, Jobs and decision windows | PASS (t-run6) for progress and Jobs windows; the decision window's pending mark passes: remove it |

## Clipboard and drag and drop

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-X01 | Ctrl+C puts the marked files on the clipboard to copy | `files_clipboard_drag` | clipboard paths and intent | PASS (run before 14:00 today; its folder was removed) |
| T-X02 | Ctrl+V copies them into the focused panel; sources stay | `files_clipboard_drag` | bytes beside kept sources | PASS (run before 14:00 today; its folder was removed) |
| T-X03 | Ctrl+X, Ctrl+V moves | `files_clipboard_drag` | source gone, facts equal | PASS (run before 14:00 today; its folder was removed) |
| T-X04 | A plain drag on one volume offers and does Move | `files_clipboard_drag` | feedback 4; moved | PASS (run before 14:00 today; its folder was removed) |
| T-X05 | Ctrl+drag offers and does Copy | `files_clipboard_drag` | feedback 3; copied | PASS (run before 14:00 today; its folder was removed) |
| T-X06 | A drop onto a folder row lands inside it | `files_clipboard_drag` | tree in the folder | PASS (run before 14:00 today; its folder was removed) |
| T-X07 | Ctrl+Shift drag is refused and starts nothing | `files_clipboard_drag` | feedback 5; no job | PASS (run before 14:00 today; its folder was removed) |

## Delete

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-R01 | F8's confirmation is not the permanent one and names the Trash; Escape cancels | `files_delete` | dialog; file kept | NOT RUN since the rework (check written) |
| T-R02 | F8 of a file: the Trash holds it with a well-formed `.trashinfo` | `files_trash` | bytes, `Path=`, `DeletionDate=` | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R03 | F8 of a folder keeps its tree, modes and times | `files_trash` | tree equal | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R04 | F8 of a multi-selection | `files_trash` | every file in the Trash | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R05 | Awkward names: percent-encoded paths, 32-byte labels | `files_trash` | five names | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R06 | A read-only file keeps its mode in the Trash | `files_trash` | mode 0444 | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R07 | The Trash folders are private (0700) | `files_trash` | modes | NOT VERIFIED: t-run6 stopped at T-R08 before reporting |
| T-R08 | F8 of a nested tree row | `files_trash` | tree in the Trash; row gone | FAIL (t-run6), under investigation: the preceding delete of two files in a mode-0555 folder, answered Skip all, stayed Running 0/2 for 60 s and the nested row's Trash waited behind it; the harness answered while the question was still closing (fixed: `answer_delete`), rerun to tell a product hang from the race |
| T-R09 | Shift+F8 of every kind, links (targets kept), a 0444 file, a multi-selection, a 12-level tree | `files_delete` | entries gone | NOT RUN since the rework (check written) |
| T-R10 | A mode-0555 folder with contents (the deleted tree) is deleted, to the Trash and permanently | `files_trash`, `files_delete` | gone without a question | PENDING in `files_trash` (owner write inside the deleted tree); `files_delete` part NOT RUN since the rework (check written) |
| T-R11 | A file of a mode-0555 folder asks Retry/Skip/Skip all/Cancel; Retry works once writable | `files_trash` | question; file in the Trash | PENDING (delete questions); a question appeared in t-run4, unverified since |
| T-R12 | Two such files: Skip all keeps both and ends the job | `files_trash` | both kept | PENDING (delete questions) |
| T-R13 | Shift+F8 of a tree holding a mode-0555 folder deletes it whole | `files_trash` | tree gone | PENDING (owner write inside the deleted tree) |
| T-R14 | The account's own Trash is neither used nor read | `files_delete`, `files_trash` | private `XDG_DATA_HOME` | PASS (t-run6) (staged private `XDG_DATA_HOME`) |

## Harness

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-H01 | Every work folder is deletable after its scenario | runner | no entry without owner permissions | PASS (t-run6) |
| T-H02 | A kept run folder is deletable | runner | same, for the whole run folder | PASS (t-run6) |
| T-H03 | A start with `--left` and `--right`, one of them or neither, against the real settings store in a private profile, is ready within the 10 s settings deadline; a given path wins, the other panel restores its saved tabs, which stay saved | `startup_paths` | tabs, active tab, listings, profile afterwards | PASS (t-run5) |
