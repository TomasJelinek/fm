# Acceptance suite — file operations and panel features (2026-11-20)

Scope: the `files_*` scenarios of the opt-in acceptance suite (see
[the acceptance suite](../developer/ACCEPTANCE.md)): Copy and Move of every fixture
entry kind, conflicts, links, Delete, Rename, New file and folder, Attributes and
times, selection, sorting, the panel filter and filename search, navigation and
history, tabs, Hot Paths, Make File List, the inline tree and a busy folder. Every
result below names the command that produced it; a scenario's existence is not
evidence that it passed.

## Environment

Linux host without the GUI development packages, so Cargo ran in the project's Docker
build image (`midnight-explorer-build:9049146550…`) with the worktree mounted read-only
at `/workspace`, the shared `_build` and `.tools` writable, `CARGO_INCREMENTAL=0` and a
session-specific `--config` that gives this crate its own artifact hashes. Several other
sessions built and ran concurrently into the same target directory. The suite runs as
the invoking (non-root) user, so permission checks behave as for an ordinary account.

## How the scenarios work

Each scenario stages what it changes into its own `work/<scenario>` folder (copies of
fixture folders, or files it writes itself) and drives the application only with keys,
clicks and pointer moves. Shared helpers live in `tests/acceptance/files_support.rs`:
the F5/F6 dialog with its options and the decision window (Replace, Replace all, Skip,
Skip all, Merge, Copy as, Cancel job), Shift+F8, the F2/F7/Shift+F4 name dialog,
notices (read from the notice presentation; the message properties keep the last
notice after it closes), on-disk facts (content hash, permission bits, modified time;
equality ignores the access time, which hashing changes), timings of "report to row
listed" and, for slow transfers, the job's state timeline. A finding that is a missing
or weak feature rather than a wrong result is recorded as a review flag on a
screenshot, so the scenario reports `WARN`; `--strict-review` turns those into
failures. Failure messages of a recorded finding start with its id (`[AF-n]`).

## Commands

All runs used the Docker invocation of the task brief with
`--config /workspace/_build/tmp/coord/acc-files.toml` and
`MIDNIGHT_ACCEPTANCE_RUN_DIR=/workspace/_build/acceptance-acc-files/<UTC run id>`:

```text
cargo test --workspace --locked --config /workspace/_build/tmp/coord/acc-files.toml \
  --features midnight-acceptance/acceptance --test acceptance -- --scale smoke --keep-fixture files_
cargo clippy --workspace --locked --config /workspace/_build/tmp/coord/acc-files.toml \
  --features midnight-acceptance/acceptance --test acceptance -- -D warnings
```

The final runs built once with `cargo test … --test acceptance --no-run` and ran the
printed `_build/debug/deps/acceptance-<hash>` binary directly in the same container,
so no Cargo lock was held while the scenarios ran.

## Results

Clippy on the acceptance target (the command above, after the last source change):
no warnings, exit 0.

Smoke scale, every `files_` scenario (`acceptance-<hash> --scale smoke --keep-fixture
files_`, run `20261120-023819`), 9 min 50 s:

| Scenario | Status | Duration | Shots | Flags | Why not PASS |
| --- | --- | --- | --- | --- | --- |
| `files_copy_kinds` | WARN | 132.2 s | 11 | 1 | AF-2 |
| `files_move_kinds` | PASS | 156.5 s | 9 | 0 | |
| `files_conflicts` | FAIL | 40.4 s | 4 | 0 | AF-1 (three times) |
| `files_links` | FAIL | 25.0 s | 5 | 0 | AF-3 (eleven cases) |
| `files_delete` | PASS | 108.5 s | 9 | 0 | |
| `files_rename` | PASS | 15.5 s | 3 | 0 | |
| `files_create` | FAIL | 14.7 s | 2 | 0 | AF-5 |
| `files_attributes` | PASS | 17.9 s | 5 | 0 | |
| `files_selection` | WARN | 20.0 s | 3 | 1 | AF-6 |
| `files_sorting` | PASS | 5.6 s | 4 | 0 | |
| `files_filter_search` | WARN | 7.4 s | 4 | 1 | AF-4 |
| `files_navigation` | WARN | 12.5 s | 4 | 1 | AF-8 |
| `files_tabs` | PASS | 5.9 s | 2 | 0 | |
| `files_hot_paths` | PASS | 4.9 s | 1 | 0 | |
| `files_file_list` | PASS | 3.1 s | 1 | 0 | |
| `files_tree_ops` | PASS | 8.0 s | 3 | 0 | |
| `files_busy_folder` | PASS | 11.9 s | 1 | 0 | |

Every FAIL and WARN is a recorded finding below; no scenario failed for another
reason. The two smoke runs before it (`20261120-013806`, `20261120-015713`) gave the
same results apart from durations, except that in the first `files_hot_paths` failed
on a menu entry scrolled out of view (fixed in the scenario by scrolling the menu) and
`files_navigation` passed because its AF-8 flag did not exist yet.

What the passing scenarios verified, beyond their table rows in ACCEPTANCE.md:

- Copy: 71 entries one at a time (bytes, exact names including NFD/NFC, emoji, RTL,
  leading/trailing spaces, the 224-byte name; folder trees), the 12-level tree, and 63
  files as per-folder multi-selections with "Preserve exact modified time" (equal to
  the nanosecond). Median 431 ms from Enter to the terminal report and 248 ms from the
  report to the new row; completed roots are deselected; sources unchanged.
- Move: 66 entries one at a time and 69 as multi-selections, content, exact names,
  modes and modified times kept (same-volume renames). A mode-0555 folder cannot be
  moved into another folder (its `..` entry would change); the error is reported in
  the decision window and nothing changes (see AF-7 for its wording).
- Delete: 61 entries, 4 links (link removed, the linked folder and file unchanged),
  `sizes/` by Ctrl+A including the large file, the 12-level tree; the mode-0444 file
  is deleted, the mode-0555 folder is refused with "Permission denied" and left intact;
  F8 shows "Move to Trash" (not dangerous), Escape cancels it, and the account's Trash
  folder is unchanged.
- Rename: Unicode (listed 158 ms after the report, cursor on it), case-only, collision
  (decision window; Skip keeps both, Replace takes the name and keeps the original in
  recovery), `x/y.txt` and a 256-byte name refused with a notice, empty name keeps the
  dialog open, 255-byte name accepted, folder renamed with its contents.
- Create: single-name F7 listed at once with the cursor on it; nested segments and `..`
  created; an existing folder name refused with "File exists"; Shift+F4 with nested
  segments refused ("Enter a path for directory creation, or a single name for rename
  and empty-file creation."); Shift+F4 on an existing file asks, Escape keeps it, the
  Replace file button empties it (no notice in this snapshot); leading and trailing
  spaces kept exactly.
- Attributes: Group write set and Other read cleared (0644 → 0660), octal 444, Modified
  2021-03-04 05:06:07.123456789 +02:00 and Accessed 2020-01-02 03:04:05 Z read back
  exactly, three files at once, a folder (its child unchanged); no notice or window
  stays open after Apply.
- Busy folder: a writer created, appended to and deleted `w-*.tmp` about every 25 ms
  (239 steps in 6 s): in 37 sampled frames no loading state, the cursor and the two
  marks never moved, the 122 unchanged rows kept identity, size and time; a Rename was
  listed 95 ms and a Copy 285 ms after their reports; afterwards the listing (130
  entries) equals the disk.
- Panels: sorting by name, extension, modified time and size in both directions;
  Masks `*.txt;*.TXT`, `*.*|.*`, Expression `not name like ".*"` and Show all; search
  "lo" moved to the first of its two matches, Down to the second, Escape restored the
  list; history back/forward, filesystem root, typed path, breadcrumb, Ctrl+U twice,
  Ctrl+R keeping cursor and mark; Ctrl+T, Ctrl+Shift+Tab, tab click, the close button
  hidden until the pointer is over its tab, the + button, Ctrl+W and the last tab kept;
  Ctrl+Shift+Plus saved Hot Path 1, the toolbar menu opened it, Manage renumbered it to
  2, removed it and Add current saved the root; Make File List previewed
  `a.txt   3` / `bb.log  10` (tabs expanded) and wrote `a.txt\t3\r\nbb.log\t10\r\n`
  exactly; the inline tree expanded and collapsed and copied, renamed (row kept at depth
  3, cursor on it) and deleted a folder three levels down.

Full scale, the whole suite (`acceptance-<hash> --scale full`, run `20261120-024909`,
same binary as the smoke run above), 936.6 s: 18 passed, 5 warned, 3 failed of 26.

| Scenario | Status | Duration | Shots | Flags |
| --- | --- | --- | --- | --- |
| `startup` | PASS | 1.2 s | 1 | 0 |
| `navigation` | PASS | 39.2 s | 13 | 0 |
| `copy_move_delete` | PASS | 7.9 s | 4 | 0 |
| `create_rename` | PASS | 6.6 s | 5 | 0 |
| `tree_expand_all` | PASS | 2.2 s | 1 | 0 |
| `folder_sizes` | PASS | 1.6 s | 1 | 0 |
| `find_by_mask` | PASS | 3.7 s | 2 | 0 |
| `checksums` | PASS | 9.3 s | 8 | 0 |
| `files_copy_kinds` | WARN (AF-2) | 159.0 s | 11 | 1 |
| `files_move_kinds` | PASS | 132.5 s | 9 | 0 |
| `files_conflicts` | FAIL (AF-1) | 42.6 s | 4 | 0 |
| `files_links` | FAIL (AF-3) | 24.4 s | 5 | 0 |
| `files_delete` | PASS | 69.0 s | 9 | 0 |
| `files_rename` | PASS | 20.7 s | 3 | 0 |
| `files_create` | FAIL (AF-5) | 12.1 s | 2 | 0 |
| `files_attributes` | PASS | 23.0 s | 5 | 0 |
| `files_selection` | WARN (AF-6) | 14.6 s | 3 | 1 |
| `files_sorting` | PASS | 13.3 s | 4 | 0 |
| `files_filter_search` | WARN (AF-4) | 7.4 s | 4 | 1 |
| `files_navigation` | WARN (AF-8) | 15.6 s | 4 | 1 |
| `files_tabs` | PASS | 6.5 s | 2 | 0 |
| `files_hot_paths` | PASS | 4.8 s | 1 | 0 |
| `files_file_list` | PASS | 4.1 s | 1 | 0 |
| `files_tree_ops` | PASS | 8.2 s | 3 | 0 |
| `files_busy_folder` | PASS | 15.8 s | 1 | 0 |
| `ui_review` | WARN | 289.7 s | 186 | 19 |

The failure messages of the three failing scenarios are the same recorded findings as
at smoke scale; `ui_review`'s flags belong to the UI review, not to this area. With the
24 MiB `sizes/huge.bin` the median F5 was 454 ms and the median F6 415 ms; the slow
copies (2–4.2 s, files of 13 B to 13 KiB) spent most of their time in the Syncing phase,
the one slow move (`names/.hidden`, 11.5 s) in Running (AF-9).

## Findings

Severity is a first estimate; owners decide. Paths and lines refer to this snapshot.

| Id | Severity | Area | Steps | Expected | Actual | Suspected source |
| --- | --- | --- | --- | --- | --- | --- |
| AF-1 | Medium | Copy conflicts | F5 onto an existing name, then Skip in the decision window (also Skip all, and Merge folders followed by Skip of a child). | A skip the user chose completes the job; at most a neutral summary. | The job ends `CompletedWithErrors` and a modal notice titled "Copy failed" says "\n1 entries or subtrees skipped; source data retained." (empty first line). Cancel job's notice body starts with an empty line too. Evidence: the `files_conflicts` failure message. | `crates/midnight-engine/src/copy_worker.rs:1891` (a skip makes the state `CompletedWithErrors`) and `:1909-1913` (the message starts with `\n` when there is no error text); `crates/midnight-ui/src/presenter/copies.rs:323-335` (any `CompletedWithErrors` with a message is titled "Copy failed"). |
| AF-2 | Medium | Copy metadata | F5 of `run.sh` (0755), `shared.txt` (0664), `private.txt` (0600) and `tools/` (0750); also every fixture file (0644) and folder (0755). | Permissions are not promised (TRANSFER_OPTIONS.md), but a new file normally gets the source mode limited by the umask, as `cp` and other file managers do. | Every copied file is 0600 and every copied folder 0700; a copied script is no longer executable and shared files become private. Review flag on `files_copy_kinds/copied-modes`. | `crates/midnight-platform-linux/src/transfer_destination.rs:309` (`O_TMPFILE` stage created `RUSR \| WUSR`) and `:520` (`mkdirat` with `RWXU`); nothing applies a mode after publication. |
| AF-3 | Medium | Links | F5 of `links/to-folder`, `to-file`, `dangling` and `to-self`, with "Follow directory links" checked (the default) and cleared; F5 of a folder holding only a link to itself; F6 of each link. | The task's requirement: a link is transferred as a link with the same target, never followed (dangling and cycle included). | F5 never creates a link: the file link fails ("link target is not a directory"), the dangling link fails ("No such file or directory"); the folder link is followed (its 23-entry target tree is copied) or, with the box cleared, replaced by an empty folder; `to-self` gives an empty folder plus an error decision; the self-link-only folder stops with "directory link target is an active ancestor" (no endless nesting). F6 moves the folder link as a link but refuses the file, dangling and self links ("destination is inside the source directory" for `to-self`). Sources stay unchanged. The documented scope (TRANSFERS.md, "Current limits"): link-preserving Copy exists only behind the engine-internal `CopyLinkPolicy::Preserve`, not in the UI. | `crates/midnight-ui/src/copy_draft_windows.rs:21` (follow defaults to checked), `crates/midnight-ui/src/presenter/copy_dialog.rs:601` (the UI sends only Follow/EmptyDirectories); file links are refused by the directory-link materializer, `crates/midnight-platform-linux/src/transfer_directory_links.rs:433`. |
| AF-4 | Low | Panels | Look for a hidden-files toggle (keys, F10 menu, Options). | A way to hide dot-names, as file managers offer. | None exists; `.hidden` and `.hidden-folder` are always listed. Only a panel filter (`*.*\|.*` or the Expression `not name like ".*"`) hides them. Review flag on `files_filter_search/hidden-listed`. | Feature not implemented (no preference or action in `crates/midnight-core/src/shortcuts/catalog.rs`). |
| AF-5 | Low | New folder | F7 `alpha/beta/gamma` in a panel whose cursor is on another row. | The created `alpha` (the listed ancestor of the new path) gets the cursor, as a single-name F7 does. | The cursor stays where it was. | `crates/midnight-ui/src/presenter/operations.rs:547-553` reveals `renamed_to` only when its parent is the panel folder, so a nested path is never revealed. |
| AF-6 | Low | Selection | Numeric keypad +, − and * in the file list. | Select by mask, deselect by mask, invert (Total Commander and Salamander conventions). | Each starts a filename search for "+", "-" or "*"; there is no select/deselect-by-mask action anywhere (F10 → Selection lists none). Review flag on `files_selection/keypad-plus`. | Feature not implemented. |
| AF-7 | Low | Move errors | F6 of the mode-0555 folder `readonly/locked-folder`; F6 of a folder into itself or its subfolder. | A message naming the real cause: the source folder is read-only; the destination is inside the source. | "Creating a destination folder — Permission denied (os error 13) … Operation: copy file" although the destination is writable; the refused self-move says "destination is inside the source directory … Operation: copy file". Both are Moves of folders. Nothing changes on disk. | Every Linux transfer error is built as operation "copy file", Move included: `crates/midnight-platform-linux/src/transfer.rs:253`. |
| AF-8 | Low | Properties | Alt+Enter on a folder (Linux). | Useful properties: type, size or contents, times, permissions, owner. | Linux has no system dialog (`PropertyService::show_properties` returns false), so a notice titled with the path says only "1 items / 0 bytes in 0 regular files / 1 items without a file length". Review flag on `files_navigation/properties`. | `crates/midnight-platform-linux/src/desktop.rs:353`; summary text in `crates/midnight-engine/src/desktop.rs:779-819`. |
| AF-9 | Observation (perf) | Copy, Move, Delete | Single-file F5, F6 (a same-volume rename) and Shift+F8 while the host compiled for other sessions. | Sub-second, as the medians (about 430–450 ms, 415–422 ms and 600 ms). | Outliers: F5 of 8 B–13 KiB files 2–7.6 s, mostly in the Syncing phase; F6 2–14.5 s, Running from about 0.4 s to the end with no wait reason (a native Move syncs three directories, `crates/midnight-platform-linux/src/transfer_native_move.rs:634-643`); one Shift+F8 of 23 s (`text/crlf.txt`, no timeline recorded). Consistent with slow syncs under heavy concurrent writes; not attributable without profiling on a quiet machine. For the Perf agent. | — |

Observations that are not findings:

- A typed path to a missing folder shows "Cannot open folder: No such file or directory
  (os error 2)" and the panel ends on the nearest existing ancestor (navigation
  fallback), not on the folder it showed before.
- Copying a file into its own folder asks "Replacement and merging are unavailable for
  these items. Choose another name or skip."; Copy as works.
- Replace keeps each displaced original in `.midnight-recovery-*/item` next to the
  target; those folders are listed in the panel (no hidden-files toggle, AF-4).

## Not covered

- Ctrl+Shift+digit tab switching and digit Hot Paths: those bindings use physical
  digit events from the native event loop, which synthetic Slint key events cannot
  produce; tabs and Hot Paths were switched and opened by click instead.
- Middle-click on a tab (the harness clicks with the left button only).
- The real Trash (never used, by rule; only the confirmation and its cancellation).
- Cancelling a running copy from its progress window (the fixture's copies finish too
  fast); Cancel job was exercised in the decision window instead.
- Windows; the scenarios use Unix permissions and symbolic links where noted.
