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
`20261122-024919`, `20261122-025708` and `20261122-041256` (Windows 11, NTFS, smoke scale, no plugin
packages).

Wave 1 of the acceptance phase (T-D14, T-D15, T-C10, T-C15, T-L05…T-L14, T-K10…T-K13,
T-E06, T-E11, T-E12, T-J03, T-J17, T-J18, T-X08…T-X13, T-R15): the Linux run
`20261122-165133` (WSL 2, Ubuntu 24.04 on ext4, smoke scale, commit 66a02637 with this
record's suite, built after the shared build directory's path packages were cleaned; the
phase's shared plugin packages, built at b1b89343; `MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR` a
folder on the Windows drive, `MIDNIGHT_ACCEPTANCE_FAT_DIR` and
`MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR` a 64 MiB FAT and a 128 MiB ext4 image that the owner
mounted) and the Windows run `20261122-171856` (Windows 11, NTFS on a drive without 8.3
names, smoke scale, the same commit; the phase's release plugin packages;
`MIDNIGHT_ACCEPTANCE_NETWORK_DIR` an administrative share of the run's drive,
`MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS` links made in an elevated prompt), each of
`copy_move_delete`, `files_copy_kinds`, `files_move_kinds`, `files_conflicts`,
`files_links`, `files_links_volumes`, `files_delete`, `files_copy_options`,
`files_transfer_jobs`, `files_transfer_errors`, `files_clipboard_drag`,
`files_archive_transfers`, `files_trash`, `tools_jobs_copy`, `journeys_links_readonly`,
`journeys_filtered_conflicts` and `journeys_concurrent_jobs`: Linux 17 of 17 PASS;
Windows 14 PASS, 2 SKIP (`files_trash`, `journeys_links_readonly`: Linux only) and 1 FAIL
(`files_links_volumes`: finding O-1b below). Commit 0a6d5a4a fixed findings W-3, O-1 and
O-2 (below), which these runs check. Earlier runs of the same set: Linux
`20261122-070636` (commit b1b89343, before T-C15: 17 of 17 PASS) and `20261122-103036`,
Windows `20261122-103025` (commit 26c18895: O-2 not fixed yet, and T-C15 failed on a
harness defect, fixed: the other panel still showed an archive) and `20261122-141629`
(commit 0a6d5a4a: O-1b, and T-L07 on a harness defect of that adaptation, fixed). Red runs,
each new check's expectation broken in a throwaway copy: Linux `20261122-072703` (each
new check that runs on Linux: 29 failures in 8 of 8 scenarios) and `20261122-173744`
(T-C10, T-C15, T-X10, T-X12, T-X13 and the FAT Move's refusal: 14 failures in 4 of 4),
Windows `20261122-104235` (T-L07, T-L11, T-L12, T-L13 and the then per-link question: 10
failures) and `20261122-173443` (T-C15, T-L11 after the W-3 and O-1 fixes, T-X12, T-X13:
13 failures in 3 of 3). The parts that need a volume this host does not have are SKIP
with their variable.

## Copy/Move dialog

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-D01 | F5 shows the selection and its source folder | `files_copy_options` | summary names `a.txt`; the source folder is its tooltip | PASS |
| T-D02 | The destination is the other panel's folder | `files_copy_options` | "Destination folder" value | PASS |
| T-D03 | A typed existing folder is the destination | `files_copy_kinds`, `files_transfer_jobs`, `files_transfer_errors` | files arrive in the typed folder | PASS |
| T-D04 | A missing folder is refused in the dialog | `files_copy_options` | dialog stays open, "Enter the full path of an existing destination folder.", no job | PASS: ACC-OPT-01 (a missing folder admitted as a job) no longer happens |
| T-D05 | A relative path is refused | `files_copy_options` | same | PASS |
| T-D06 | An empty destination cannot be confirmed | `files_copy_options` | Enter starts nothing | PASS |
| T-D07 | Escape copies nothing | `files_copy_options` | target folder empty | PASS |
| T-D08 | Exact modified time: offered for files, not for folders; kept to the nanosecond | `files_copy_options`, `files_copy_kinds` | check box state; times equal | PASS (both scenarios) |
| T-D09 | Copy offers one **Links** choice; on a fixed local folder it shows "Keep links as links", of exactly "Copy the files links point to" and "Keep links as links", with no reason or warning line (none read by a screen reader either); the former "Follow links" check boxes are gone | `files_copy_options` | the Links box's entries, value and description once the folder's link facts arrived; no former check box | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-D10 | Move offers the same Links choice (same default on a fixed local folder); its policy line names renames | `files_copy_options` | as T-D09 for F6; policy text | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-D11 | "Wait for earlier transfers" shown, unchecked | `files_copy_options`, `files_transfer_jobs` | check box state | PASS |
| T-D12 | Full SHA-256 verification (default) reads every byte back | `files_copy_options` | `verified_bytes` equals the size; policy text | PASS (2,097,152 of 2,097,152 bytes read back; at full scale 25,165,824 of 25,165,824) |
| T-D13 | Size-and-samples verification from Options › General | `files_copy_options` | policy text; at most three 64 KiB samples read back; bytes equal | PASS (196,608 bytes read back) |
| T-D14 | Move refuses a relative path, an empty field ("Type the destination folder first.") and a missing folder as Copy does; its Cancel button closes the dialog and moves nothing | `files_copy_options` | refusal texts; dialog closed, no job, source in place | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-D15 | Tab visits the dialog's controls in the order shown (destination, Links, the exact-time option where offered, the queueing option, Cancel, the primary action) and wraps; Shift+Tab goes back | `files_copy_options` | the focused control after each Tab (Move dialog of one file) | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |

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
| T-C10 | A Move to another filesystem: one that cannot keep a replaced file recoverable (Linux: all but ext2/3/4, so tmpfs; Windows: all but NTFS on a fixed drive) refuses a Move from another drive in the dialog, before any job ("Items from another drive cannot be moved to this drive: … Copy them instead.", finding O-1); one that can moves the file or keeps the source untouched | `files_transfer_errors` (`$XDG_RUNTIME_DIR`; Windows: the private installation's drive when the run is on another one) | the refusal's words and no job, else either outcome consistent | PASS (Linux `20261122-165133`: tmpfs, refused in the dialog with "Items from another drive cannot be moved to this drive: …"; Windows `20261122-171856`: the private installation's NTFS drive takes the Move, which ends Failed with the source kept: "Move needs a protected source parent: …"); red in `20261122-173744` |
| T-C11 | Cross-volume Move's source removal clears owner write protection inside the moved tree | – | needs two qualified (ext) filesystems | SKIP: one ext filesystem on the test host |
| T-C12 | Archive members as a source: a file, a nested and an empty folder from a ZIP; the same tree from a tar.gz | `files_archive_transfers`, `viewers_archives` (V) | known bytes; the two copies equal | PASS |
| T-C13 | An archive as a target is refused; F6 of a member is refused | `files_archive_transfers` | archive bytes unchanged; refusal reported; F6 notice | PASS |
| T-C14 | Inline tree rows as a source, marked at different depths of two branches | `files_copy_options`, `files_tree_ops` (P), `files_trash` | each marked row copied as its own folder; nested row trashed | PASS (copy of marked tree rows and the nested row's Trash, T-R08) |
| T-C15 | F5 of a password-protected archive's members asks for the password ("… Enter its password to copy its protected members; without it they are skipped."), "Wrong password — try again.", Cancel skips the locked members without asking per member, the right password copies exact bytes and is asked again after leaving the archive | `files_archive_transfers` (`locked.zip`: a ZipCrypto and a deflated AES-256 member and a plain one, written by `encode::zip_entries`) | the dialog's sentence before any Copy dialog; Cancel: only the plain member arrives, no question; a wrong password by keyboard refused in the dialog, the right non-ASCII one by pointer: every member's bytes; F5 again while the archive is open asks nothing; left and entered again it asks (Escape: nothing copied) | PASS (Linux `20261122-165133`, Windows `20261122-171856`: without the password the job ends Completed with errors, each protected member skipped and counted); red in `20261122-173744` and `20261122-173443` |

## Links

The Links choice of Copy and Move (`files_links`; its default and wording on other kinds
of destination in `files_links_volumes`). Linux uses the fixture's symbolic links
(`links/to-folder`, `to-file`, `dangling`, `to-self`, all relative); Windows uses
directory junctions to a folder, to a missing folder and to their own folder (file
symbolic links need Developer Mode or administrator rights).

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-L01 | Keep links as links (the default on a fixed local folder) recreates each link with the same target: Linux symbolic links with the same text, Windows junctions to the same folder; the job details name each kept link whose relative target leads outside the copied items | `files_links` | `read_link` of each copy; the notes ("… which leads outside the copied items") for `../` targets only | PASS (Linux `20261122-024836`, Windows `20261122-024919`) |
| T-L02 | Copy the files links point to: a folder link arrives as the folder's tree, a file link as the file; a dangling link stays a link with its reason in the job details; a selected link to its own folder is followed once and the links met again inside that copy stay links, each with its reason (Linux and Windows alike); a folder holding only a link to itself copies once | `files_links` | trees, bytes, `read_link`, notes ("its target does not exist", "contains it") | PASS on Linux (`20261122-024836`). Windows (`20261122-024919`): the folder and dangling junctions PASS; the junction to its own folder was followed once, against the check's expectation, which was wrong (the product does so on Linux too, and its engine test asserts it); the corrected check PASS in `20261122-041256` |
| T-L03 | The Links choice by keyboard (Tab to it, Up or Down, Enter from the box), by pointer (a click on the entry) and with the open list's keys (Enter in the list chooses the entry and never confirms the dialog) | `files_links` (each pass uses one way) | the dialog stays open with no job until confirmed; the choice shown when confirmed | PASS (Linux `20261122-024836`: keyboard, pointer and the list's keys; Windows `20261122-024919`: keyboard and pointer) |
| T-L04 | F6 with the offered choice (keep links) moves each link itself, the folder it points to unchanged | `files_links` | the moved link's target; source gone | PASS (Linux `20261122-024836`, Windows `20261122-024919`: all three junctions) |
| T-L05 | F6 of a folder holding a folder link, a dangling link, `plain.txt` and (Linux) a file link, with "Copy the files links point to": copies of what the folder and file links point to arrive, the dangling link moves as a link (noted), the folder leaves the source and is not renamed whole; `plain.txt` is renamed; what the links point to keeps its bytes and identity | `files_links` | trees and bytes; identities (device and inode; Windows volume serial number and file index) before and after | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-L06 | F6 of the same folder keeping links (as offered) on one volume renames it whole: it and every entry in it keep their identity, the links their targets | `files_links` | identities; `read_link` | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-L07 | A Move keeping links to another volume recreates each link with its target, removes the originals and leaves what they point to | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_OTHER_EXT_DIR`; Windows: the private installation's drive when the run is on another one) | `read_link`; source gone; identities of the targets | PASS on Linux (`20261122-165133`: an ext4 image) and on Windows (`20261122-171856`: from the private installation's drive to the run's); red in `20261122-104235` |
| T-L08 | FAT/exFAT: the dialog proposes "Copy the files links point to" with "This drive cannot store links (FAT…), so the files they point to are copied."; choosing "Keep links as links" shows "This drive cannot store links (FAT…): each link is left out, and the job details name it."; the first Enter of a Copy holds with that warning and "Select Copy again to keep links anyway, or choose “Copy the files links point to”.", the second starts it, and no link arrives: Windows leaves each link out without a question, names it in the job details and completes (as T-L11); Linux refuses the destination for the files (no O_TMPFILE). A Move from another drive is refused in the dialog ("Items from another drive cannot be moved to this drive (FAT…): …", O-1) | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_FAT_DIR`) | offered view, warning (shown and read), hold text, links at the destination; the job's outcome, notes and questions; the Move's refusal | PASS on Linux (`20261122-165133`: the Copy's only question is the O_TMPFILE refusal; the Move refused in the dialog with "… to this drive (FAT): …"); red of the Move's refusal in `20261122-173744`. Windows SKIP: `MIDNIGHT_ACCEPTANCE_FAT_DIR` not set (no FAT drive) |
| T-L09 | A removable drive that stores links: the dialog proposes copying the files with "Removable drive: the files links point to are copied, so that no link there points back to this computer."; a selection already on that drive keeps links without a reason | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR`) | offered views | SKIP: `MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR` not set (needs a removable drive; a loop device is not one) |
| T-L10 | A project moved to a removable drive with the defaults arrives without links (only the dangling one, which cannot be followed, moves as a link); what its links pointed to stays, unchanged. A drive that cannot keep a replaced file recoverable refuses the Move in the dialog instead (O-1) | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR`) | links at the destination; source gone; identities; or the refusal and the project in place | SKIP: `MIDNIGHT_ACCEPTANCE_REMOVABLE_DIR` not set (as T-L09) |
| T-L11 | A Windows network location: "Network location: no links are created there, so the files they point to are copied.", the warning "No links are created at this network location: …" and the hold of the first Enter; the Copy then leaves each link out without a question, names it in the job details ("<path> was left out: …"), counts it and completes with "2 links left out where no links are created; the job details name each." (finding W-3). A Move from another drive there is refused in the dialog: "Items from another drive cannot be moved to a network location: Move cannot verify their copies there before removing the originals. Copy them instead." (finding O-1) | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_NETWORK_DIR`) | as T-L08; the job's state, `links_left_out`, notes and outcome line; the Move's refusal, no job, the project in place | Copy: PASS on Windows (`20261122-171856`: no question, 2 links left out and named, Completed with the summary line; W-3 fixed); red in `20261122-173443`. Move: **FAIL**, finding O-1b (`20261122-171856`, also `20261122-141629`): the dialog holds with the links warning instead of refusing the Move. Linux SKIP (only a Windows network location copies the files) |
| T-L12 | A typed folder whose default differs from the one shown holds the first Enter with "The link handling changed for this folder. <reason> Select Copy again to start." and shows its default; the reverse says "Links are kept as links here."; a choice the user made never holds | `files_links_volumes` (with FAT, removable or network) | hold text and the choice while held | PASS on Linux with the FAT drive (`20261122-165133`) and on Windows with the network location (`20261122-171856`); red in `20261122-104235` |
| T-L13 | Windows symbolic links kept as links: recreated where this process may create them; otherwise a directory link becomes a junction with "…was created as a junction to …, because creating a symbolic link needs Developer Mode or administrator rights" and a file link asks "Creating a symbolic link to a file needs Developer Mode or administrator rights…" with Skip and Skip all, no Retry | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_WIN_SYMLINKS`: prepared links) | `read_link`, note, question | PASS on Windows (`20261122-171856`, also `20261122-103025`: without Developer Mode the directory link became a junction with its note, the file link was asked about with Skip and Skip all and no Retry); red in `20261122-104235`. Linux SKIP (Windows links) |
| T-L14 | A Linux destination without O_TMPFILE refuses a Copy in words ("This filesystem cannot create anonymous O_TMPFILE copies; …") and nothing arrives | `files_links_volumes` (`MIDNIGHT_ACCEPTANCE_NO_TMPFILE_DIR`) | question or outcome text | PASS on Linux (`20261122-165133`: a folder on the Windows drive as WSL mounts it); red in `20261122-072703`. Windows SKIP (a Linux requirement) |

Finding W-3 (the dialog and the job disagree; Windows run `20261122-052836` and T-L11's
runs): keeping links at a Windows network location, the dialog warns "No links are
created at this network location: each link is left out, and the job details name it."
and the first Enter holds with "… Select Copy again to keep links anyway, or choose “Copy
the files links point to”."; after the second Enter the job stops at the first link with
"Copy/Move needs attention": "No links are created at a network location, so this link is
left out" (Skip, Skip all this error type, Cancel job, Decide later). The question is the
documented behavior ([TRANSFERS.md](../TRANSFERS.md): a link the destination cannot create
is reported with Skip and Skip all); the warning promises that each link is simply left
out. A FAT drive takes the same path ("This drive cannot store links (FAT32), so this link
is left out"), not run here. Decided (Architect): the warning is the contract; such links
are left out and named in the job details, the job never asks about each one, and
drag and drop and paste follow the same rule. Fixed in 0a6d5a4a (the job counts and names
each link it leaves out, TRANSFERS.md); T-L08 and T-L11 check the fixed behavior.

Finding O-1 (a Move admitted where it cannot happen; same runs): F6 of the project to the
network location shows the Move's warning ("… each link is left out and stays where it is,
…") and holds; after the second Enter the job asks "Cross-volume Move requires a qualified
verified-copy destination; the source was retained." for the whole selection. Decided
(Architect): such a Move is refused in the dialog, saying why and suggesting Copy; no job
refuses it afterwards. Fixed in 0a6d5a4a for F6, a pasted Cut and a drag; checked by T-L08,
T-L10, T-L11, T-C10 and T-X10.

Finding O-1b (T-L11, Windows `20261122-141629` and `20261122-171856`): F6 of the project
to the network location, an administrative share of the run's own drive
(`\\localhost\D$\…`), is not refused: the dialog holds with the links warning ("No links
are created at this network location: each link is left out and stays where it is, …
Select Move again to keep links anyway, …"), and before the fix a second Enter started a
job that refused the Move ("Cross-volume Move requires a qualified verified-copy
destination; the source was retained.", `20261122-103025`). The dialog decides that source
and destination share a volume by the volume serial number, which a share of the local
drive reports as the drive's own. Decided (Architect): one same-volume determination
serves both the dialog's facts and the Move's admission. Open; T-L11's Move part fails
until it lands.

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
| T-K10 | Replace all keeps each replaced original for recovery, once | `files_conflicts` | the recovery copies added by the run | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-K11 | Move Replace keeps the replaced original for recovery, and the job reports where | `files_conflicts` | the recovery copy added by the run (Linux: in the Move's private `.midnight-move-*` stage beside the source; Windows: a `.midnight-recovery-*` folder beside the destination); the job's one reported recovery path holds the original | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703`. In `20261122-052952` the check looked only for `.midnight-recovery-*` folders and missed the Linux stage (harness defect, fixed) |
| T-K12 | Move onto an existing folder with Merge folders moves its entries in beside the existing ones and the folder leaves the source | `files_conflicts` | merged tree; source gone | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-K13 | Escape and the window's × in a question act as Decide later; Resolve on the job's row in the Jobs window (opened with its window's Jobs button) brings the question back | `files_transfer_errors` | `WaitingForDecision`; the question again; Skip | PASS for Escape and the window's × (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |

## Failures

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-E01 | A read-only destination asks Retry/Skip/Skip all this error type/Cancel job; Retry works once it is writable | `files_transfer_errors` | buttons; file copied; no further question | PASS |
| T-E02 | Skip all this error type: one question for three files | `files_transfer_errors` | skipped 3, nothing written | PASS |
| T-E03 | Cancel job at an error | `files_transfer_errors` | Cancelled, nothing written | PASS |
| T-E04 | An unreadable file: Retry once it is readable | `files_transfer_errors` | file copied | PASS |
| T-E05 | An unreadable folder in a tree: Skip and continue | `files_transfer_errors` | rest of the tree copied | PASS |
| T-E06 | A Move out of a write-protected folder (Linux mode 0555; Windows deny entries) asks once, while it is prepared, with Retry, Skip, Skip all this error type, Cancel job and Decide later, and says why in words; Retry once the folder is writable moves the file | `files_transfer_errors` | buttons; the reason (Linux "Permission denied: it cannot be moved out of its folder, which is write-protected …", Windows "… the permissions of the item and of its folder do not allow removing it. …"); file moved, no further question (finding ACC-ERR-01 otherwise) | PASS, strict (Linux `20261122-165133`, Windows `20261122-171856`): ACC-ERR-01 does not recur; red in `20261122-072703` |
| T-E11 | T-E06 with Skip: the file stays with its bytes, nothing arrives, the job ends Failed (skipped 1, failed 1) with the item's reason and "1 entry or subtree skipped; source data retained." | `files_transfer_errors` | outcome lines; source and destination | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703`; both PASS runs predate the W-2 fix. Finding W-2 (wording) fixed: one item now reads "1 entry or subtree" (engine test `one_skipped_item_is_counted_in_the_singular`) |
| T-E12 | A selected file removed while the dialog is open: the question says "removed from the source while copying" ("… while moving" for F6), without the system's text; Skip ends the job Failed with the skipped line | `files_transfer_errors` | question text, outcome | PASS for Copy and Move (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-E07 | The job window shows the error itself, no following dialog | `files_transfer_errors` | window text, no notice | PASS |
| T-E08 | Details shows more | `files_transfer_errors` | more texts after Details | PASS |
| T-E09 | An outcome lists only items with errors and counts notes (a skipped unreadable child folder, a link copied as a link) | `files_transfer_errors` | error path listed, no per-note sentence | PASS: "1 skipped item was not copied." and "1 link was copied as a link; Details says why." |
| T-E10 | An unreadable selected folder fails while the transfer is prepared; Skip leaves it out and copies the rest of the selection | `files_transfer_errors` | Skip offered; the other file copied | PASS in `20261121-164146`. In `20261121-144246` the check read the question's buttons before sizing its window, saw only Decide later and answered Cancel job (harness defect, fixed by the Linux harness patch) |

## Jobs and their windows

| ID | Behavior | Scenario | Check | Status |
| --- | --- | --- | --- | --- |
| T-J01 | Wait off: a second job runs at once beside a paused one | `files_transfer_jobs` | job 2 completes while job 1 stays paused | PASS on Linux (also `20261122-024836`, `20261122-165133`) and on Windows on a drive without 8.3 names (`20261122-052836`, `20261122-171856`). ACC-JOB-02 (Windows `20261122-024919`, `20261122-025708`: job 2 stayed Preparing beside the paused job 1, its wait shown as SameFolders) was the environment: those run folders were on a drive that creates 8.3 short names, where the folders' identity cannot be confirmed and jobs run one at a time by design. Since e942d006 such a wait says so (UnconfirmedFolders: "Waiting for another job: jobs whose folders cannot be confirmed run alone") |
| T-J02 | Wait on: a third job waits for the paused one and runs after it | `files_transfer_jobs` | `EarlierTransfers`, "Queued — waiting for earlier transfers", copies only after job 1 ends | PASS (job 1's 400 files completed 2.1 s after Resume) |
| T-J03 | The Jobs window, opened with the function bar's Jobs button, lists waiting, completed and paused jobs | `files_transfer_jobs`, `tools_jobs_copy` | row states | PASS, opened by the function bar's Jobs button (Linux `20261122-165133`, Windows `20261122-171856`) |
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
| T-J17 | A paused job's time remaining holds still and says so ("Paused", or "About … (paused)" once estimated) | `files_transfer_jobs` | the "Time remaining:" line of the paused job 1 | PASS: "Time remaining: Paused" (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-J18 | Cancel operation of a Move that waits for an earlier transfer asks "Cancel this operation?" ("This does not undo completed changes.", Continue); Escape keeps the job waiting; Cancel operation then ends it Cancelled with "Move cancelled; nothing was moved." and nothing moves | `files_transfer_jobs` | window texts, job state, disk | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |

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
| T-X08 | Escape during a drag ends it and drops nothing | `files_clipboard_drag` | drag inactive after Escape; no dialog or job within 2 s of the release | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-X09 | A drop on a file row lands in that panel's folder, beside the file | `files_clipboard_drag` | the copy beside the untouched file | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-X10 | Across volumes a plain drag offers and does Copy; Shift+drag offers Move where the other volume takes a Move from another drive (Windows: the private installation's NTFS drive), which moves the file or keeps the source untouched; where it cannot keep a replaced file recoverable (Linux: tmpfs), the drag refuses the Move up front ("Not allowed here") and starts nothing (finding O-1) | `files_clipboard_drag` (`$XDG_RUNTIME_DIR`; Windows: the private installation's drive when the run is on another one) | feedback 3, and 4 or 5; jobs; disk | PASS (Linux `20261122-165133`: tmpfs, feedback 5 "Not allowed here", no job, source kept; Windows `20261122-171856`: the NTFS drive takes the Move, which ends Failed with the source kept); red in `20261122-173744` |
| T-X11 | F5 with two marked files during a filename search closes the search and copies both marked files, not the search's match | `files_clipboard_drag` | search inactive; the two copies | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-072703` |
| T-X12 | More selected items than the 64 a Move once took: F5 of 65 marked files copies all of them, Ctrl+X, Ctrl+V of them in an empty folder moves all of them there, and F6 from there moves all of them back | `files_clipboard_drag` | each job Completed; the trees equal the files as they were; the emptied folders empty | PASS (Windows `20261123-011352`); the refusal it replaced was red in `20261122-072703` and `20261122-173744`. Linux not run. A drag of 65 is not covered |
| T-X13 | A drop released within 0.4 s of the press is cancelled ("Drag cancelled: released within 0.4 s" on the source panel's status line), starts no dialog or job and moves nothing; one released later moves | `files_clipboard_drag` (the scenario clock held: press, 0.15 s, drag to the other panel, then 0.1 s or 0.3 s before the release) | the note; jobs; disk | PASS (Linux `20261122-165133`, Windows `20261122-171856`); red in `20261122-173744` and `20261122-173443` |

Finding O-2 (T-X12) is superseded: a Move no longer has a limit of its own and takes as
many selected items as Copy (up to 1,000,000), so T-X12 now checks that F6 and Ctrl+X,
Ctrl+V of 65 items move all of them (a drag of 65 is not covered).

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
| T-R15 | F8 of a file on another filesystem than the Trash is refused ("home trash is on another filesystem; cross-filesystem trash is unsupported") and the file stays: a failed Trash is never a permanent delete | `files_trash` (tmpfs `$XDG_RUNTIME_DIR`) | the refusal's words; the file's bytes; the Trash unchanged | PASS on Linux (`20261122-165133`); red in `20261122-072703`. Windows SKIP (`files_trash` is Linux only) |
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

Wave 1's own: `files_conflicts` looked for a Move Replace's original only in
`.midnight-recovery-*` folders, while Linux keeps it in the Move's private stage beside the
source (T-K11, `20261122-052952`); the check now counts both and reads the location the job
reports. In the same run `files_archive_transfers` failed on the environment: the run's
checkout had been copied from the Windows drive with every file writable by all, so the
plugin host refused its staged packages ("Plugin path is writable or replaceable by
another account"). Linux results of Wave 1 before `20261122-070636` are not counted: the
shared build directory could reuse another checkout's compiled workspace packages, and
those runs were built before it was cleaned.
