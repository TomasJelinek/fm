# Compare directories

Ctrl+F10 (or **Compare directories** in the shared toolbar, the menu or a panel's
context menu) compares the two panels' displayed native folders and replaces both
panels' selections with the differences, as Open Salamander's *Compare Directories*
does. The behaviour follows Salamander's implementation (its `CompareDirectories`,
`CompareDirsAux`, `CompareFilesByTime`, `CompareFilesBySize`, the *Compare
Directories* dialog and its configuration defaults), re-implemented from the
description below; no Salamander code is used. Where Midnight Explorer differs, the
table says so and why.

## Salamander semantics and Midnight Explorer

| Topic | Open Salamander | Midnight Explorer |
| --- | --- | --- |
| Start | Both panels must show different paths; otherwise "There is same path in the left and right panel." | Same: the dialog says that both panels show the same folder. Both panels must be native folders (Salamander also refuses content and subfolder comparison outside disks and archives). |
| Selection | All names are unselected first; the differences are then selected in both panels. A cancelled comparison leaves both panels unselected. | The differences replace both selections at once when the comparison ends. A cancelled comparison changes nothing. |
| Which entries | Each panel's listed files and folders (its hidden-file and filter settings apply); `..` is skipped. Inside compared folders everything is listed, hidden files included. | Same. In an inline tree, only the top-level rows are compared and selected; expanded branches are not a second comparison (they show marks, below). |
| Names | Files and folders are matched separately: a file and a folder of the same name are two one-sided entries. Names match without regard to case. | Files and folders are matched separately. **Match exact case** is off by default on Windows (as in Salamander) and on elsewhere, where names differing only in case are different files. Names that fold to the same key on one side (possible only on case-sensitive file systems) are an ambiguous group: all of them are selected. |
| One-sided files | Selected on their side. | Same. |
| Size (default on) | The larger file is selected. | Same. A size that is not known selects both files as not verified (Salamander takes an unknown size as the smaller). |
| Date and time (default on) | The newer file is selected. Times are compared in whole seconds; times at most *N* seconds apart are equal (Configuration › *Compare Directories: ignore time differences up to N seconds*, default on with 2 s, which also covers FAT's two-second times). Times one or two hours apart, give or take *N*, are a daylight saving shift: different, unless *ignore time differences of exactly one or two hours* (default off) is on; a note after the comparison says how many pairs had such a shift. | Same rule. The tolerance (0–86,400 s, default 2) and **Ignore differences of exactly one or two hours** are in the dialog itself; the result line and details give the count of such pairs either way. Times are compared as instants: Salamander converts both sides to local time with the same bias, which cancels out. |
| Attributes (default off) | Both files are selected when read-only, hidden, system, archive, encrypted, temporary, compressed or offline differ. | Same attributes on Windows; elsewhere the permission bits (setuid, setgid and sticky included). Unknown attributes compare equal, as Salamander compares them only where both panels show them. |
| Content (default off) | Different sizes select both files (even without Size). Equal-size files are read byte by byte unless both are selected already; empty files are equal; different bytes select both. A read error asks, then selects both. A first pass totals the bytes so the progress bars are determinate. | Same rule. Reads run on a bounded pool of readers while the walk continues (no second pass). A file that cannot be read, or changed while it was read, selects both files, is counted and listed in the result; no question interrupts the comparison. |
| Folders without subfolders | Not compared, names included, unless *Select subdirectories contained only in one panel* (default off): then one-sided folders are selected. | Same (**Select one-sided folders**). |
| Compare subdirectories (default off) | Same-named folders are compared recursively. A pair differs when the numbers of files or folders differ, any name differs, a file differs by an enabled criterion (size; attributes; with content, size and then bytes; time last), a subfolder's attributes differ (with *Compare attributes of subdirectories*), or a subfolder pair differs. Folders are not compared by time. A differing folder is selected **in both panels**; one-sided folders are selected on their side. Salamander stops reading a pair at its first difference. | Same verdict and the same symmetric selection. Midnight Explorer walks every folder pair to the end so that each subfolder and file can show whether it matches (below); the selection is the same. |
| Attributes of subdirectories | With subdirectories: same-named folders whose attributes differ are selected and not walked. | Same (**Compare folder attributes**). |
| Ignore lists | *Ignore files* and *Ignore directories* masks (default off and empty), applied on both sides, to the panels' entries and inside compared folders; directory masks only where folders are compared. | Same (**Ignore files**, **Ignore folders**), with the application's Masks grammar: `;` separates masks, one `\|` starts excluded masks, case-insensitive. |
| Links | Junctions and folder links are listed as folders and walked (a loop can recurse until paths get too long). | Links and mount points are never followed: a pair of links is matched by name only and counted; a link opposite a file or folder differs. |
| Unreadable folders | An error box per folder (OK continues, Cancel stops). | No question: the folder pair counts as different, the error is counted and up to eight are listed in the result details. An entry inside that one side could not read is not taken as missing there: its counterpart is marked **?** (could not be compared completely), not **+**. |
| Progress | A dialog: "Comparing *left* with *right*" naming the folder pair being compared; with contents, a file bar and a total bar; Cancel. | The Compare directories dialog turns into its progress view: "Comparing *left* with *right*" (paths shortened in the middle, like the path bar) naming the folder pair being listed or the file pair being read, a bar (displayed folder pairs done; bytes read once only contents remain), the counts of folders and files compared and differences found, the file pairs and bytes read; **Cancel** (Escape, Alt+C). No line is added at the bottom of the window. |
| Result | Only when nothing differs: "The files in two directories are identical" (with content) or "appear to be identical"; the daylight saving note. Otherwise the selections are the result. | The dialog closes and one line appears below the panel that was active (as long as both panels show the selections it made): the counts selected here and in the other panel, or identical / appear identical, folders that differ, different contents, one- or two-hour shifts, not readable. Its tooltip has the details. When something could not be read, or a limit was reached, the dialog stays open with the details instead. |
| Inline subfolders (extension) | — | With subfolders, the rows of both panels show a small badge at the end of the name: **=** matches (a folder: everything inside matches), **≠** differs (a folder: something inside differs), **+** exists in this panel only, **?** could not be compared completely. The badge is visible on a collapsed folder; its expanded subfolders and files show theirs too, also when expanded after the comparison. Pointing at a badge (only the badge) shows the shared tooltip with what the comparison found for that row, and the row's and the badge's accessible descriptions say the same: *Larger here: 64,293 vs 63,880 bytes*, *Newer here: 23.11.2026 18:49:05 vs 21.11.2026 18:49:05 (2 d)*, *Daylight saving shift of 1 h (ignored)*, *Attributes differ: here R, there —* (POSIX permissions in octal), *Content differs (first difference at byte 4,096)*, *Differs inside: 3 files and 1 folder differ, 2 only here*, *Folder attributes differ*, *Only in this panel*, *Not verified:* and why (unreadable, changed while read, a limit, metadata unavailable), *Matches: same size and time (within 2 s)* by the compared criteria. The glyph carries the meaning, the colour only emphasises it. The result line has a square × at its right end (*Close comparison result and clear marks*, reachable by Tab): it closes the line and removes every badge from both panels. Escape in a panel without a filename search, Ctrl+Shift+F11 and *Clear comparison marks* in the main menu's Tools do the same. Once a selection change hides the counts, the line becomes *Comparison marks: = matches, ≠ differs, + only here, ? not verified · × clears them* with the same ×, as long as badges are shown. The selection the comparison made stays, as in Salamander. Badges also clear when either panel opens another folder or is re-read, and when a new comparison starts. |

### Dialog

The dialog keeps Salamander's groups: *Compare files with the same name by* (Size,
Date and time with the tolerance field and the daylight saving option, Attributes,
Content), *Subfolders* (Compare subfolders, Compare folder attributes, Select
one-sided folders) and *Names* (Match exact case, Ignore files, Ignore folders, each
with its masks). Each check box has its effect beside it ("The newer file is
selected"); a narrow dialog keeps the effect in the tooltip. The tolerance is a small
number field with its unit. Alt+letter toggles an option and focuses it: Alt+Z Size,
Alt+D Date and time, Alt+T tolerance, Alt+H one- and two-hour differences, Alt+A
Attributes, Alt+C Content, Alt+S Compare subfolders, Alt+O folder attributes, Alt+E
one-sided folders, Alt+M exact case, Alt+F Ignore files, Alt+N Ignore folders, Alt+R
Compare. Enter compares; Escape closes or, while a comparison runs, cancels it (as do
Alt+C and the Cancel button). The choices are kept for the next comparison.

## Threads, memory and cancellation

The UI captures the displayed rows (identities, projection and selection revisions)
and the options, and sends one request. The engine's comparison controller runs one
request at a time on a blocking worker anchored to the engine's service lifetime:

- The top-level rows are matched and judged on the worker (`comparison/compute.rs`;
  the per-pair rules are in `comparison/rules.rs`).
- Same-named folder pairs are walked depth first (`comparison/walk.rs`). Folders are
  listed from their native folder records (`TransferSource::next_listed_child`), so
  a folder of many files is listed without opening each file; without the transfer
  service the search service lists them, opening each child. One folder pair's
  listings are held at a time; the pending stack, the folder-pair nodes and the marks
  have budgets (32, 64 and 64 MiB) and a listing has one (32 MiB); a folder beyond a
  budget or 256 levels deep is not verified and the result says that a limit was
  reached.
- File pairs that must be read go to a bounded queue (64 pairs) read by three
  readers (`comparison/contents.rs`) through the native search service, which pins
  each file without following links; a file must still have the size and time it was
  listed with, and is revalidated after the read. Each reader holds two 128 KiB
  buffers.
- Progress samples (`Event::ComparisonProgress`, at most five per second, dropped
  when the UI queue is full) carry the pair being compared and the counts.

### Why a badge shows what it shows

Each stored mark keeps why (`CompareDetail`, `midnight-core/src/compare_detail.rs`):
flags (time counted, daylight saving shift, POSIX permissions, metadata unavailable,
unreadable, changed while read, a limit, kinds that differ, names that differ only in
case) beside the two marks, and only the numbers that apply, packed into 8-byte words
of one arena: both sizes (2 words), both times in whole seconds (2), both compared
attribute values (1), the first differing byte (1) and, for a folder pair, its
immediate children that did not match (3: differing files and folders, entries only
on either side, not verified). The flags and the arena index fill the marks' padding:
a map entry with its key stays as large as before (40 bytes on Windows, 32 elsewhere),
charged as before at the key's length plus 48 bytes; numbers add 8 bytes each, from the same 64 MiB budget. A file that differs by
size and time costs 32 bytes more, a differing folder 24; matching files inside
compared folders are still not stored. When the budget leaves no room for numbers,
the mark and its flags are still stored. The walk counts a folder pair's children as
it compares them (a counter set per folder-pair node, charged to the node budget: a
node takes 48 bytes instead of 24). The
UI words the detail only for the rows Slint asks for (`presenter/comparison/describe.rs`):
nothing is formatted or kept per row. Time texts are the panels' own localized texts
of both rows when the other panel lists the counterpart and its time is the compared
one; otherwise the line names the difference only ("Newer here by 2 d"). Displayed
pairs whose daylight saving shift was ignored keep it as a detail of their match;
inside folders such matches are not stored, so the budget still grows with folders,
not files. A child the listing could not describe keeps only its name: its folder is
not verified, and both it and its counterpart are marked **?** (*Not verified:
unreadable*), counted once in the folder's *not verified*.
A displayed pair that the metadata already separates and whose contents are then read
(a newer file with Compare contents) keeps **≠** and its reason when the bytes match,
as it stays selected; in 0.5.1 and earlier its badge said **=** (inside folders such a pair was
already **≠**).

Cancel (the button, Escape, Alt+C, a newer request, a panel change or shutdown)
cancels the request's token: listing stops at its next entry, each read at its next
chunk, the readers end and the worker returns; a cancelled request publishes nothing.
Every request the UI still waits for is answered, with a result or the reason there
is none.

### The 0.5.1 fix of the stuck comparison

Before 0.5.1 a request whose panel was re-read, navigated or changed while it ran
was dropped by the controller without an answer (`comparison.rs`, the publish step
and `invalidate`). The UI kept waiting for it: the bottom line kept showing
"Comparing subfolders: … entries in … folders…", automatic re-reads stayed blocked,
and F2, F7, Delete, Copy and Move were refused for as long as the application ran.
A long subfolder comparison of an active folder (a working copy whose files change
while it is compared) hit this nearly every time. The walk itself also opened every
listed file and folder twice (search-service listing), and judged a folder that
differed only on its newer side, so a copy with newer times was selected on one side
only. The controller now answers every awaited request
(`deep_tests::a_panel_change_during_a_comparison_is_answered_not_dropped`, red
before); listing uses folder records; and folder verdicts follow Salamander.

Selections are published only if both panels still show the captured rows; panel
paths never authorize native access. While a comparison with contents or subfolders
runs, F2 Rename, F7, Shift+F4, Delete, Copy/Move, Attributes, Paste and Show hidden
files are refused with a notice that names Cancel; automatic re-reads wait for it.

## Verification

Engine (`crates/midnight-engine/src/comparison/`): `rules_tests.rs` (size, time
tolerance, whole seconds, daylight saving shifts, attributes, when contents are read,
ignore masks, case folding), `deep_tests.rs` (an in-memory search service:
subfolders and their symmetric selection, the newer copy of the report, marks at
every depth, files and folders of one name, ignore masks inside folders, case
folding, daylight saving inside folders, links, unreadable entries, contents read in
parallel, a file written while read, limits, the same folder in both panels, the
controller's progress, a panel change answered, cancellation within 100 ms),
`tests.rs` and `tree_tests.rs` (displayed rows, trees, the controller under output
backpressure), and `native_bench_tests.rs`, a timing run explicitly over a native
tree of 5,000 folders and 40,000 files per side. Presenter:
`crates/midnight-ui/src/presenter/comparison_tests.rs` (the dialog's progress and
cancellation, the result line below the active panel, marks),
`comparison_badge_tests.rs` (the badge's words for every verdict, Clear comparison
keeping the selection, the line that names the marks once the counts no longer
apply) and `crates/midnight-ui/src/compare_badges_tests.rs` (the window: the badge's
tooltip and accessible description, only over the badge; closing the result line by
its square, centred, right-aligned ×, Escape after a filename search, Ctrl+Shift+F11 and the menu; layout at fonts
8, 12 and 32, Desktop and Touch). Engine details: `rules_tests.rs`
(`a_verdict_records_what_differs_with_both_sides_values`) and `detail_tests.rs`;
the packing and its memory: `crates/midnight-core/src/compare_tests.rs`. End to end: the
`tools_compare_dirs` acceptance scenario ([ACCEPTANCE.md](ACCEPTANCE.md)).
