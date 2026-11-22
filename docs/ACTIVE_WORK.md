# Active work and requirements evidence — 21 November 2026

<a id="active-work-and-requirements-evidence--13-november-2026"></a>
<a id="active-work-and-requirements-evidence--15-november-2026"></a>
<a id="active-work-and-requirements-evidence--16-november-2026"></a>
<a id="active-work-and-requirements-evidence--17-november-2026"></a>
<a id="active-work-and-requirements-evidence--18-november-2026"></a>
<a id="active-work-and-requirements-evidence--19-november-2026"></a>
<a id="active-work-and-requirements-evidence--20-november-2026"></a>

This is the current implementation/acceptance ledger alongside
[Feature status](FEATURE_STATUS.md) and the preserved
[requirement IDs](IMPLEMENTATION_BACKLOG.md). Historical runs are evidence only
for their named source/environment; they do not turn open acceptance into missing
code or a green subset into complete qualification.

<a id="current-checkpoint"></a>

## Current checkpoint

**ACTIVE — explicitly resumed by the user on 15 November 2026:** implement all
designed app features, test each deeply and polish all UI. Copy/Move convergence
comes first; role work still requires scoped acceptance and exclusive ownership.

### Handover implementation — 21 November 2026

The user's 20 November handover listed undone work (half-done items, product work
not started, acceptance, documentation, earlier follow-ups), a viewer TODO and open
bugs. This checkpoint integrates what was finished on 20 and 21 November. Windows
work was built and tested on Windows; Linux code was built and tested in WSL
(Ubuntu 24.04, ext4). Each commit message lists its exact tests; a commit's tests ran
on exactly that change (an isolated checkout of the parent plus the change) unless it
says otherwise.

The user's requests of 20 November:

- Checksums: Ctrl+F8 opens Calculate with a target and SHA-256 only (`4004145f`), the
  result columns resize by their header separators (`b70f551a`), every file is listed
  before any is read and each result shows as it finishes (`21fc78fa`), Copy writes the
  saved checksum file's format (`57680e2d`), a selected folder contributes its own
  files without Include subfolders (`2340d0f6`, `4328654b`), the hotkey sweep requires
  the ready Calculate page (`77e5a1a7`), and the second page is named Verify
  (`dea6681d`). These came from a feature session's branch.
- The About box sky runs at a sixth of its speed at the same frame rate (`c5c95186`).
- Options: OK and Cancel stay on one row while they fit beside the footer actions, and
  the list of settings kept after an import shows whole at the end of the General page
  (`89c2f693`); a settings file exported on the other operating system imports with
  that system's paths kept as inactive data (`494d9a67`).

Handover items:

- The documentation owner list (`aaa6e579`).
- Windows Delete asks about each entry it cannot remove and handles Read-only on
  filesystems without POSIX delete; Copy pauses on one unreadable child instead of the
  whole folder (`9a41840c`).
- A notice's Copy details include its reason (O-5), the fed-list count reads
  correctly (J-1), select masks are checked by the backend before they are applied or
  saved, keypad * in fed lists, and Resolve for waiting Delete questions on Jobs rows
  (`fdc81466`).
- Viewers: in short windows Find takes the typography row's place, CSV files may use
  semicolons, and the reported selected-text contrast was refuted (312 checks with the
  real renderer; the readings were antialiased edges) (`a56d0a4b`).
- UI polish gate, batch 1: the main window (`ca79fc6d`) and the dialogs (`985c8ec0`).
- A waiting Copy holds no handle on what it will read, which fixes the intermittent
  Windows sharing violation in
  `reading_the_busy_copys_output_waits_for_it_with_its_reason` (`accfcc53`).
- Archive limits: file lists hold any number of paths within their 4 MiB, only native
  roots of one Copy or drop are bounded (4,096; a Move takes at most 64), inline-tree
  captures take every
  row their projection selects, and an archive member claims its spool space before
  it is staged (`e835f4ec`).
- Performance (`b3974c06`): archive panel rows 649 → 639 bytes per member, Linux
  collation keys about a third of their former size, a native Move on Linux syncs two
  folders instead of four, and ancestry walks take one `statx` per level and run once
  per folder in a refresh batch
  ([transfer and listing performance](verification/2026-11-21-transfer-and-listing-performance.md)).
  Copy's per-file folder sync is designed only (PERF-05).
- The command icon publication and the worker store release their file locks
  explicitly (`32068fda`): a child process starting on another thread shares the
  open lock file until it executes, so closing it alone left the lock held. This was
  the rare `icon_snapshot` test failure seen in WSL.
- The spool's unused flush hook and its test are removed (`e6396e83`).
- A waiting Copy opens each item before it creates the item's stage or folder in the
  destination (`c7185efe`), so a file another program holds without sharing leaves
  nothing there while its Retry question waits. CI run 44853258726 found this in
  `real_sharing_violation_retry_succeeds_only_after_exclusive_handle_is_released`:
  `accfcc53` had moved the open to the first read, after the stage existed. Its
  isolated check had run the engine library only; checks of engine changes now also
  run its integration tests.
- UI polish gate, batch 2 (`93e7f43a`): Find's column headings, missing tooltips, the
  command line's folder, Enter with an empty name, the remaining key routers, themed
  selection in Make File List and Compare directories, Options at 320×200, job
  windows at their minimum size, Enter in Compare directories and Checksums, and
  focus in Checksums, dialog tabs and Jobs; see the
  [UI polish record](developer/acceptance/ui-polish.md).
- A Linux size-warnings test compares the report file's length too, so a file that
  reuses its descriptor and inode after it is closed no longer looks like it
  (`d7d65be9`).
- Copy that follows links follows one into another filesystem, local, network or
  FUSE, but never into the kernel's interface filesystems such as /proc or /sys
  (`3424b0c1`); Windows already followed a junction into another local volume.
- The path bar of a Network location shows its ancestor chain, one part per
  location that browses it (`196a1a61`).
- Move empties owned write-protected folders of the moved tree (Delete's rule), a
  Move child that cannot be prepared is skipped alone, and an uncertain commit or
  cleanup whose data stays in place asks Skip or Cancel instead of ending the job
  (`80e8f98a`).
- The acceptance suite runs on Windows (`scripts\acceptance.ps1`; 91 of 107 scenarios
  pass, see the [Windows results](developer/acceptance/windows.md)), and CI lints the
  suite on both platforms (`4d788480`).
- Viewers: the Rust source travel lost at the raster frontier is reproduced and
  fixed, refills of line-budget pages read only the rows past the shown page, the
  HTML viewer on Windows scrolls the host's distance per wheel notch, and the zoom
  buttons have tooltips (`6cd20942`).
- UI polish gate, batch 3 (`5fc9d45e`): a running job's footer at 320×200 shows
  glyph-only actions with tooltips, the Checksums Verify path is a themed field,
  captions of the keyboard, Custom Commands, thumbnail and archive options wrap
  instead of being cut, wrapping check boxes and colour swatches show hover, the
  Network path bar's omitted parts have a tooltip, and the Windows gate's WIN-2 (the
  command line's folder), WIN-3 (the path bar's fit), WIN-7 (Find's compact status)
  and the minimum-size finding U-D08 (About, Hot Paths, a notice's Details) are fixed;
  Jobs rows and transfer details count one in the singular. On Linux one caption was
  still cut at 320×200 with 22–24 px fonts (Options › Archives' filter placeholder);
  it is shorter since `32e79cf9`.
- Windows Move durability (WIN-6): a same-volume native Move flushes the folders it
  changed (`2597a6d8`); a cross-volume Move and a merge confirm each removal by delete
  disposition once the job has closed its handles on the entry (looking the name up
  through the held folder, then flushing that folder), and a Move saves its renames
  together, each folder once per round (`8db2091a`). Such Moves end Completed; a
  removal Windows has not completed, or a folder that cannot be flushed, still warns.
- A paused transfer's current-file line stays in view under Details (WIN-1,
  `cc8a952e`).
- Final review of the handover's commits (four code reviewers and a documentation
  reviewer, read-only): no critical or high findings. The medium and low findings are
  fixed: semicolon CSV read by consistency, HTML wheel steps at any wheel setting,
  one CI Clippy step, the acceptance script's environment and the harness's
  junctions, stand-ins and settings guard (`e3bb01e2`); checksums (a mount point left
  out without Include subfolders, Enter after Verify, Copy's reasons, folders named
  like checksum files, singular counts), terminal commands of the other system and
  their key bindings on import, faster hover and checksum tests (`153bd96f`); spool
  claims counted per volume and the spool root's choice, Linux Move and Delete mode
  restores and set-group-ID, a moved folder checked once, a root notice that held the
  disk slot, xenfs, functionfs and gadgetfs, Windows Delete's Read-only restore
  (`53def575`, `d6b7750a`); inline-tree Copy roots opened before staging, planned
  folders rechecked by identity, Skip keeping only folders with leftovers
  (`dbb1f07c`); the checksum listing pass, which opened every file a second time,
  now describes files from their folder's listing and leaves out subfolders unopened
  (20,000 files listed in 0.14–0.19 s instead of 2.4–3.1 s, `7ff14608`); the
  documents (`42c09152`), and no machine, account, branch or checkout path in
  tracked files (`df479453`, `5984bf66`, `08914523`).
- Viewers: very long rows keep their whole scroll travel (`d0733dc7`): a page holds
  at most what one layout places correctly, and pages of rows too long for the window
  keep the retained camera. The renderer then cut every layout longer than 64 KiB into
  shaping runs, because parley keeps a cluster's offset in its run in 16 bits, without
  changing a glyph, caret or pixel; long-row pages hold 120 KiB since, and 2400-byte
  rows fill the window at rest (`ffa6f3fe`).
- WIN-8: a Windows destination's ancestry is checked once per refresh batch, 121 → 44
  ancestor opens per copied file (`cd2f821a`; see the open item below).
- Archives: a container whose change time alone moved (Defender for Endpoint's DLP
  tagged one with an alternate data stream during the WIN-8 measurement and stopped a
  copy-out at member 315) is read on, as Copy and Move sources and viewers are
  (VIEW-05), and a lasting change fails without Retry (`1f8dcac5`); requests to an
  archive whose worker has stopped offer no Retry either (`26117b66`); a Retry that
  opens the archive again is ARCH-08.
- A settings test round-tripped a PowerShell terminal command, which Linux now keeps
  inactive on import; it uses the system's own shell (`a045453e`, CI run 44889924835).
- Test speed pass (`ddc95448`,
  [record](verification/2026-11-21-test-speed-pass.md)): every test timed again on
  Windows (7,973 tests; 254 over 0.5 s, listed); the slowest new ones split per view,
  laid out instead of rendered, or given a smaller busy fixture, each still failing
  with its fix undone; dead test code removed. All 67 test executables 84.9 s →
  75.1 s; tests that wait on real timing or real fixture sizes are listed for their
  owners.

CI runs 44828132118 (`3987d3c8`), 44837536773 (`c5c95186`), 44845219935
(`9a41840c`) and 44851212423 (`985c8ec0`) passed every job on Windows and Linux. Run
44841450903 (`aaa6e579`) failed only in the Windows engine test that `accfcc53` fixes.
Run 44853258726 (`accfcc53`) failed only in the Windows integration test that
`c7185efe` fixes, and run 44861514459 (`c7185efe`, which contains `e835f4ec` to
`c7185efe`) passed every job on Windows and Linux, as did runs 44865587302
(`3d42cb1f`), 44869629215 (`196a1a61`) and 44871872999 (`80e8f98a`). Run
44872751062 (`4d788480`) failed only in Linux's acceptance Clippy step (two
Windows-only fixture helpers unused on Unix; `f499e364`). Runs 44877701093
(`5984bf66`) and 44881878957 (`08914523`) failed only in Linux's
`options_caption_fit` (the archive filter placeholder; `32e79cf9`), and run
44884116446 (`32e79cf9`) passed every job on Windows and Linux. Runs 44889924835
(`d0733dc7`) and 44893276832 (`ffa6f3fe`) failed only in Linux's test step, in the
settings round-trip of a PowerShell command (`a045453e`), and run 44895582122
(`cd2f821a`, which contains every commit from `8db2091a` to `cd2f821a`) passed every
job on Windows and Linux. Runs of commits that a newer push superseded were
cancelled.

On 21 November drive C: filled up: the WSL disk image (73 GB), which the agents'
Linux build clones had grown, lives there, and WSL failed with I/O errors. The
finished agents' clones were deleted inside WSL (71 to 55 GB used), but the image
does not shrink by itself; moving it to D: (`wsl --manage Ubuntu-24.04 --move <a
folder on D:>`) or compacting it needs the user. Until then no Linux check runs in
WSL: Linux code is type-checked and linted on Windows (Clippy for
`x86_64-unknown-linux-gnu`, whose standard library the project toolchain now has;
crates that need a Linux pkg-config sysroot, such as the UI's fontconfig, cannot be
checked this way), and its tests from `80e8f98a` on run in CI only (the acceptance
suite, which CI only lints, and the HTML viewer plugin, which CI does not build, not
at all on Linux). The Windows acceptance suite cannot run either while C: is full:
its private installation lives under the user profile, because the plugin host
loads plugins only from a folder no other account can change.

Still open from the handover:

- PERF-05, batching Copy's per-file folder sync, is designed but needs the
  Architect's approval of a core transfer API change.
- Copying more than 4,096 selected native items at once, by opening each root just
  before it is copied so that open handles follow folder depth instead of selection
  size, is planned ([current limits](developer/TRANSFERS.md#current-limits-and-next-stage))
  and needs an Architect decision.
- A prepared Move item still takes its guard at preparation, so two Moves that meet,
  or a granted job renaming an item a waiting Move holds, can fail instead of waiting.
- The UI polish gate and the acceptance suites have not been run again since these
  changes (Windows: C: is full; Linux: WSL needs C:): WIN-1, WIN-2, WIN-3, WIN-6,
  WIN-7 and WIN-9 are fixed in source and not re-run, and the acceptance scenarios
  changed for all platforms need a Linux run
  ([Linux reruns needed](developer/acceptance/windows.md#linux-reruns-needed)).
- WIN-8, copying 100,001 archive members out: the Windows namespace pins' ancestry
  checks are batched (121 → 44 ancestor opens per file; release 97–132 → 133–183
  files/s, median 931 → 562 s, on a machine shared with builds), so a release build
  meets the scenario's 600 s on a moderately loaded machine but not under full load.
  Further steps, each an Architect decision: one namespace change per file instead of
  two (a job-private staging folder), a cheaper proof of a held ancestry, fewer archive
  source validations per member and fewer blocking-task hops per file
  ([Windows acceptance record](developer/acceptance/windows.md)).
- Viewers: unwrapped rows longer than about 2,800 bytes still leave blank space below
  a page's rows at rest in a window of 44 rows (a page holds 120 KiB and at most 16
  plugin reads; 8000-byte rows fill 15 rows), and larger plugin pages need a wire API
  change (an Architect decision).

### Integration follow-up — 20 November 2026

The integration batch below (`d9cafd45`) and the text page join (`edba714a`)
entered `main` without a full test run. Their CI runs failed in tests that the
batch added or whose behavior it changed, and each failed step hid the steps after
it. The
commits below fix those failures step by step and merge Export/Import
configuration. Unless an item says otherwise, a fix changes only tests.

- Linux elevation (`bb40612d`): the sandbox profile test expected every
  group-writable folder to be refused, but the installer and the application accept
  one whose group is its owner's private group (umask 002), as documented. 18 sandbox
  profile tests pass in a WSL clone whose user has a private group.
- Windows time precision (`806e6b55`): a localization test read back 789 ns, which
  Windows SystemTime (100 ns ticks) cannot hold; it now uses 789,100 ns.
- Copy and Move, a product fix (`c47132a2`): the batch's preparation Skip also
  applied to a job requiring the exact modified time, so Skip copied the file its
  preflight had refused. Such a job now offers Retry and Cancel only.
- Windows Delete, a product fix (`b2bbb329`): a replaced Delete target was reported
  as NotFound, which the batch's Delete decisions retry; it is now final
  (InvalidInput, as on Linux). The operation tests' waits end after 60 s and name
  any unanswered decision instead of hanging.
- Windows engine tests (`3346d2a7`) and Windows UI tests (`4bc4f671`, `23f92546`,
  `b7a99059`, `aea14ce0`) follow the batch's Skip, link and Delete retry behavior,
  and check text layout by the dialogs' and panels' own rules instead of Linux font
  metrics. Windows engine library tests: 1,809 passed.
- CI-only tests (`f26f7c72`): `Spool::finish` no longer flushes, and a Source
  position that the loaded window cannot show settles at its nearest edge; the
  Windows spool test and the installed Source position test still expected the old
  behavior.
- CI's archive worker step (`e862517b`) skips
  `real_sandboxed_worker_lists_archives_of_millions_of_members`, a manual
  measurement that the batch added and that needs generated archives
  ([installed archive worker qualification](developer/TEST_EXECUTION.md#installed-archive-worker-qualification)).
- Export/Import configuration (`9e4e2040`, from a feature session's branch), with
  the Options footer fix `440f9537`: the new
  footer buttons pushed OK under Cancel at 320×200 with a 32 px font, so a click on
  OK hit Cancel and Escape reached no handler. Where the footer has no room, the
  buttons now show icons only or move to the end of the General page
  ([user guide](SETTINGS_FILES.md)).

Before the pushes, CI's Windows test step (all ten workspace gates, run locally
without stopping at a failure) passed at `9e4e2040` except the two footer tests that
`440f9537` fixes. At `440f9537`, midnight-ui's integration and library tests passed
on Windows (902 and 1,415, Clippy and rustfmt clean) and on Linux in WSL (897 and
1,418).

CI run 44795512430 at `806e6b55`, which predates the 60 s operation test waits, hung
in a Windows engine test until it was cancelled. Run 44804002194 at `aea14ce0`
failed in the two CI-only tests that `f26f7c72` fixes. Run 44814555541 at
`f26f7c72` passed every step on Windows and Linux before the archive worker step
and failed there only in the manual test that `e862517b` skips. CI run 44819849685
at `632c12ab`, which contains every item above, passed all 26 jobs on Windows and
Linux, with every check step of each platform run. Its check jobs passed 7,743 tests
on Windows and 7,799 on Linux and failed none; among them are the non-ignored
workspace tests that the batch's table names without a recorded result. The
acceptance scenarios it names (`viewers_archives`, `tools_compare_dirs`) need the
opt-in `acceptance` feature and are not part of CI.

The batch's nested-archive worker test
(`real_workers_open_archives_inside_archives_and_give_every_level_up`) ran in no CI
step, although CI has the archive worker it needs. CI's check job and both
validation scripts now run it in a step of its own (`test:nested_worker`). Against
archive workers built from the current tree it passed 3 of 3 runs on Windows and on
Linux (WSL); against a worker built on 19 November, before the plugin's "This file is
not a valid ZIP archive." message, it failed.

Not established:

- the Linux settings-file picker filter (`*.json`), which is compiled but exercised
  by no test.

### Integration batch — 20 November 2026

This batch enters `main` as one commit. The table names the source of each change
and the dated record that holds its test results; where no record lists results, the
row says so. Those notes predate CI run 44819849685, which passed the non-ignored
workspace tests they name on Windows and Linux
([Integration follow-up](#integration-follow-up--20-november-2026)). The opt-in acceptance suite (`./build.sh --acceptance`,
[ACCEPTANCE.md](developer/ACCEPTANCE.md)) found most of these gaps. Its area runs
are recorded for [files](verification/2026-11-20-acceptance-acc-files.md),
[tools](verification/2026-11-20-acceptance-acc-tools.md),
[viewers](verification/2026-11-20-acceptance-acc-viewers.md) and
[Options](verification/2026-11-20-acceptance-acc-options.md), each made before the
fixes of its findings. Other findings of these runs were fixed in source too (for
example AF-1, AF-7, ACC-TOOLS-03 and ACC-TOOLS-04). A later whole-suite run by the
coordinator (host desktop session, on the integration tree with the viewer speed
work up to its spool-staging fix) recorded 50 passed, 3 warned and 0 failed of the
53 scenarios ([viewer speed record](verification/2026-11-20-viewer-speed.md#html-file-that-did-not-open));
which scenarios warned is not recorded, and it is the only recorded run after those
fixes. No full test suite, Windows build or CI run of the integrated tree is
recorded here; the fixes it needed and the CI runs after it are in
[Integration follow-up](#integration-follow-up--20-november-2026).

| Work | Implemented in source | Evidence, and what is not established |
| --- | --- | --- |
| Links in Copy and Move (COPY-03, MOVE-01) | On Linux, Copy with **Follow links** cleared and every Move transfer a symbolic link as a link with its exact target text: staged, read back and published without replacing an existing name. With the option checked (the default) Copy follows a link where it can and records a note where it cannot ([links](developer/TRANSFERS.md#links-in-copy-and-move)). Copy and cross-volume Move also give new files and folders the source's permission bits masked by the umask. Windows keeps its directory-link options. | Findings AF-2 and AF-3 of the files run, before the change. No dated record lists the results of the link and permission tests. Windows link preservation remains open. |
| Properties on Linux | Alt+Enter shows a read-only summary: type, size, owner and group, permissions, and created, modified and accessed times. A single folder's files, folders and size are then counted on a backend worker, without following links or entering other filesystems (`desktop_properties.rs`). Windows opens its own dialog. | Finding AF-8. No dated record lists the results of `desktop_properties_tests.rs`. |
| Show hidden files; selection by mask | Ctrl+H, F10 → Panels and navigation, each panel's View menu and Options → General (settings schema 35; hidden entries are shown by default). Select and Deselect by mask on Ctrl+= and Ctrl+−, for keyboards without a numeric keypad, and on keypad + and −; keypad * inverts ([keys](KEYBOARD_SHORTCUTS.md#hidden-files-and-selection-by-mask)). | [Record](verification/2026-11-20-hidden-files-and-mask-selection.md): the new and neighbouring targeted tests and five smoke-scale scenarios passed. That record predates the Ctrl+= and Ctrl+− keys; no record lists the results of their tests (`panel_choice_shortcuts.rs`). Not verified: the Windows hidden-attribute policy (not compiled), real keypad events and the full suites. |
| Viewers | Source highlighting for CSS, Makefiles and CSV/TSV with one colour per column (25 lexical profiles; `Makefile` and `GNUmakefile` match by whole name). First-page PDF thumbnails drawn with Hayro 0.9.0 inside the image thumbnailer. A refused image or PDF names the decoder's reason through image-viewer API 0.3.0 (`FailureReason`) and SDK 0.4.0, which the image (0.3.0) and PDF (0.2.0) viewers pin. | [Record](verification/2026-11-20-viewer-feature-gaps.md): the plugins' CI scripts, host and native tests, and two smoke runs of `viewers_files` and `viewers_thumbnails` (the HTML pages were not run in the build image). Not verified: Windows, the release profile, full-scale acceptance and PDF thumbnail time on large documents. |
| File comparison (VIEW-01) | Text up to 32 MiB, 1,000,000 lines and 16 KiB per line. Binary files, larger text and other pairs are compared as bytes in a side-by-side hex view with a summary and difference navigation; the Hex button shows a text pair's bytes ([byte comparison](developer/FILE_COMPARISON.md#byte-comparison-hex-view)). | The alignment measurements of 20 November are in [FILE_COMPARISON.md](developer/FILE_COMPARISON.md#large-text-comparison). No dated record lists the results of the byte comparison tests. |
| Archives inside archives (ARCH-02) | Enter on an archive member opens it in the same panel, up to 4 archive levels and 3 nested mounts. The member is staged and verified through its enclosing session and read by a restricted worker of its own; every read revalidates the enclosing chain. Session-only: tabs and Hot Paths keep the outermost archive's folder ([details](ARCHIVE_FORMATS.md#archives-inside-archives)). | `viewers_archives` covers a ZIP inside a ZIP; no dated record lists its result after the change or the results of the host and engine tests. Inline branches do not expand archives, and Find does not open archives inside archives. |
| Archive staging in memory (ARCH-01) | A staged stream or member of at most 16 MiB stays in host memory while a 128 MiB budget shared by every archive mount has room; larger content goes to a spool file, whose reads check a CRC-32 per 64 KiB block instead of flushing it ([filesystem API](developer/FILESYSTEM_API.md)). This implements the design's RAM budget for small content. | [Viewer speed record](verification/2026-11-20-viewer-speed.md#archive-browsing-targz): staging tests and mount timings beside a disk writer on one Linux machine. Peak memory across both workers is not measured. |
| Find | Content regular expressions matched per line, Search inside archives through the installed archive plugins, and Feed to panel ([Find](developer/FIND.md)). | Archive search is verified only with a fake provider session (`crates/midnight-engine/tests/find_archives.rs`); the acceptance harness has no archive plugin. No dated record lists the results of these tests. |
| Compare directories | Compare contents (byte by byte after the size and time pre-check) and Include subfolders ([Compare directories](developer/COMPARE_DIRECTORIES.md)). | Engine (`comparison/deep_tests.rs`) and presenter tests and `tools_compare_dirs`; no dated record lists their results. |
| Panel fixes | Ctrl+F2's Now fills a row with the current local date, time and offset. On Linux all folder watches share one inotify instance; a folder whose registration fails is checked about every second while the registration is retried, and only a minute of failures shows the status line. A Rename, New file or New folder holds its panel's input while the job runs and for up to 3 s more until the change is listed (`Renaming “a” to “b”…` after half a second; Escape releases it). Busy folders refresh in place. | [Busy folder record](verification/2026-11-19-busy-folder-refresh.md): targeted Linux suites for the watch and hold follow-ups; Windows was not run. The Now fix has a test (`metadata_times::now_shows_the_current_local_time_in_its_row_and_apply_sets_that_moment`) whose result no record lists. |
| Viewer speed and fixes | Faster opening: video support, the font database and the font catalog are prepared once at startup, Linux shares one wgpu device, image and PDF decoding starts before the window acknowledges its canvas, and the plugins' `dev` profiles are optimized. Local HTML pages open again (V1). The Markdown viewer scrolls past tables taller than a page, and a scrollbar position that no window can show settles at the nearer edge. `scripts/install-html-viewer-apparmor.sh` and an in-app message handle Ubuntu's restriction of unprivileged user namespaces ([system requirements](SYSTEM_REQUIREMENTS.md)). | [Viewer speed record](verification/2026-11-20-viewer-speed.md): measurements on one Linux desktop and targeted tests. HTML scrolling was measured (20–35 ms from Page Down to the first changed frame, software renderer) and not changed. The script's root install and uninstall steps were not run. |
| Kinetic scrolling | The momentum decay time constant rose from 0.35 to 0.8 s, so panels and viewers glide more than twice as far and for about four seconds, and frames slower than 16 ms no longer slow the glide down ([kinetic scrolling](developer/SCROLLING.md#touchpad-momentum-kinetic-scrolling)). | No record lists test results for the change; the feel on a physical touchpad is not verified. |
| UI review and Options findings | Function-key captions, breadcrumbs, the Find footer, the function-key layer, right-to-left names, one glyph style and sharper tooltips; Options findings OPT-ESC, OPT-HOTPATH, OPT-CMDERR, OPT-CMPERR, OPT-TIP and OPT-F9. | [UI review record](verification/2026-11-20-ui-review-pass.md): `ui_review --all-themes` went from 21 flags to 0, and the listed suites passed. The Windows launcher change was compiled only as an extract. |

### Integration — 20 November 2026

`main` integrates the commits of 19 and 20 November and implements the eight
decisions of 20 November. Each change below was reviewed against its source and
verified on Windows as stated. CI run 44708841616 at `3fcfb3d3`, which contains the
first two items, passed every job on Windows and Linux. CI run 44715333960 at
`1ee00535` (the morning's four decisions) passed 25 of 26 jobs, the Linux checks and
both release builds included; the Windows check failed only in
`icons::tests::unassociated_document_art_is_identified_for_semantic_fallbacks`,
which ran its native stock-icon lookup outside the guard that serializes the
module's native icon tests; `064df204` adds the guard. CI run 44746089017 at
`67298a96` (VIEW-05) and run 44753500560 at `1a33b1fd`, which contains every item
below, passed every job on Windows and Linux.

- CI follow-ups: rustfmt on the panel selection bridge (`2e562958`) and a named
  closure type for Clippy in an operation test (`c2ea6035`). CI run 44706184619 at
  `c2ea6035` then failed only in
  `operations_native_ui::real_f2_rename_shows_the_new_name_and_holds_panel_input_until_then`
  (cursor 19, expected 20): Windows collation sorts "část 文档 🦀.txt" with "c", so the
  renamed file is the fixture's last row and Down cannot move. The test checks with
  Up instead (`56372c05`); all 9 native operation UI tests pass on Windows.
- Docs: 508 places in 35 documents ran a word into the number after it ("at most100
  distinct strings"); `9759d391` splits them (whitespace only, `git diff -w` is
  empty).
- COMPARE-02, decision "re-check content" (`0f8d68f8`): a text comparison source
  whose revision moved only in its change time is adopted after a complete fresh
  read reproduces the retained text byte for byte; whole-input verification restarts
  on the new pair. Images and every other change still report "Compared file
  changed". The three comparator UI tests that timed out after 30 s each that
  morning passed 17 of 17 three times in about 2 s.
- Copy timing, decision "accept unchanged selection" (`35b5bb26`): Copy accepts any
  frame painted, or retained surface published, since the selection last changed, so
  a Ctrl+C between a find and its position refinement copies the match; a frame from
  before the change still copies nothing. The engine tests' `copy_from` retry helper
  is gone. `c3de5577` also lets Copy in the non-retained fallback mode (very long
  lines) copy the page selection after a find; before, it copied nothing.
- BUILD-02, decision "unify now" (`2d7daf46`): one link rule for package inputs
  (links above the caller-named root accepted, links and `..` below it refused) and
  outputs (below the checkout's own `_build`, no link from `_build` down). BUILD-03
  lists the scripts that still apply the old rule.
- TEST-04, decision "bounded retry in tests" (`1ee00535`): the two platform tests
  rerun on a brand-new fixture and the shortcut test retries its own operations and
  the installations expected to succeed, for about 2 s on errors 5, 32 and 33.
  `19d9e3f8` fixes the shortcut installer's cleanup: a staged shortcut that could
  not be removed left the Desktop and installation pins open and replaced the
  original error.
- VIEW-05, decision "accept ChangeTime-only" (`67298a96`): endpoint protection moves
  a fresh file's ChangeTime 1–2 s after it is written, so viewers showed "Content
  read failed" and Copy/Move "consented destination changed". Viewer reads and
  Copy/Move checks now accept a revision whose change time alone moved (identity,
  length and write time are still compared) through explicit policies whose default
  stays exact; the comparator, Find, checksums, user commands, Rename, archives and
  thumbnails stay exact, as VIEW-05 lists. The real-plugins viewer test passed 6 of
  6 with its fixture's ChangeTime moved four times per run and failed again with
  "Content read failed" when the viewer capture was set back to exact.
- Build scripts, decisions "Yes, now" for BUILD-03 and "Retry for ~2 s" (`d0207d3e`,
  `1a33b1fd`): the BUILD-02 link rule covers the remaining build scripts and test
  runners (BUILD-04 lists the image decoder's native helpers, which still apply the
  old rule), and on Windows the renames of a script's own fresh output wait out
  errors 5, 32, 33 and File.Replace's 1175 for about 2 s. `c8e5f19d` verifies them
  with short budgets: the new hold tests take about 19 s instead of 48 s.
- BUILD-04, the user's "fix BUILD-04 too" (`497a69a3`): the image decoder's native
  helpers and the plugins' Linux build and CI scripts follow the same link rule, so
  image, PDF and thumbnail builds work from linked, junctioned or hidden checkouts
  while links from `_build` down stay refused; the native caches stay link-free and
  the dav1d archive is extracted from the bytes that were verified; fresh native
  downloads and stages wait out Windows holds for about 2 s. A real offline native
  build through a junctioned checkout (Windows) and the Linux image `build.sh` path
  through a symlinked parent folder succeeded. BUILD-05 lists the scripts still
  outside the rule.
- Docs, decision "clean up both" (`4f62d8f2`, `aee6d5a4`): FEATURE_STATUS keeps each
  of seven repeated evidence paragraphs once, with links where they repeated, and
  both documents' run-together words and numbers are split (whitespace only).
- CI flakes in untouched tests: an icon test now takes the module's native-icon
  guard (`064df204`); a thumbnail test accepts either drain order once cancelled
  reads free their slots (`a51de7b3`); the web viewer watchdog test allows one wake
  per busy limit a stalled write burst spans (`e9c7809f`).
- Test speed (Windows, per-test timing of every workspace test executable): 238.8 s
  of executable wall time for 7,136 timed tests, against 238.5 s on 18 November with
  1,034 fewer tests and 295.4 s that morning. The comparator UI target fell from
  51.5 s with three 30 s timeouts to 24.0 s with none. The slowest test is the 5.7 s
  independent SHA-256 corpus check; the 5.0 s busy-folder test watches a changing
  folder for 3 s of real time against the production one-second watch cooldown.

Not established:

- Live endpoint-protection activity during the verification runs: no hold was seen
  after 10:04, and a 14:08 probe saw no ChangeTime move on fresh files (at about
  10:50 the ChangeTime of fresh 22.5 MB files had moved 1–2 s after writing), so
  COMPARE-02, TEST-04, VIEW-05 and the build-script retries are verified with
  injected revisions, injected ChangeTime moves and scripted holds.
- A full Rust plugin build and an application build from a linked checkout (the
  native image decoder build was run through a junction and a linked parent folder;
  BUILD-05 lists the scripts still outside the rule), and the manual native CEF
  Office/HTML and qualified Source acceptance tests.

### Post-merge validation integrated — 19 November 2026

`codex/post-merge-validation-20261118` (`4d451bae`) was fast-forwarded into `main`.
CI then needed these follow-ups; CI run 44611283273 at `3aab132f` passed every job
on Windows and Linux, the first fully green run of `main` since 17 November. This
establishes the CI item left open by the integration validation checkpoint below.

- Build and lint: rustfmt on one file; the comparator's unit tests compile again
  (the branch's merge of `origin/main` declared `extern crate self as
  midnight_compare_ui` twice); two Clippy lints in `midnight-platform-windows`; the
  archive reader test again seeks with a zero relative offset, which its `#[expect]`
  and purpose require.
- Stale tests: the FFmpeg pin test counts swresample; seven engine and UI tests
  follow the contracts of `72d621f4` (a late cancellation after the last committed
  root completes the job, discovery runs on a pause-interruptible child token,
  panels show whole rows, copy errors split each path); the application source
  search expects the whole-row page; the elevation trust test accepts either
  protected-installation refusal (a temporary folder that standard users may change
  is refused first on CI).
- Races: engine copy tests copy from the newest frame, since a later position
  refinement retires the input of earlier frames; restricted workers serialize and
  briefly retry creating their per-user AppContainer profile, which failed once on a
  fresh CI user with E_UNEXPECTED while workers started in parallel.
- Rebuilds: a dav1d build directory configured before `vcs_version.h` became a
  configure-time file is configured afresh (reconfiguring it in place deleted the
  header and failed the image viewer build of every existing checkout); verified
  with stale directories on Windows and Linux.

`build.bat` completed end to end on the Windows development machine for the first
time, with inputs this machine otherwise lacks: `MIDNIGHT_HTML_SDK_CONFIG` naming a
web SDK bundle packaged by WSL's classic-zlib Python (its API crate matches the HTML
viewer's pinned checksum), an MSVC FFmpeg 9.0.2 prefix with swresample from the
branch's media work, the pinned FFmpeg source archive and libclang. It built all
viewers including the native CEF HTML viewer, the three filesystems and the
thumbnailers, deployed a private installation and passed its `--smoke-test`.

Not established:

- The AppContainer profile race could not be reproduced where the profile already
  exists; the fix is defensive and one CI run has passed since.
- The manual native CEF Office/HTML and qualified Source acceptance tests.
- Copying in the instant between a find and its position refinement named a retired
  frame and copied nothing; decided and implemented on 20 November (above).

### Post-merge validation — 19 November 2026

The Windows application at `4a6ccf77` built with the locked dependency graph.
The packaged F5 Copy, F6 Move and F10 Delete checks passed. The Delete check
declined a confirmation, then removed only the selected fixture after confirming.
The final independent inventories matched the intended paths and SHA-256 hashes.
Seventy-one transfer and job window tests passed, including both directory
junction modes, and 23 progress presentation and eight viewer control tests passed.
The settings migration passed seven tests and the Office parser passed 68.

Packaged Office Find passed for DOCX, XLSX, PPTX, ODT, ODS and ODP. The test
observed active highlights moving in both directions. Clear restored the visible
content, and an absent query reported zero matches. F3 video playback passed
with a VP9/Opus MP4, including pause, seek and resumed audio. The bundled FFmpeg
build lacks H.264 and AAC. Windows F3 PDF passed on a 128-page mixed-size fixture
using Paged navigation and Continuous drag and scrollbar controls. Ubuntu 24.04
under WSL 2 passed 94 targeted media/viewer tests and two installed PDF worker
tests. All Windows acceptance runs used an authored private desktop and files.

The subsequent Delete regression reproduced a false failure after partial native
cancellation. After the fix, all 15 targeted Delete tests passed, including the
case retaining a genuine permission error. The amended Copy tests passed both
a cancellation with pending work and a late cancellation after full publication.
Ten Windows junction tests passed, and a separate C:/D: run verified the
cross-volume junction fallback and retained source identity.

### Media follow-up — 19 November 2026

The reviewed 64-file Media overlay is integrated. After EOF/empty-stream fixes,
the Media host passes 34 cases. Media output passes all six cases, including
the actual silence-only default Windows endpoint. The earlier native failure
was traced to an incorrectly rejected zero latency; negative latency, zero
frequency and overflow remain rejected. Engine passes seven video cases and
one retirement-mailbox case. Media UI has 10 distinct passing cases across
controls, aspect/resize, navigation and final four-case synchronization; default
physical Ctrl+F is blocked while media is active and opens Find after retirement.
Earlier fixture failures and overlapping runs retain their historical scope.
Backend two-package and integrated eight-package all-target strict Clippy pass;
Python tooling retains 14 passes and 18 skips.

The matching r3 worker consolidates 45 functional passes with both helpers'
five-DLL closures checked. A separate trusted restricted-worker SDK scenario
decodes VP9/Opus: 82 video/131,328 PCM frames before seek and 52 video/83,328 PCM
frames afterward, reaching EOF both times, advancing the seek epoch and
confirming retirement. Loaded DLL paths/hashes match; the owned C-drive package
was removed with a 28-file D-drive mirror retained. The native profile's 54
warnings still leave it unqualified. Whole-app F3, audio through F3, native MP4
and Linux remain open.

Viewer routing passes 12 cases, including video/process-index and user policy
coverage. The plug-in Options replay now passes all 17 cases; root visually
reviewed all seven bundled viewers and the keyboard-revealed last row of an
expanded 20-row catalog above the pinned Rescan action. Earlier fixture and
compilation failures remain retained.

Root reviewed/applied the 17-file typed media frame fence, verifying every
preimage and proposed hash. Its real prepared-batch RED reproduced stale frame
11 replacing frame 10 after a newer seek. The final direct UI/image replay
passes 31 cases and producer/routing replay passes eight; these groups overlap
earlier evidence and are not added to it. They cover missing/future controls,
rapid/refused seeks, replay, later Play/Pause preserving the sole preview and
session retirement. Two existing Runtime fixtures were repaired without
weakening live/closed-session or primary-panic checks. Four-package all-target
strict Clippy with `--no-deps` and warnings denied also passes.

Four earlier completed HTML controls superseded the pending diagnostic status: short
profile OFF/ON, OFF/BASIC without a notifier, software/DX12 OFF and Vulkan/DX12
OFF. That single-backend same-binary Vulkan/DX12 control measured maximum
publication→draw of 18.398 versus 562.788 ms and burst changed-draw gaps of
68.391 versus 528.250 ms. Both opens confirmed WGPU and their single-backend
masks. Earlier opening/path failures remain retained. BASIC also recorded a
766.157 ms redraw span without a notifier; publication delays remain on the
other backends. These results support a renderer-path contribution on this
machine, not an exclusive cause, physical-display timing or general speedup.
Protected-runtime guards and cleanup passed, with sampled compiler counts zero.
The Windows VULKAN | DX12 default now passes its policy RED→GREEN regression:
one parent case covers 12 backend/debug-validation subprocess configurations,
preserving explicit overrides and indirect-call validation. This supplies DX12
availability fallback, not device-failure/runtime retry. The newer same-binary
combined-default OFF timing and separate ON identity controls both pass. OFF
wheel publication→draw max is 18.909 ms, but first/reopen draw still takes
2125.929/1792.499 ms and upstream publication delay reaches a real 498.268 ms.
ON directly observes Vulkan in both opens; its notifier makes it timing-ineligible
and does not prove OFF's adapter selection. The measured host/probe source
`6ad6008b…` and binary `5caa539e…` precede the later frame fence; they do not
identify current HTML provider source. These controls executed the fixed
baseline runtime `MEHtmlProbe-96673981` and its 803,328-byte provider
`9159cf31…9d669`; candidate `MEHtmlProbe-e1593972` was guarded, not executed
in those controls.
Thus the opening/upstream figures do not qualify the current HTML provider.
The linked follow-up records full baseline provenance and its execution receipt.
Runtime/ACL/cleanup guards pass;
all sampled compiler counts were zero.

A subsequent single OFF control executes the current candidate provider
`4a1bdd…3787`, with all 49 source pins, three SDK pins and the CEF archive/receipt
matched to its normal non-diagnostic configuration. It retains host `5caa539e…`,
so it is not the latest whole-app binary. First/reopen draw takes
1589.258/1165.933 ms; wheel input→publication median/max is 46.866/538.527 ms,
while publication→draw max is 18.768 ms. All 24 burst inputs arrive. This
reproduces the upstream tail with the current provider; one control establishes
neither a general speedup nor its cause. Runtime/ACL/cleanup guards pass.

The five-sample font microprobe measures a 1.861 ms first constructor and
approximately 0.3 ms subsequent constructors; first layout is 90.959 ms then
approximately 0.6 ms. Its 2862.328 ms enclosing process interval includes
unattributed time outside those measurements. One process with potentially warm
OS/font caches is not whole-viewer opening evidence; no lazy-font refactor was
made. The first current-source diagnostic provider/BASIC run localizes two
slow wheels after fast dispatch/command return and before the next paint, while
host timers continue. It separately retains large opening UI-delivery gaps.
The later sparse diagnostic observes a 507.466 ms wheel→paint-entry interval
despite 34 short recurring CEF pumps, maximum start gap 16.692 ms and maximum
completed-pump duration 0.355 ms. All 12 isolated wheels have complete groups; nine burst
wheels lack their own group under latest-wheel coalescing and are not claimed
as individual paint acknowledgments. Same-thread CEF ownership rules out an
ordinary cross-thread surface-lock wait; the Chromium compositor/capture path
remains unresolved. Standalone diagnostic/model tests pass 19 cases; the full
native diagnostic strict Clippy replay and build also pass. Earlier lint
failure remains retained. Both runs restored the reserved QA runtime and
checked retirement/guards, with NTFS ChangeTime changes recorded separately.
These are localization results, not a production fix or speedup. The earlier
env-toggle revision passed 20 standalone tests, diagnostic native-provider
Clippy/build and the default cfg-off check, but its configured A/B was **never
staged or run**: the Windows worker environment allowlist (lines 162–173) and
Linux `env_clear` strip that private variable. The
[Build stop receipt](../../../_build/qa/html-cef-compositing-control-20261119-034838-0eb1b8a0/unexecuted-env-control-stop.json)
retains that integration failure; it is not a valid native control.

The replacement [compile-feature handoff](../_build/qa/html-pump-aggregates-20261119/software-compositing-feature/handoff.json)
freezes four source files with 19 standalone tests passed. Strict builds and
one native two-feature comparison now pass the actual global-switch gates
A `0/0` / B `1/1` for both opens; accepted configuration is not GPU-status proof.
The [completed A/B handoff](../../../_build/qa/html-cef-compositing-feature-20261119-040645-8d214e09/handoff.json)
retains long B1 draw/burst gaps and Root's rejection of a production software-
compositing default as an unjustified net regression. B1's approximately 0.1 ms
odd-round publications precede native wheel delivery and cannot be credited as
input responses. Its provider cap leaves three late burst wheels uncovered;
protected runtime restoration passes, while owned-D residue cleanup is recorded
separately. No SDK/environment boundary or production default changed.

The corrected HTML QA probe passes its meaningful RED→GREEN sequence
(5 failures/1 positive pass, then all 9 passed) and example check/build with zero
diagnostics. Native-wheel ordering gates both publication and redraw credit;
quiet periods include observed changed-redraw return. It has no native timing
rerun, and historical timings are not retroactively corrected or pixel ACKs.
The separate [three-arm raster pass](../_build/qa/raster-replacement-native-20261119-007e387d/handoff.json)
succeeds once per mode with two warmups/twelve measured serials, identical
1377×477 raster/DPI geometry, no notifier/CEF/input and zero drops. Even unchanged
image+request-redraw retains a 398.195 ms observer remainder; precreated/fresh
maxima are 401.841/374.680 ms. This operational control neither identifies a
cause nor establishes a speedup or actual selected GPU adapter.

The current native Windows filesystem ABI/SDK gate passes 150 cases with two
ignored and no failures. It uses the fresh 862,208-byte DLL `2200e210…972cbdc`
and its verified source closure, rather than the stale packaged DLL. This is a
headless boundary check; it does not establish installed trust-catalog/F3 acceptance.

The current-source app build, including the frame fence and fixture repairs,
now succeeds. Root verified the resulting `me.exe` exists and recorded its
hash in the linked follow-up. The earlier F3r2 attempt failed before the F3 key: eleven
correct-desktop UIA queries return no rows. Host/native-filesystem runtime paths
qualify; playback does not. All owned processes/desktop/child are retired and
protected guards pass. The actual `1fa0c8d8…` compiled-feature audit confirms
Slint accessibility was disabled. The corrected QA app build now passes with
zero diagnostics: 116,560,896 bytes, SHA-256 `27dd0c65…b2f1898`. Actual compiler
artifacts confirm five required Slint/Parley features and four Windows AccessKit
packages. Dependencies/notices and all 46 capture-pair inventories are verified;
no normal-runtime promotion is claimed.

The [first accessibility/capture canary](../_build/qa/accessibility-feature-20261119/native-preparation/runs/native-ax-20261119-r1/result.json)
passes once in 8.36 seconds under the normal nonadministrator owner. It uses a
private desktop, Winit/software and a synthetic 100,000-row model: raw UIA count
43→43, correct plain Value/Text, password `IsPassword`/bullets with no queried
private-use sentinels, and 57 unique rows per phase (19 visible, envelope 232;
76 cold calls then 57/57, rows 0–99,998). Recorded activation barriers read zero
rows. Both owned processes exit 0 and retire; desktop/thread desktop/DPI restore.
`WM_PRINTCLIENT` API success still yields nine black pixels and fails; owned-
client GetDC/BitBlt matches RGB17/83/149 at all nine points at 216 DPI/2.25 in the
180×135 canary. Root reviewed the crop. This is not default-WGPU/display or F3
playback proof. Check/full-feature build/strict Clippy pass; two owned variant
continuations were canceled, not semantic failures. Concurrent Clippy and cold
font/window startup preclude CPU/latency/memory/speedup claims. Independent Perf
review finds no scoped dispatch blocker while retaining preactivation work and
broader measurement limits. R3v2 later stops staging before writes because the
old reserved runtime is missing; no R3 native run occurs. Old reserved-root
absences predate R4 and are not attributed to our cleanup.

The [fresh R4 outcome](../_build/qa/media-playback-r3/r4-rebased/handoff.json)
is **failed filename-viewer detection**, not a demonstrated production routing
defect. Initial new-36-file staging passes; a pre-native guard then detects an
extra local SID-1001 RX ACE on 47 objects plus ChangeTime changes, with stable
payload identities/hashes and unknown cause. After explicit user authorization,
a D-only observed-state baseline preserves the original record without changing
C ACLs. The actual `27dd0c65…`/verified-native-filesystem run reports
`Normal F3 did not open exact file: None`, zero qualified video observations and
no playback-worker proof. Source review finds an unchecked focus result,
no post-click View-enabled confirmation and no unmatched-window inventory;
filename-only matching can miss a generic error viewer because initial errors
may publish Status without a caption-bearing Frame. Actual viewer/error state
is therefore unknown; the later R5/R6 outcomes below establish no production
routing fix.

The host exits normally with no helper remaining, and desktop/thread desktop/DPI
restore. After a transient code-use cleanup stop, fresh users/processes are
empty and unchanged Verify/Cleanup pass. Exactly 36 new leaves and empty
directories/root are removed; `MEF3-20261119-27dd0c65` is absent. Home ACL, old-root
absences, actual known preferences and integrity checks pass; nine audited
ChangeTime deltas preclude an all-timestamps-unchanged claim. Prior scoped
build/inventory/Clippy/canary passes remain valid. Playback/A/V, installed F3,
MP4, Linux, normal-runtime promotion and general performance remain unqualified.
The [R5 diagnostic](../_build/qa/media-playback-r3/f3-diagnostic-r5/handoff.json)
confirms focus but stops on a compound desktop-inventory assertion before click,
F3 or UIA queries. Missing Win32 detail leaves its cause unresolved; cleanup,
three payload audits, Home/preferences and old-root absences pass. API calibration
finds an owned STATIC window, but an extra-window assertion leaves full enumeration
false; this is not a whole-preflight pass. R6 fails Stage with **zero native runs**
on copy metadata/known SID-1001 RX delta. Its later exact, nonrecursive cleanup
removes **15 leaves + five empty directories + root**, with no existing ACL,
Home or preference change. F3 playback remains unqualified.

The later [R7 attempt](../_build/qa/media-playback-r3/f3-current-981b8583-r7/r7-final-handoff.json)
uses the normal `981b8583…` build and sends F3 once. A generic Viewer exposes Play,
position and `0:00 / 0:08`, but the filename/content gate times out; no pixels or
playback actions qualify. A separate real regression then proves VIDEO's valid
`palette=None` acknowledgment was rejecting its paused first frame despite ready
controls. The [VIDEO palette-gate fix](../_build/qa/current-video-palette-gate-20261119-r1/final-handoff.json)
adds the authored-pixel exemption: **1 behavioral RED → 24/0 selected GREEN** in
eight serial software processes, followed by affected strict Clippy and a normal
app build, both exit 0. That preserved app is `442d307b…e51f42`,
116,562,432 bytes, `profile.test=false`.

The [R8 follow-up](../_build/qa/media-playback-r3/f3-current-442d307b-r8/r8-final-handoff.json)
reaches the exact AVI filename caption and visible `Video` label, then **fails**
because QA still searches for `File content` instead of the observed unnamed
Image. No capture or playback action is reached. The process observer also skips
a raw `\\?\C:\…` playback-worker candidate under its literal root filter; that is
neither accepted loaded-code proof nor evidence of worker absence. R8 establishes
no additional production defect. Both R7/R8 pass strict Stage/Verify/exact Cleanup:
owned C roots are absent, normal hosts exit 0, helpers retire and desktop/DPI
restore. The later [R9 attempt](../_build/qa/media-playback-r3/f3-current-442d307b-r9/r9-final-handoff.json)
qualifies the authored first frame (frame 0, 289 samples) and actual host/restricted-
worker/five-FFmpeg-DLL loaded-code proof. Playback progression still **fails**:
all four captures remain frame 0 while UIA reaches Pause/position `0.097509176`.
UIA state is not repaint/decoder-progress proof. Strict cleanup passes and the
owned root is absent.

The [VIDEO resize-fence regression](../_build/qa/current-video-resize-gate-20261119-r1/final-handoff.json)
then reproduces frame **10 instead of 11** after a status-row canvas resize.
The VIDEO-only correction preserves its media session during image-style scaling;
Text/Source still require newer position generations. **22/0/0 exact selected
cases** pass in 22 serial software processes, followed by strict Clippy with zero
diagnostics and a normal app build. The current preserved app is `2f5f7f25…17dc7`,
116,562,944 bytes, `profile.test=false`.

The first actual [R10 native session](../_build/qa/media-playback-r3/f3-current-2f5f7f25-r10/r10-final-handoff.json)
**passes scoped silent-AVI F3 verification** after an earlier pre-Stage resource
stop. Its 25 authored-pixel observations show initial 0, Play 4→9 in 0.844 s,
paused/resized 15, seek 19, near-end 44→47, EOF 47 and replay 0; malformed AVI
shows a decoding error. Actual host/NativeFS/restricted-worker/five-DLL proof
passes without observer errors. Strict Stage/Verify/exact Cleanup pass, the C
root is absent, the host exits 0 and helpers/desktop/DPI retire or restore.
Root reviewed three downsampled PNG copies with originals unchanged, including
changed color/timeline and fitting resized layout. This is one software-rendered,
private-desktop run with the current normal app and a minimal 36-file runtime:
audio/device clock, physical display, full catalog, MP4/Linux, normal-runtime
promotion and general performance remain unqualified. Later Source/Office and
HTML outcomes below retain separate functional, staging and cleanup boundaries.

[The Media follow-up record](verification/2026-11-19-media-follow-up.md) preserves
receipts, failures and limits. Full suites, packaged execution and all 54
retained requirements remain open; neither overall completion nor a main merge
is claimed.

### Parallel viewer verification — 19 November 2026

The retained read-only ancestry check found original `ecf9374e` in work-branch
base `544b48db` and then-main `22fe65fa` through the earlier integration. That
historical ancestry does not establish functional acceptance or identify today's
main. Commit/push of this branch is the requested closing step; no main merge
or normal-runtime promotion is authorized.
The [first immutable parallel handoff](../_build/qa/parallel-verification-20261119-build-handoff.json)
retains the earlier Source pixel failures; the later renderer correction below
is a new evidence epoch, not a rewrite of that failed aggregate.

| Selected boundary | Verified current result |
| --- | --- |
| [Core/platform filename colors](../_build/qa/current-core-colors-20261119-r1/filename-colors-result.json) | **27/0**; PDF split/custom palette migration. |
| [Independent Source](../_build/qa/current-source-20261119-r1/tests-result.json) | Original **94 passed/1 failed** retained; test-only `1bd9…` YAML oracle correction passes its exact case, with direct YAML groups **4/0 + 4/0**. Production lexer unchanged; no fresh all-95 replay. |
| [Office host adaptation](../_build/qa/current-engine-office-green-20261119-r1/selected-results.json) | Three behavioral RED failures → **29/0**; one separate service case covers all six real first-content formats. Generator-owned links become document-relative at the correct page depth; canonical OfficeTarget IDs and private-document policy remain unchanged. The later six-format native initial-content review below passes its authored fixtures; Find and full fidelity remain open. |
| [Engine Source boundaries](../_build/qa/current-engine-office-green-20261119-r1/selected-results.json) | Acknowledgment **13/0**, syntax **20/0**, continuity **8/0**. Earlier position **29 passed/2 failed/1 ignored** retained; both failed CPU-camera/gutter cases now pass in the targeted renderer-correction epoch below. |
| [PDF/document engine boundaries](../_build/qa/current-engine-office-green-20261119-r1/selected-results.json) | Continuous PDF **7/0**, document view **9/0**, document render **6/0**; selected host checks, not native decoder/workflow acceptance. |
| [Archive](../_build/qa/current-archive-20261119-r1/selected-results.json) | Reader **8/0**, streaming **7/0**, archives **14/0**: **29 selected functional passes**. Direct user-ZIP probe below is separate. |
| [HTML navigation routing](../_build/qa/current-html-navigation-red-20261119-r1/green-result.json) | Stale-fixture RED **4/4** → fixture-only `d035…` **8/0**, with old-root/lookalike refusals retained. Policy unchanged; three existing warnings remain, not a strict Clippy pass. |
| [Unified software UI](../_build/qa/current-unified-ui-20261119-r1/selected-ui-results.json) | **10/0** selected comparison/PDF/options/workflow/theme cases, plus **1/0** existing theme-selection test. Each theme check's 52 internal palette/density combinations are not 52 unit passes. App feature match is 14/14. |

Root viewed three PNG copies converted from retained PPMs: compact PDF/font32,
Onion comparison/font32 and compact green identical-text result. Original PPMs
remain unchanged; Build viewed two retained theme BMPs. These are scoped software
renders. Cargo TEST emitted the `me` **test harness** (`profile.test=true`), not a
new normal `me.exe` or a native app result.

The [requested ZIP probe](../_build/qa/current-archive-20261119-r1/userzip-probe/result.json)
reads a downloaded ZIP of the Salamander sources, **20,096,167 bytes**,
SHA-256 `e882a11e…4f3b92`. Direct debug plug-in open/list observations are
**50.3369 / 1.7479 ms**. The selected member's SHA-256 matches independent Python
ZIP decoding and input identity/hash remain unchanged. UI compilation was
concurrent: this is not an isolated speedup, application path, installed worker
or IPC qualification.

The [direct annotation RED](../_build/qa/current-renderer-annotation-red-20261119-r1/attempt2/red-result.json)
reproduces the fractional-camera pixel mismatch. The accepted correction carries
annotation camera/label origin separately as `f64`, preserving the old body
viewport path. [Targeted GREEN](../_build/qa/current-renderer-annotation-green-20261119-r1/tests-result.json)
passes **70/0**: renderer library 24, the two failed host cases, related raster 41
and gutter 3. Full renderer integration passes **48/0 with 3 ignored**. These are
**118 functional passes at that production-code epoch**, not full-suite/native
acceptance. The original strict-Clippy test-only chunk-iterator failure remains;
`9bfd…` changes only that test iteration, with production hashes unchanged.
Final render/engine strict Clippy passes (`--lib --tests --no-deps -D warnings`),
and exactly **3 annotation tests** replay after the style fix—not all 118 again.

Root's latest whitespace and 2,742-file source-size checks pass. The separate
[normal app/HTML-probe build](../_build/qa/current-app-and-html-probe-20261119-r1/artifacts.json)
passes with `profile.test=false` for app `981b8583…` and probe `17bd4731…`.
The VIDEO palette fix passes 24 selected cases and builds normal app `442d307b…`;
R9 qualifies first-frame/loaded-code proof but fails playback progression. The
subsequent VIDEO resize fix passes 22 exact selected cases, affected strict Clippy
and normal app build `2f5f7f25…`; neither phase rebuilds HTML probe `17bd4731…`.
R10 passes the scoped silent-AVI/native pixel and loaded-code workflow above.
Earlier groups retain their epochs and are not summed with repeated coverage.

The pre-fix [Source R4](../_build/qa/source-r4-launch-20261119-r1/final-handoff.json)
retains its C++ `0.82` rapid/reference failure and separate ChangeTime guard
failure/independent cleanup pass. The [first correction replay](../_build/qa/current-source-leading-context-green-20261119-r1/green-attempt-handoff.json)
remains **79 passed/4 failed across 83 unique cases**. The later host fix preserves
`partial_line`, recovers bounded prefix context and covers short-byte fallback,
real endpoints and corrected UTF-16 fixture geometry. After a retained private-
helper compile failure and minimal visibility fix, the [current replay](../_build/qa/current-source-leading-context-green-20261119-r3/green-attempt-handoff.json)
passes **87 unique cases**, strict Engine Clippy and the unchanged 370-input guard.
[Native R5](../_build/qa/source-current-leading-context-native-20261119-r5/actual-native-result.json)
passes C++ `0.82` cold/warm exact pixels, then **fails overall 0/1** at C++ `0.43`
warm syntax. The [scalar diagnosis](../_build/qa/source-r5-failure-diagnosis/scalar-diagnosis.json)
finds legitimately zero highlighted tokens and correct endpoint metrics, not a
production defect. The QA-only pinned-target neutral oracle has [4 rendered
checks/strict Clippy passing](../_build/qa/current-source-oracle-neutral-20261119-r1/green-attempt-handoff.json);
three overlap the 87 and are not added. Exact pixels and all other positive
chroma checks remain required. [R6](../_build/qa/source-current-leading-context-native-20261119-r6/actual-native-result.json)
completes five destinations (all three C++ and two Lua), then fails Lua `0.999`
plain-Text calibration acknowledgment after 45 s; YAML is unreached. A real Text
coverage failure, not a mode-oracle defect, drives the narrow mapped-edge fix:
retain the opposite anchor and grow once within the existing request budget.
A [meaningful RED](../_build/qa/text-eof-correction-red-20261119-r1/red-attempt-handoff.json)
precedes [91-case GREEN](../_build/qa/text-eof-correction-green-20261119-r1/green-attempt-handoff.json);
the latter passes strict Engine Clippy with 371 inputs unchanged. Those **91 unique passes**
include the earlier 87, the fourth oracle and three Text cases; they are not
added together. [Actual native R7](../_build/qa/source-current-leading-context-native-20261119-r7/actual-native-result.json)
then **passes 1/0/0**, completing **all nine destinations** (three each C++/Lua/
YAML), including Lua `0.999` Text calibration. Stage, native guards, owned EOF,
lease release, Restart Manager and exact cleanup pass; the root is absent.
This qualifies the scoped CPU viewport/syntax/recovery corpus with the recorded
2f/SDK authority, not normal F3, gutter/retained-state or performance acceptance.

The [Source/Office session](../_build/qa/f3-source-office-launch-20261119-r1/final-handoff.json)
retains **2/8** automated passes and Root's [eight-crop review](../_build/qa/f3-source-office-2f-e25-20261119-r1/runs/run/visual-review-initial/root-visual-review.json):
Lua/YAML colors and all six authored Office initial markers are visible before
Find/navigation, with XLSX A1 wrapping over two visual rows. Initial-content
acceptance covers those software/private-desktop fixtures; Find/full fidelity
remain open. [Office Find R3](../_build/qa/f3-office-find-primary-2f-e25-20261119-r3/actual-handoff.json)
stops on first DOCX ownership query (`WinError 5`) before content/query/input:
**0/6**, five unattempted, no captures. App exits 0 and desktop/DPI restore.
The controller conflates window ownership with process lifetime; exact failing
API is unknown. [Safe recovery R4](../_build/qa/office-r3-safe-recovery-20261119-r4/actual-handoff.json)
now passes overlapping 750 leases, final critical/strict-original checks,
owned-controller EOF, all lease releases, zero Restart Manager users across 566
files and exact 566-leaf cleanup. The root is absent, independently checked by
Root. This cleanup does not qualify Find; no UI retry or product defect is claimed.

HTML probe `d5cc2128…` retains its [light checks](../_build/qa/current-html-validation-light-20261119-r1/final-handoff.json),
[guarded build](../_build/qa/current-html-validation-cached-build-20261119-r1/final-handoff.json)
and failed package-level **13/14** feature gate. Actual compiler/runtime unit
review permits only new functional/phase attribution, not whole-app/generated-UI
or timing parity. The earlier Stage failure is retained. [Leased R3](../_build/qa/html-primary-leased-d5cc-a0-20261119-r3/actual-handoff.json)
fails WGPU device creation before HTML. The separate [software R4 control](../_build/qa/html-primary-leased-software-d5cc-a0-20261119-r4/actual-handoff.json)
**passes once**: both opens, all **36 wheels** (12 isolated/24 burst), two CEF CSV
EOFs and `provider.validate` **421,519/503,338 microseconds**. These are validation
phases, excluding browser startup/draw/retirement. Actual software renderer and
unchanged A0 CEF mode 0 are recorded; foreign compiler context precludes idle/
performance claims. All 548 critical-file checks, native EOF, lease release,
Restart Manager and cleanup pass; the owned root is absent. Continuous primary
read leases qualify these QA C copies' content, identity and security; ChangeTime
is diagnostic with unknown cause. D originals retain strict ChangeTime checks.
Old failures are not rebased and application loader policy is unchanged. HTML
hardware/speedup qualification remains open.

The [final normal app build](../_build/qa/current-final-app-20261119-r2/build-handoff.json)
now passes after the Text fix: `5b8b6c76…`, **116,569,088 bytes**,
`profile.test=false`, Cargo/source-guard exit 0, 14 feature unions matching and
cached main Slint bindings. It supersedes `6894ceb4…`. Compile and scoped runtime
evidence do not establish fresh full-GUI/full-suite acceptance or normal-runtime
promotion. Root's closing scope
is to commit and push `codex/post-merge-validation-20261118`, then stop; no merge
to main is authorized. Broader acceptance and remote CI retain their earlier
limits, and overlapping epochs are not summed.

### Find and panel follow-up — 18 November 2026

Work remains on `codex/post-merge-validation-20261118`, based on `544b48db`,
with no current-batch commit, merge or push recorded. Comparator Engine Find
passed 9 cases, including the prior 8 and a new immutable-snapshot case; the
three comparator UI Find cases also passed again. Viewer Engine Find passed 19,
including a replay after lint-fixture changes; cursor 4, real Basic/Source 1 and resident
lifecycle 6; comparator/viewer UI library checks passed 14. Final filter polish
passed all five affected cases. Main UI consolidates 25 unique cases (5 latest
filter passes and 20 retained unaffected passes), not a repeated full target.
The focused filter capture shows one clear X before the history arrow.

Earlier comparator replays fixed keyboard/Esc, hidden Find spacing and resize.
Root then found actual tooltip text overflowing a tiny popup despite the earlier
origin assertion. The corrected containment test failed, then the shared-adapter
and weak post-event anchor fix passed compilation and all 12 exact affected
runtime cases, including both headers' popup and text bounds. Main UI's four
tooltip cases also passed after helper extraction; these overlapping results are
not added to the 25-case consolidation. The real Source 20,000-line position,
Copy and preferences case passed. The dependency inventory records unchanged
615 resolved identities with the existing pinned compiler/helper selections.

Root reviewed Keyboard, selection-marker and normal/narrow Find captures.
The four-run native HTML ABBA measurements were mixed and establish no overall
speed claim. The timeline example passed three tests and built successfully,
but pinned Slint adds flush/submission work when its notifier is installed and
`AfterRendering` precedes surface presentation. The same-binary trace-OFF→ON
native control has a lease and no completed result at this checkpoint.
Strict all-target Clippy for six targeted packages now passes with warnings
denied; prior diagnosed failures remain recorded. Root verified source-size
checks for 2,708 files and diff whitespace. The 64-file Media overlay remains
unapplied while Architect refreshes its worker preimage.
[The follow-up record](verification/2026-11-18-find-and-panel-follow-up.md)
retains the evidence chain and limits. Windows software tests and recorded
Properties arguments do not qualify an OS property sheet, the full suite,
packaged app, Linux or all 54 retained requirements. The overall goal remains
open.

### Screenshot follow-up — 18 November 2026

The follow-up checkout is based on published `main` at `544b48db`. The
warning-free local Explorer `cargo check --tests` below covers compilation.
GitHub Actions run `44363264736` on that same commit completed with five failed
plug-in jobs. The two archive jobs stopped on the current-position seek lint.
Both Source jobs failed an assertion in `checkpoint_peak`. The Windows image
viewer step exited 1 and its cause remains under Build review.
[CI run](https://github.com/TomasJelinek/fm/actions/runs/44363264736) ·
[Archive Windows](https://github.com/TomasJelinek/fm/actions/runs/44363264736/job/135010552908) ·
[Archive Linux](https://github.com/TomasJelinek/fm/actions/runs/44363264736/job/135010553060) ·
[Source Windows](https://github.com/TomasJelinek/fm/actions/runs/44363264736/job/135010780889) ·
[Source Linux](https://github.com/TomasJelinek/fm/actions/runs/44363264736/job/135010780905) ·
[Image Windows](https://github.com/TomasJelinek/fm/actions/runs/44363264736/job/135010553148)

The screenshots are layout and interaction references. The new backlog records
readable settings JSON, the silent Delete report, panel and shortcut polish,
editable plug-in associations, FFmpeg playback and thumbnails, `.manifest` and
`.usda` highlighting, the reported HTML file, and the full Copy/Move workflow.
The checks below now include a locked `.claude` reproduction and native
first-open and reopen smoke runs on the reported HTML file. Comparable HTML
timings and scrolling latency measurements are still pending. Verification for
the screenshot follow-up remains open alongside all 54 retained requirements.
[Screenshot follow-up backlog](IMPLEMENTATION_BACKLOG.md#screenshot-follow-up--18-november-2026)

- **Settings JSON.** New saves normally use two-space indentation and a final
  newline. When indentation and the newline would exceed 4 MiB, the writer falls
  back to compact JSON and enforces the same bound. The initial two-test run
  reproduced the formatting failure, with the near-limit compatibility case
  passing. The corrected run of
  `cargo test -p midnight-platform-common --lib settings::tests:: --locked --offline --jobs 1`
  recorded 21 passed, 0 failed and 101 filtered out. Formatting takes effect on
  save. Verification used fixture preferences and left the user's settings file
  alone. [Settings writer](../crates/midnight-platform-common/src/settings.rs) ·
  [Initial run](../_build/settings-json-red/cargo.log) ·
  [Corrected run](../_build/settings-json-green/cargo.log)
- **Windows settings replay.** On the composed follow-up source,
  `cargo test -p midnight-platform-common --lib settings:: --locked --offline --jobs 1`
  passed 49 tests with 0 failures and 76 filtered out. The replay covered schema
  34 and legacy defaults, compatibility with the required comparator field,
  malformed and duplicate overrides, indented UTF-8 JSON, the size-bounded
  fallback, and atomic writes. The 49-test result includes the formatting and
  near-limit cases from the earlier settings check.
  [Replay log](../_build/thumbnailer-settings-schema34/cargo.log)
- **Windows Thumbnailer backend dispatch.** On the composed follow-up source,
  `cargo test -p midnight-thumbnail-host --lib providers::tests:: --locked --offline --jobs 1`
  passed 3 tests with 0 failures and 39 filtered out. The cases verified stable
  provider precedence, accepted extension overrides, disabling a provider with
  an empty override, restoring its defaults with Reset, image-only artwork
  filtering, and the existing Windows library trust check. The initial compile
  hit a borrow lifetime error in the test closure. The replay corrected that
  closure and exercised the same production resolver.
  [Initial compile log](../_build/thumbnailer-host-associations/cargo.log) ·
  [Replay log](../_build/thumbnailer-host-associations-recheck/cargo.log)

- **Source now in the follow-up.** Panel smooth scrolling, the inset tab close
  control and directional file Move icon are in the checkout. Delete has an
  interruption classification change and a native fixture with a locked handle.
  Copy/Move has presentation V2, a cancel-confirmation pause token and a terminal
  receipt. Windows multi-file Properties and backend lookup of the stored link
  target are also present. The focused transfer and Properties tests are in
  progress. Stored link target behavior still needs qualification. Thumbnailer
  still needs end-to-end qualification, including Options and video formats.
- **Viewer source.** Options has the full Viewer list and editable extension
  associations. The seven-file Source change covers XML `.manifest` and USDA
  highlighting. The two-file HTML timing probe targets the reported local file.
  Extension persistence and routing, syntax highlighting, FFmpeg playback and
  broader thumbnail format coverage still need execution evidence.
- **Early Windows compile and isolated runtime.** The first Engine compile
  exited 101 on two stale test interfaces. The repaired compile produced Engine
  and UI test artifacts with zero compiler diagnostics. Four Engine tests
  passed, including the fixture that distinguishes a preparation interruption
  from explicit cancellation. Three UI library tests passed. Both verified
  Engine fixture repairs are now in the follow-up source.
  [First compile](../_build/qa/follow-up-win-v2/engine-compile-result.json) ·
  [Repaired compile](../_build/qa/follow-up-win-v2/engine-recheck/compile-result.json) ·
  [Engine runtime](../_build/qa/follow-up-win-v2/engine-recheck/runtime/engine-repaired/result.json) ·
  [UI library runtime](../_build/qa/follow-up-win-v2/engine-recheck/runtime/ui-lib/result.json)
- **Early UI integration.** Three Delete and confirmation cases plus drag
  feedback passed. The cursor-reveal test failed to cancel native motion, and
  the tab close geometry assertion failed. The original wrapper also counted
  a render filename lookup error against one passing Delete test. The corrected
  receipt verified the original captures and records four passes and two test
  failures. Root visually reviewed the 700×500 and 350×300 Delete renders.
  [Original wrapper result](../_build/qa/follow-up-win-v2/engine-recheck/runtime/ui-integration/result.json) ·
  [Verified runtime and renders](../_build/qa/follow-up-win-v2/engine-recheck/runtime/ui-integration/verified-runtime-and-renders.json)
- **Windows panel replay.** The isolated `panel-runtime-fix` run passed both
  formerly failing integration selectors with zero failures. The cases cover
  fractional scrolling, whole rows and cancellation of native motion on cursor
  reveal, plus tab close geometry and file-focus shortcuts. The production
  fixes are integrated into the follow-up source.
  [Panel replay](../_build/qa/follow-up-win-v2/panel-runtime-fix/run-20261118-195547-285/runtime/panel-runtime/result.json)
- **Windows locked `.claude` Delete.** The first run exited 101. The fixed
  integration run passed the exact locked-directory test on a disposable
  `.claude` fixture. Delete reported an error while the handle was held, then
  succeeded on retry after release.
  [First run](../_build/qa/follow-up-win-v2/locked-delete-v1/result.json) ·
  [Fixed run](../_build/qa/follow-up-win-v2/locked-directory-fix/run-20261118-203000-164/runtime/locked-directory/result.json)
- **Combined Windows Explorer compile.** On the exact pinned follow-up snapshot
  of 4,145 files based on `544b48db`, `cargo check --tests` exited 0 with zero
  warnings or errors. This receipt covers compilation. Focused transfer and
  Properties runtime results are pending.
  [Combined compile receipt](../_build/qa/explorer-combined-win-v1/check-v2/check-result.json)
- **Windows HTML package and native report smoke.** The separate package passed
  32 tests with zero failures across surface (9), output (16) and Find (7), with
  one surface case ignored. In separate native smoke sessions at 640×320, the
  baseline and candidate both drew the exact reported file on first open and on
  reopen in the same process. Both runs also received all 24 posted wheel events.
  The smoke sessions establish correctness in those runs. Compiler activity
  during both sessions makes their timings non-comparable.
  [Package tests](../_build/qa/html-dirty-paint-win-v1/verified-native-tests-result.json) ·
  [Native smoke receipt](../_build/qa/html-native-probe-win-v1/verified-smokes.json)

### Published main and follow-up evidence — 18 November 2026

The published merge `cf8138e4` was followed by three repair commits spanning
six files. `origin/main` fast-forwarded to `544b48db`, and Root verified that
remote head. This follow-up checkout now has the same base. Its ref and index
advanced after all six repair files already matched the published bytes. The
27 remaining worktree paths and prior sealed evidence retain their own scope.

On the original merge, Cargo PID `33712` exited 101 with `build-finished: false`,
524 artifact events and clean input guards. The two Slint errors were
`Unknown property checked in ActionButton` at `viewer.slint:394` and `:403`.
The 54 retained requirements remain the acceptance scope. Package and product
qualification remain open. [Merged check receipt](../_build/merge-main-20261118/_build/merge-explorer-check-v1/result.json) ·
[Original diagnostics](../_build/merge-main-20261118/_build/merge-explorer-check-v1/cargo.stderr.log)

- **Warm Explorer check.** The one-file PDF fix in local `48db5ec6` cleared the
  binding errors. Viewer and main bindings generated successfully. The check
  still exited 101 with `build-finished: false` after 535 artifact events. Two
  comparator callers still used `try_reserve_nowait`. Commit `45ede585` changed
  only those callers to `try_reserve`. A memory check at
  18:41:30 local reported 617,222,144 physical bytes available and 5,410,189,312
  bytes of commit headroom. After fresh admission, the warmed check of `45ede585`
  launched as Cargo PID `42872` at 18:43:39 local. It cleared the reservation
  errors and exited 101 with `build-finished: false` after 541 artifact events.
  Explorer test compilation reported two `Result` assertions at
  `comparator_external_native_tests.rs:809` and a preferences error-type mismatch
  at `viewers.rs:407`. Engine compilation also reported four unused actor helper
  warnings. The fourth warm check, on exact `544b48db`, qualified. Cargo exited
  0 with `build-finished: true`, 542 artifact events and zero errors or warnings.
  The dev-profile check finished in 39.60 seconds. `cargo check` ran zero tests.
  Runtime and executable qualification remain open.
  [PDF warm check result](../_build/merge-main-20261118/_build/merge-explorer-pdf-recheck-v1/result.json) ·
  [Comparator recheck result](../_build/merge-main-20261118/_build/merge-explorer-retirement-recheck-v1/result.json) ·
  [Qualified fourth check](../_build/merge-main-20261118/_build/merge-explorer-test-api-recheck-v1/result.json)
- **HTML native Find.** H2 finished with 0 passed, 1 failed, 0 ignored and test
  exit 101 in 25.84 seconds. Both offline and online captured HTML roots passed
  Find, Copy and paint checks. The loopback page responded once over HTTP, then
  an identical initial Find received no new callback after event 35. Pinned CEF
  forwards `findNext` to Chromium. Chromium returns early for an unchanged,
  unaborted session when the caller requests no next match. That explains why
  this repeat can leave the caller waiting. The observer retained 630 bytes of
  Chromium log. Cache errors coexisted with successful paints. The two-file
  Find repair is source-ready. The original native case remains unrun on that
  candidate. [H2 handoff](../_build/verification/viewer-refresh-20261117/build/office-html-short-profile-h2-collector-v2/handoff.json) ·
  [H2 trace](../_build/verification/viewer-refresh-20261117/build/office-html-short-profile-h2-collector-v2/observation-handoff.json) ·
  [Chromium 156.0.8078.88 FindTabHelper](https://github.com/chromium/chromium/blob/156.0.8078.88/components/find_in_page/find_tab_helper.cc#L36-L63) ·
  [CEF 8ea0c73 BrowserHostBase](https://github.com/chromiumembedded/cef/blob/8ea0c73/libcef/browser/browser_host_base.cc#L610-L611)
- **Source coverage v2.** The paired Engine coverage and UI position-unavailable
  packets are source-ready after rebasing, with the original C++ fixture preserved
  byte for byte. Independent review found two retained rebase motion ownership
  paths that need correction before validation. The relative handoff covers source
  only. The wider corpus failure below remains the latest verified runtime result.
  [Earlier Engine source receipt](../_build/verification/viewer-refresh-20261117/viewer/source-coverage-recovery-v2/source-ready.json) ·
  [Earlier UI source receipt](../_build/verification/viewer-refresh-20261117/ui/source-position-unavailable-v1/source-ready.json)
- **Delete progress.** The isolated compile exited 0 with zero warnings or
  errors and produced two test artifacts, `midnight_ui` and `native_job_windows`.
  The presenter case and the software window case for the cancelling Close action
  passed in the first run. Earlier attempts at the third case stopped before
  launch at the memory or app identity checks. Root then ran the final close
  confirmation case on the same sealed artifact. It exited 0 with one pass,
  zero failures or ignores, and clean source and artifact guards. That one
  precompiled software case used a resource gate of 2 GiB physical memory and
  8 GiB commit headroom. The two receipts together cover three distinct software
  cases on the earlier private artifact. The follow-up checkout contains the
  four Delete polish files and the separate software regression test from
  packet `f799a7fd`, plus the two-file freshness follow-up that prevents
  a buffered snapshot from undoing accepted cancellation. The combined visual
  check and new cleanup path remain pending. Native desktop validation remains
  open.
  [Compile receipt](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/resume-compile-1/compile-result.json) ·
  [Two passes and memory stop](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/runtime-three-1/result.json) ·
  [Standalone attempt 1](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/runtime-final-one-1/result.json) ·
  [Standalone attempt 2](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/runtime-final-one-2/result.json) ·
  [Observation-only preflight](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/runtime-final-one-3/result.json) ·
  [Final case pass](../_build/verification/viewer-refresh-20261117/build/delete-progress-state-v2-build-prep/runtime-final-one-root/result.json) ·
  [Delete polish source packet](../_build/verification/viewer-refresh-20261117/ui/delete-cancelling-polish-v1/source-ready.json)
- **Archive performance.** The direct-plugin ZIP measurements below apply to
  their dated runs. End-to-end performance validation remains open.

### Checkpoints from main
### Integration validation — 19 November 2026

With PowerShell 7.7 installed on the Windows development machine, and cmake, nasm,
libclang, Ninja and the FreeType headers (the packages of `scripts/setup-linux.sh`)
in its WSL Ubuntu 24.04, `main` was validated end to end on both platforms; the
commits merged since the deep code review were integrated as reviewed. CI on `main`
had been failing: its check job stopped at Format since `4f072e00`, and its Linux
native viewer engine step had failed since the 18 November integration (the Windows
check job did not reach it).

Fixes on `main`:

- *Remove unset variables when restoring them under PowerShell 7*: PowerShell passes
  `$null` to a .NET string parameter as `''`, and PowerShell 7.7 on .NET 11 keeps
  such a variable as an empty one (Windows PowerShell 5.1 deleted it). The web SDK
  packager therefore left `CARGO`, `CARGO_HOME` and `RUSTUP_HOME` empty in a calling
  build when they had been unset, and `validate.ps1` left the variables a run adds
  empty in a calling session; both now remove them, and a test runs `validate.ps1`
  inside a PowerShell session.
- Two *Format … as rustfmt does* commits: `cargo fmt --all` on the 21 files that
  CI's Format step reported up to `ec96710a`; no other change.
- *Borrow the test root as a slice in the About licenses test*: past Format, CI's
  Clippy step (`-D warnings`) rejected `&[root.clone()]` in `about_licenses.rs`
  (`clippy::cloned_ref_to_slice_refs`). CI's Clippy command then passed here on
  Windows and Linux; CI's run of `e9bab4ef` passed every other job, including both
  release builds.
- *Fix the viewer engine's source position and PDF integration tests*: the source
  position test took the first frame after `LineNumbers(false)` as the repaint, but
  under load that frame still carried the numbered pixels (4 of 16 runs failed under
  CPU load; 16 of 16 passed once it waits for the changed frame). The PDF navigation
  tests also discovered web viewers, which validates the application executable that
  a private test installation lacks; they now cache an empty web catalog.
- *Skip the manual native CEF and qualified Source tests in the engine step*: CI,
  `validate.ps1` and `validate.sh` skip `office_preview::native_tests` (trusted CEF
  installation, prepared D: profile) and `tests::source_native_acceptance`
  (qualified product build), manual qualifications tracked in the [feature
  status](FEATURE_STATUS.md).
- *Accept thumbnail notices reached through a linked cargo home* and *Require an
  explicit root for thumbnail notice inputs* (with a security and a compatibility
  review): `scripts/thumbnail-notices.py` refused every notice input whose absolute
  path contained a link, so the thumbnailer build failed with a linked `CARGO_HOME`,
  `~/.cargo` or `/home` (observed with a linked registry crate folder in WSL). Each
  input is now checked against the package root its caller names, which alone is
  resolved (it and its ancestors may be links); below it every name must be portable
  and no link, reparse point or `..` is accepted. The notice tests pass on Windows
  (16, 4 skipped without symlink privilege) and Linux (16, 3 skipped), as do the
  script tests (Windows 253 / 0 / 137, Linux 321 / 0 / 69).

Evidence (orchestrator logs in the integration worktree's `_build/validate/logs` and
`_build/qa`, and in its WSL clone):

- Linux, `./build.sh --validate` at `a8f93b88` in a fresh clone: the build,
  including the pinned FFmpeg 9.0.2 and dav1d, then 37 of 40 steps passed (workspace
  tests 6,740 / 0 / 133, script tests 316 / 0 / 66); the viewer engine step failed
  as in CI and the two PowerShell steps were skipped as Windows-only. The video
  thumbnailer (5 / 0) and HTML viewer (33 / 0 / 1) steps ran on this machine for the
  first time.
- Windows, `scripts\validate.ps1` at `a8f93b88`: 31 of 40 steps passed (workspace
  tests 6,815 / 0 / 143), 3 failed (the script tests on the PowerShell 7 restore,
  the PowerShell 7 deployment tests on the shortcut flake of TEST-04 and the viewer
  engine step) and 6 were skipped; the image viewer, PDF viewer and image
  thumbnailer steps ran here for the first time. The HTML viewer step passed (33 / 0
  / 1) with `--web-sdk` and a web SDK bundle packaged by WSL's classic-zlib Python,
  whose API crate matches the HTML viewer's pinned checksum.
- After the fixes the viewer engine step passed on Windows and Linux (5 / 0), and at
  the integrated tip (`main` `ec96710a` with these commits) the workspace tests
  passed on Windows (6,825 / 0 / 143) and Linux (6,750 / 0 / 133), as did the script
  tests (Windows 248 / 0 / 135, Linux 316 / 0 / 67) and, one commit earlier, both
  PowerShell deployment test steps.

Not established:

- `build.bat --validate` end to end on this machine: `build.bat` now builds the
  host, both filesystem modules and five viewers here, then stops as documented at
  the HTML viewer's web SDK packaging (Python 3.15 is the only Python and bundles
  zlib-ng); its thumbnailer build would also need the MSVC FFmpeg prefix
  (`MIDNIGHT_FFMPEG_PREFIX`, `MIDNIGHT_FFMPEG_SOURCE_ARCHIVE`, `LIBCLANG_PATH`),
  which is why the Windows video thumbnailer step is skipped.
- The manual native CEF Office/HTML and qualified Source acceptance tests (not run),
  and CI results for these commits.
- The desktop shortcut test failed intermittently on this machine under both
  PowerShell versions; it is recorded with TEST-04 in the
  [backlog](IMPLEMENTATION_BACKLOG.md).
- The other build and packaging scripts still handle links inconsistently: some
  refuse a checkout reached through a linked or hidden folder, others follow links in
  package license inputs (BUILD-02 in the backlog; a policy decision first).

This supersedes the first two "Not established" items of the test speed checkpoint
below.

### Test speed and full validation — 19 November 2026

Two follow-ups of the deep code review are on main. The test speed pass
(`57d03d20`..`8dedeac9`, with TEST-04 in `62f3e14d`) cuts the workspace test run
without losing a test: optimized `regex-automata` in development builds takes the
comparator budget test from about 5 s to 0.5 s, the rejected-work engine test runs
its nine kinds concurrently (1.95 s to 0.43 s), three targets without tests no
longer build test harnesses, the Linux-only worker-platform files are modules of
their topic executables, and guards keep unit tests out of `test = false` targets.
Warm runs are about 10 % shorter on Windows and 12 % on Linux, with 63 instead of 69
test executables ([test speed pass](verification/2026-11-18-test-speed-pass.md)).

`build.bat --validate` and `./build.sh --validate` (`e8eb7878`..`a9ea2b53`) run
`scripts/validate.ps1` or `scripts/validate.sh` after the build: every workspace
test target including doc tests, the Python script tests and the separate
workspaces' CI test commands, tests only (no formatting, Clippy or source-size
checks). A step whose prerequisite is missing is SKIPPED with its reason and named
in a warning; `--strict` makes such skips fail; on Windows a build that fails
validation is not deployed ([test execution](developer/TEST_EXECUTION.md)).

Evidence (orchestrator logs in the integration worktree's `_build/validate/logs` and
`_build/qa`): a full Windows run of `validate.ps1` at `f3bde653` passed 27 of 40
steps, including the workspace tests with 6,813 passed / 0 failed / 143 ignored; one
step failed on a flaky directory rename in the script tests, which `34bd4e29`
retries, after which the script tests passed (209 / 0); 12 steps were skipped for
missing prerequisites or as Linux-only. On Linux the workspace tests passed (6,738 /
0 / 133) and, in a git checkout, the 15 SDK, viewer, PDF, archive and qualification
steps passed (473 / 0); the script tests passed (316 / 0).

Not established:

- `build.bat --validate` end to end on this machine: PowerShell 7 is not installed
  here, so `build.bat` stops before building; its validation hooks are covered by
  the build hook tests, which ran here only with a test stand-in for PowerShell 7.
- The image viewer and image thumbnailer steps (they need PowerShell 7 or cached
  pinned dav1d inputs), the video thumbnailer (FFmpeg) and the HTML viewer (its web
  SDK package needs a Python with classic zlib, 3.13 or older on Windows) were
  skipped here.
- Reusing the build helpers' target directories in validation, to avoid compiling
  the modules twice, is not done.

### Merge repair — 19 November 2026

The deep code review (`3ebc2001`) and the viewer, comparator and filesystem
integration work (`ecf9374e`) were developed in parallel from `3a27eeb2` and merged
in `cf8138e4`; `48db5ec6`, `45ede585` and `544b48db` followed. That merge did not
build the test targets of the engine, viewer engine, comparator, plugin host and
explorer, failed workspace Clippy, and failed several comparator and UI tests on
every run. Twenty-seven repair commits on `544b48db` resolve these semantic
conflicts; each commit message names the cause and the side whose intent it follows.

- Engine: production files included into the single `integration` test binary reach
  their modules through `super::` paths; library tests use the merged comparator
  snapshot and resident-worker APIs; the actor-thread module that the resident
  worker pool replaced is removed, and the engine contracts describe the resident
  workers.
- Viewers and comparator: viewer-engine tests start resident pools and use
  shared-pixel document rasters; the comparator test builds, configuration tests
  (now linked into core's integration binary as `file_comparators::`) and fixture
  paths are fixed; a picker closed by an Options page change is published at once,
  also when the change is deferred (a product fix); tests that met the asynchronous
  font catalog frame now wait for it or hold the scan.
- UI, plugin host and explorer: tests follow the merged accessible names, restored
  theme fills, optional-source status, retained viewer windows, web policy
  presentation and picker callbacks; HTML captured navigation below the synthetic
  document root is admitted (a product fix); queued change handlers settle before
  the viewer mode list opens.
- Separate workspaces: the source viewer's tests follow the merged lookahead and
  YAML lexer, and the archive reader keeps its zero relative seek check under strict
  Clippy.

The two TEST-03 timeouts were races inside the tests and are fixed on top of the
repair ([test timeout races](verification/2026-11-18-test-timeout-races.md)).

Validation of the integrated tip (orchestrator logs `vrepair-*` in the repair
worktree's `_build/qa`): Windows: `cargo fmt -p <package> -- --check` for every
workspace package and workspace Clippy with `-D warnings` reported nothing, and
`cargo test --workspace --no-fail-fast` gave 6,780 passed / 0 failed / 143 ignored
in 92 groups. Linux (WSL Ubuntu 24.04): workspace Clippy with `-D warnings` reported
nothing, and the workspace tests gave 6,692 passed / 0 failed / 133 ignored in 92
groups. The separate workspaces' CI commands passed at the repair's `e0ce73fe` (logs
`repair-*-separate*`): on Windows the API, plugin SDK consumer, viewer conformance,
worker package, PDF, archive and filesystem plugin workspaces and the HTML viewer
without its native CEF feature; on Linux the API, plugin SDK consumer, viewer
conformance, PDF, archive and filesystem plugin workspaces (the HTML viewer's
dependencies cannot be resolved offline there).

Not established:

- The HTML viewer's native CEF code, where both sides changed `native.rs`,
  `handlers.rs` and `resources.rs`, has not been compiled: no CEF SDK or `cef`
  crates are cached on this machine.
- The image decoder, image viewer, image thumbnail and video thumbnail workspaces
  got formatting checks only, because their native inputs are not available offline;
  their trees equal the review's validated `3ebc2001`.
- No GitHub-hosted CI run.
- On this machine Microsoft Defender for Endpoint data loss prevention moves the
  NTFS ChangeTime of newly written files of some types, which makes live-validated
  captures report a change; a few comparator tests fail intermittently here for that
  reason (COMPARE-02).

### Slint build speed — 19 November 2026

On main `e7e0f390`, Linux only. Rebuilding the application after a `.slint` change
took 202 s, 187.2 s of it in `rustc` on `midnight-ui-bindings`; it now takes
134 s and 119.2 s ([record](verification/2026-11-19-slint-build-speed.md)).
`rustc` never divides a module over code generation units, and the Slint generator
writes every component into one module, so one unit held most of the crate and was
optimised on one thread. The build script now moves the generated component structs
into 16 private submodules and removes 569 match arms that the generator repeats in
the accessibility lookups and that can never match. Neither pass changes an
expression, a path or a public item; the release profile compiles this crate as one
unit, where the split made no measurable difference.

A targeted `cargo test -p midnight-ui` now reuses the application build's Slint,
bindings and `midnight-ui` library instead of compiling its own (the workspace names
the application's Slint features, and `midnight-ui-input` pins three shared
dependencies as the application resolves them); `cargo test --workspace` still has
its own units. The `midnight-ui` integration tests compile their `ui` helper once
instead of 115 times and `modeless` 21 instead of 51 times, and three fixtures no
longer copy the palette into every binding; the 766 tests of that binary are
unchanged.

The `.slint` sources lost cost that a shared control carries at every use: scroll
views without the bar a site never shows (`VScrollView`, `HScrollView`,
`ui/vlist-view.slint`), a focus scope instead of a hidden stock button in every
`ActionButton`, two controls that no longer inherit a stock widget, and 54 fewer
conditional components (`Tip` owns its empty-text condition and serves 19 more
sites). The main UI's generated Rust went from 30.57 MB and 591 components to
26.97 MB and 523; the viewer and comparator UIs shrank by 0.18 and 0.13 MB. With
them the rebuild after a `.slint` change takes 108 s, 95.1 s of it in `rustc` on the
bindings.

**Open:** run-time speed of debug builds with the split was not measured (the dev
machine code is 1.4 MB larger); Windows and the release executable were not built;
the running application was not looked at with the `.slint` changes, whose evidence
is the UI test suites and comparisons of the generated code.
Measured and left as they are: thin local LTO off, opt-level `"z"` or 0 for the
bindings, an optimised Slint compiler for the build script, and 16 code generation
units for the release compile of the bindings; the record gives each number and
reason.

### Slint compile time and UI sharpness — 18 November 2026

On main `544b48db`, Linux only. Rebuilding `midnight-ui-bindings` after a `.slint`
change took 41.4 s in its build script and 370.1 s in `rustc`; it now takes 10.8 s
and 172.7 s ([record](verification/2026-11-18-slint-compile-time.md)). The causes
were a `rustc` lint that copied the 63 MB generated source for every short block, the
SDK `ScrollView` inlined with both scroll bars at 71 uses, the file pane, error
notice, dialog frame and tooltips copied by the compiler's inlining rules, and
incremental compilation that cost more than it saved for this crate. The main window
now uses its own `ScrollView`/`ListView` (`ui/scroll-view.slint`) and a shared `Tip`.

The same change removes the blur of file-pane content that did not start on a device
pixel and puts the pane split, function keys, status metrics and dialog position on
whole pixels ([record](verification/2026-11-18-ui-sharpness.md)).

A second change delivers native icons at the size they are drawn. The UI reports the
device-pixel edges of rows, tiles and location choices (`Command::SetIconEdges`), the
backend loads each icon at the edges of its kind and again when they change, and the
UI keeps one bitmap per edge. On Linux the provider follows the desktop's icon theme
through XSETTINGS and its inherited themes, picks the theme directory by pixel size
and scales at most once, in premultiplied alpha. With Yaru, tiles are untouched 96 px
artwork and rows are 24 px artwork reduced to 20 px. The icons are therefore the
desktop's own and no longer Adwaita's, including more theme artwork among the
toolbar actions. **Open:** the Windows provider still delivers one 32 px bitmap
(`IconService::icon_edge` keeps it unchanged); action icons stay at 32 px because one
bitmap is drawn at six sizes; SVG-only themes fall back as before.

A third change adds two text fittings to the vendored FemtoVG. `fitted` puts stems
on whole pixel columns through the hinting engine swash already uses and keeps word
spacing with whole-pixel advances inside words. The explorer uses it since
19 November 2026, by the user's decision after comparing snapshots;
`MIDNIGHT_GLYPH_FITTING=subpixel` or `=snapped` selects the earlier rendering or the
intermediate one for a run. **Open:** `fitted` can leave ink up to one pixel from
layout positions, which may show where selections, carets or the quick-search prefix
clip text and in scrolled line edits; that was read from the code, not yet seen or
ruled out in use. The vendored renderer gains a `skrifa` edge and the explorer a
direct `femtovg` one, with no new package ([third-party record](THIRD_PARTY.md)).

UI test results equal main's in the same Docker environment: 19 tests of
`midnight-ui` and `midnight-viewer-ui` fail there before and after, and the
`midnight-compare-ui` library test target does not compile on main. They are listed
in the compile-time record and were not investigated. Not run: Windows, the full
workspace gates, release builds.

The icon and text changes were developed and measured on `544b48db`, before the
merge repair above: their records name the tests that failed there, which the repair
has since addressed, and the engine's test targets were made to compile locally to
run the icon worker's tests. The branch was then merged with the repaired main, whose
own fixes of those targets replace the local ones; what was run again after that
merge is at the end of the
[sharpness record](verification/2026-11-18-ui-sharpness.md).

### Deep code review — 18 November 2026

A deep code review of the whole repository, started from main `3a27eeb2` on
17 November, is integrated on the review branch at
**`d0f44c55`** (585 non-merge commits). Reviewers covered the engine, core,
platforms, hosts, public APIs, plugins, UI, viewers, build scripts and CI; the
Orchestrator, acting as Architect, decided the open design questions. What the
review deferred, and the residuals it accepted, are rows of the
[backlog](IMPLEMENTATION_BACKLOG.md#deferrals-and-residuals-of-the-18-november-review),
each with its trigger. The last batch merged at `cd66e919` with the follow-up
`d0f44c55`: explicit bidirectional controls in shown names and paths, the event-woken
thumbnail pulse and the comparison font restored into open comparators (UI and
Viewers below). This documentation pass and the readable Recycle Bin refusal
(`9d25722e` and `4ab0b16f`, merged at `ac4c2e87`) followed.

Changes by area (the linked contracts describe the current behavior):

- **Engine.** A transfer holds its disk-I/O gate slot only while it executes native
  steps, not while paused or waiting for a decision or admission, and a quarantined
  job returns its slot; a namespace change that a running peer refuses is repeated
  without a prompt once that peer changes; each Copy and mutation job reports its
  terminal progress once ([transfers](developer/TRANSFERS.md),
  [operation progress](developer/OPERATION_PROGRESS.md)). Recursive Attributes/Times/Case
  pin each child only while processing it, from a name snapshot of its folder
  (META-03). An archive session whose cleanup is unconfirmed is ended by its last
  owner, and provider-level owners give their references up instead of forgetting
  them; cancelling one request never ends a shared archive worker, and a busy
  archive navigation waits for a worker
  ([engine lifetime](developer/ENGINE_LIFETIME.md#archive-navigation-and-queued-size-work)).
  Flat watches keep the shown rows watched across pending and failed native
  listings and prepare a navigated folder once its listing has read its first
  entries; a polling-only or paused watch status is shown, and a panicked watch
  worker restarts at once, then at most once a minute
  ([watch status](developer/PANEL_DIAGNOSTICS.md#watch-status)). Tree watch batches
  swap without a gap, tree branches are probed for presence instead of listed, and
  provider references the tree model no longer holds are reclaimed
  ([tree watches](developer/TREE_PANEL.md#native-watches-and-reconciliation)).
  File Lists, size warnings and tree diagnostics use per-user private spool roots
  with a per-process fallback, and Linux spool bases are resolved through links by a
  resolver shared with the filesystem host. Idle clipboard sessions and open
  drag/drop sessions wait for operating-system events instead of 10 ms pumps. Find
  formats result times only for the results it keeps or publishes, and for the one
  duplicate group whose refusal by the retained-bytes budget ends the search
  ([Find](developer/FIND.md)); checksum reads are admitted by source storage kind,
  and shutdown drains the Copy and mutation actors before aborting tasks.
- **Platforms.** Windows treats only name-surrogate reparse points as links
  (OneDrive/Cloud Files placeholders, deduplicated and WOF files are ordinary
  entries), watches folders through its own `ReadDirectoryChangesW` service instead
  of `notify`, pins Move destinations and recovery parents reached through
  junctions, falls back to the classic delete disposition for permanent deletion on
  FAT32/exFAT (not verified on real media) while Copy and Move refuse such volumes,
  waits out a folder Rename refused while a file below it is open
  ([record](verification/2026-11-18-windows-directory-rename-refusals.md)), and
  reports a Recycle Bin move refused because the item or an entry below it is open
  as busy, with a message that keeps the Shell code and names this application's
  panels, expanded folders and viewers as possible holders (TRASH-01). Linux
  stages Rename and permanent Delete on mounts without POSIX modes under one
  admission rule in the platform crate and the native plugin, with ACL-aware
  private-group parents and overlayfs-aware mount checks; a completed Rename or
  Delete with a leftover staging folder ends Completed with errors; X11 drops that
  showed Copy report their refusals; the X11 clipboard waits in `poll()`; terminals
  start through `x-terminal-emulator`. On both systems the directory authorization
  helper runs only from an installation that standard users cannot change.
- **Hosts.** The filesystem host ends an archive worker on protocol violations and
  wrong reply types and reports the cause, stops staging before the spool volume
  would fall below max(1 GiB, 2% of its capacity) free, prefers a spool root whose
  volume has room and falls back to a per-process root, and keeps actively read
  members cached beyond its 512 MiB budget ([filesystem API](developer/FILESYSTEM_API.md)).
  The plugin host loads libraries outside its registry lock, refuses a full registry
  before loading another library and reports fatal viewer failures at once. The web
  viewer host session is event-driven and publishes a browser-initiated close
  ([record](verification/2026-11-18-html-idle-wakeups.md)). Editors, Custom Commands
  and the software-rendering restart are spawned under the process-creation lock,
  closed thumbnail worker streams report the worker's exit code or signal, and
  thumbnail captures are admitted by storage kind.
- **APIs and plugins.** The filesystem plugin SDK bounds consecutive removed entries
  in one list page and truncates over-long diagnostics, descriptors are validated
  more strictly, and the thumbnail SDK 0.2.1 release is held back (SDK-01). The
  archive worker decodes a solid 7z block once and keeps its siblings within a
  128 MiB window, keeps reads across seeks and keys ZIP duplicate records like its
  reader. The Linux native plugin lists each row with one no-follow `fstatat`,
  mounts no automount point and classifies links alike in Stat and listings; the
  Windows plugin fails listings on enumeration errors; the macOS plugin keeps
  unrepresentable rows. The Basic, Source and Markdown viewers fixed paging, budget,
  checkpoint and search defects; Markdown now also resynchronizes its checkpoints
  after long GFM tables and HTML blocks of CommonMark types 6 and 7, while other long
  blocks still end checkpoint discovery (VIEW-01). AVIF items without a displayable
  frame fail instead of spinning, the HTML provider builds from the verified CEF SDK
  in place, and the video thumbnailer reads large packets and samples partial videos.
- **UI.** Escape, X and the header close of a decision window defer it like Decide
  later; drop refusals reach the status line without an empty path suffix; a
  refused authorized retry states the helper requirement; the status line says when
  startup shows defaults because settings are still loading; the About window keeps
  the Slint attribution visible but inert; the pane shows its watch status. Names
  and paths shown in rows, tabs, the path bar, dialogs, errors and status text, and
  in viewer and comparator captions and status lines, display the explicit
  bidirectional controls as `\u202E`-style escapes, while operations, copy actions
  and name fields keep the raw names ([dialog ergonomics](developer/DIALOG_ERGONOMICS.md)).
  Fixed UI timers no longer run while idle, except the drag bridge's 250 ms safety
  net (about 80 to about 10 idle wakeups per second with the window active, in
  [indicative measurements](verification/2026-11-18-ui-idle-wakeups.md)), and the
  thumbnail service's notifier wakes the thumbnail pulse for its cache location and
  notices instead of the pulse polling for them. Viewer preferences are restored
  into viewers, and the comparison font into comparators, opened before the settings
  read, and unused UI surface and its tests were removed.
- **Viewers.** Text frames are painted into the buffers they share, PDF pixels move
  into their shared buffer on the image actor instead of the viewer worker, rasters
  snap to whole physical pixels, glyph painting is culled by ink reach, viewer
  workers wake by events with a reserved wake slot, and page text is shared as
  `Arc<str>`. Explicit bidirectional controls in document text are shown as escapes
  by the Basic viewer and the comparator and as U+FFFD by the Source and Markdown
  viewers, and the comparator paints its first frame in the preferred font and
  presents output on worker wakes ([file comparison](developer/FILE_COMPARISON.md)).
- **Tests.** Most crates' integration tests link into one `integration` binary (see
  the exceptions in [test execution](developer/TEST_EXECUTION.md)); many real sleeps
  were replaced by virtual clocks, events and bounded waits; Windows namespace tests
  run in both lookup modes; native DACL fixtures replaced PowerShell ACL fixtures;
  the user-command probe compiles once per compiler and source; Copy/Move planning
  tests name the qualified NTFS volume they need. Beyond the clipboard
  transfers named in the 17 November entry below, the root F5/F6 transfers of
  `tree_move` now also hold scalar watch hints through the transfer terminal ([test
  execution](developer/TEST_EXECUTION.md)). Running each Windows workspace test
  executable alone, one after another, took 238.5 s at `d0f44c55` against 2,103.9 s
  for the same kinds of targets at main `3a27eeb2` (2,147.5 s with the 29 examples
  that only that run built), with 6,155 instead of 5,364 passing tests ([test
  execution
  time](verification/2026-11-18-test-execution-time.md)).
- **CI and build.** New `viewer-conformance` and `thumbnail-image` jobs; the `check`
  job runs the isolated X11 drag/drop and clipboard tests (Xvfb `-noreset`), the
  installed archive worker tests and the image decoder's native helper tests. Build
  deployments are replaced instead of accumulating and are serialized per output,
  private deployments no longer copy the authorization helper, the pinned toolchain
  is installed explicitly, and the Windows build wrappers decode native output as
  UTF-8.
- **Documentation.** Contracts, plugin READMEs, README and SECURITY.md follow each
  change; the dependency inventory was confirmed by a complete 46-pair capture and
  `--check` ([third-party record](THIRD_PARTY.md)); this pass adds the backlog
  deferrals, updated feature-status rows and this checkpoint.

Evidence. The review's dated records are
[recursive metadata traversal](verification/2026-11-18-recursive-metadata-traversal.md),
[Windows directory rename refusals](verification/2026-11-18-windows-directory-rename-refusals.md),
[HTML idle wakeups](verification/2026-11-18-html-idle-wakeups.md),
[explorer idle wakeups](verification/2026-11-18-ui-idle-wakeups.md), [test execution
time](verification/2026-11-18-test-execution-time.md) and
[separate workspaces and opt-in qualification](verification/2026-11-18-workspaces-and-opt-in-qualification.md).
They build on [HTML frame pacing](verification/2026-11-12-html-frame-pacing.md) (the
wheel-latency viewport), the [Linux HTML qualification](verification/2026-11-10-html-linux.md),
[archive streaming](verification/2026-11-09-archive-streaming.md) (the installed
worker baseline), the isolated X11 commands of [file drag and drop](verification/file-drag-drop.md)
and the [Linux file clipboard](verification/linux-file-clipboard.md) (updated in this
review to `-noreset` and the CI command) and the
[directory authorization record](verification/2026-11-11-directory-authorization.md).
The explorer idle record predates `ba288e03`: since then the thumbnail pulse no
longer runs while it waits for the cache outcome, and the two cache-location tests
that the record names were replaced by
`the_ready_notifier_publishes_the_cache_location_to_a_quiet_pulse` and
`an_awaited_cache_location_no_longer_keeps_the_pulse_polling`.
Full workspace runs at intermediate integration tips are orchestrator logs in its
integration worktree's `_build/qa/`, not dated records: at `93d36411` workspace Clippy
reported no warnings on Windows and on WSL Ubuntu 24.04, and the workspace tests
gave 5,944 passed / 0 failed / 122 ignored in 88 test-result groups on Windows and
5,802 / 0 / 118 in 85 groups on Linux; at `05acd6a9`, 5,957 / 0 / 122 and
5,818 / 0 / 119, again with Clippy reporting no warnings. The batches merged after
`05acd6a9` were checked by the targeted runs in their package reports, and the tip
`2050f745` again by full runs (logs `vpre-*`): workspace Clippy reported no warnings
on Windows and on Linux, and the workspace tests gave 6,138 passed / 0 failed /
127 ignored in 88 groups on Windows and 5,989 / 0 / 128 in 85 groups on Linux. The
batch merged after it at `cd66e919` passed its package's Windows runs (core, UI,
viewer UI, comparator and explorer tests); with the follow-up `d0f44c55`, Windows
workspace Clippy reported no warnings and the tests of the compare engine,
comparator, viewer UI and explorer gave 350 passed / 0 failed / 22 ignored (logs
`b10e-final-*`). At `d0f44c55` no test failed in the timed Windows run: 6,155 passed
/ 0 failed / 127 ignored in 66 test executables ([test execution
time](verification/2026-11-18-test-execution-time.md)). At `ac4c2e87`, after this
pass's Recycle Bin merge, `cargo fmt --check` and workspace Clippy reported nothing
on Windows and the workspace tests gave 6,156 passed / 0 failed / 127 ignored in 88
groups (logs `vfinal-win-*`); on Linux workspace Clippy reported no warnings, and in
a third run the workspace tests gave 6,004 passed / 0 failed / 128
ignored in 85 groups (log `vfinal-linux-test3.log`). The two earlier Linux runs of
`ac4c2e87` each had one timeout, a different test each time; both are fixed in the
tests only (TEST-03, [test timeout
races](verification/2026-11-18-test-timeout-races.md)).
`navigation::blocked_left_listing_does_not_prevent_right_completion` was a race
inside the test: it waited for its listing to reach the provider without reading
its 8-slot event channel, which the Engine's unsolicited startup and navigation
events could fill before the listing sent its start event. The timeout of
`operations_native_ui::real_copy_conflict_shows_both_entries_in_the_native_locale`
is attributed, by its mechanism and an injected-delay reproduction with the
identical message, to a click on a row before the Engine's saved preferences
restored the tabs, which replaced the panel, so F5 found no target; no trace of the
historical run exists. The engine tests now read events while they wait, the
real-Engine UI harnesses wait for the restored tabs, and the operations harness
checks the cursor after each click.

Not established by this checkpoint:

- No GitHub-hosted CI run of this history (CI-02 in the backlog). Of the 17 November
  hosted failures below, the cause of the Linux tree-watch gate 43 failure remains
  unattributed; the clipboard gate 40 failure was traced to fixture ordering and
  fixed in tests only, which no hosted run has confirmed yet.
- No deployment or launcher registration of `d0f44c55` is recorded; the 18 November
  measurements used private probe installations of their own builds. The c97 EXE
  (`2d906dea…`) and the other runtimes named below predate this review's source, so
  the 17 November statement that Windows production is unchanged from the c97 EXE
  no longer holds. Private runtimes deployed by the pre-review scripts, including
  the `MidnightExplorer-20261117-*` deployments below, contain a
  `midnight-elevated-listing.exe` that their own older `me.exe` accepts from a folder
  the user can change; the [README](../README.md#windows-x64) says to redeploy and
  then delete them.
- Not run on Linux: the real-Chromium measurements of the event-driven HTML host
  session and the explorer idle measurements (Windows only), the comparator's
  first-frame font check (COMPARE-01), the tests of the last batch (the bidi escapes
  of shown names, the thumbnail notifier, the comparator font restore), builds and
  tests of the image, AVIF and video plugins (formatting only), and the engine
  stress runs. Not run at all: PowerShell 7 script paths, any Linux CEF build, a real
  CEF/MSVC build with the final HTML build and packaging scripts (`71c7804f`,
  `1ddcccd4`; the Windows HTML measurements used a package built with the scripts of
  `93d36411`), the interactive Windows desktop tests, physical IME and native Wayland
  input, real FAT32/exFAT media, live mounts without POSIX modes and accepted
  UAC/polkit prompts (PLAT-02).
- `same_navigation_success_then_exact_commit_quiets_watch` once failed under load;
  no cause was found, and it now prints the unexpected event, fence and hint log on
  failure (TEST-02 reopens it on any recurrence).

Open work is in the backlog: CI-01 and CI-02; PLAT-02 and PLAT-03; VIEW-04 and
UI-PERF-01 (idle wakeups); PERF-04 (unmeasured hot paths and the corpus test time);
ARCH-01 (small-content budget, implemented on 20 November); ARCH-05, ARCH-06 and
LIFE-02 (captures retained after a taint or quarantine); ARCH-07, REL-01 and SDK-01 to SDK-04 (next API/SDK
releases); META-04, WATCH-01, SPOOL-01, LINK-01, IO-02, TREE-PERF-01, TREE-02,
INPUT-01, PROC-01, PLUGIN-01, COMPARE-01, COMPARE-02, BUILD-01, TRASH-01, TEST-01, TEST-02, TABS-01,
NOTICE-03, IO-03 and OPTIONS-02. The role table under
[current role boundaries](#current-role-boundaries-and-exclusive-ownership)
describes assignments made before this review; the review assigned no further role
work beyond these rows. The [corrections table](#corrections-to-stale-staged-references)
records the retirement capacity of the pre-merge main checkpoint and records that
the Feature status Dialogs row and the dialog ergonomics contract are corrected.

### Viewer, Office, PDF and archive refresh — 17 November 2026

Batch input: `codex/viewer-highlighting-office-pdf-archive-20261117`, following
main `3a27eeb` / CI `44040410089` **20/20 SUCCESS**. That prior CI and the running
app do not contain or qualify this branch's new changes. No new app artifact or
branch merge is claimed; local API/SDK publication is scoped below. The retained 54-requirement goal remains active.

**Work-branch checkpoint — 18 November 2026:** the Source mapped-prefix
production fix and retained test are canonical (25af/d394, snapshot c500b7e0).
Private evidence remains 3 behavioral RED->18 GREEN plus five prior controls and one
new exact retained test pass; the prior 23 were carried without reexecution.
Build is assigned matching new normal/complete-C production inputs, canonical 24
integration and original Source 34 native validation; none is accepted yet.
Office's original five cases remain 2 PASS/3 FAIL/0 UNRUN. Two separate diagnostic
observations failed initial wait-loaded with zero events/requests/frames; their
cause is unknown. Backend 109 compilation failed, leaving 74 runtime cases unrun.
C9297 remains frozen at its epoch; all 54 requirements and final cleanup remain
open. No publishing or Git integration is claimed. Earlier results retain their
named source/environment.

The later work-branch producer and canonical 24-case integration passed on the
`c500b7e0` source epoch with normal executable `d872aabc` and complete C40dd
validation inputs. The installed-provider frontier test passed in the private
repair harness. The wider C++ corpus failed at ACK 22, before rapid C++ or the
Lua and YAML cases. Repeated HTML Find and Source native acceptance remain open.
The merge candidate requires its own build and runtime evidence.

| Slice | Implemented/verified scope | Remaining boundary |
| --- | --- | --- |
| Private ABI 1.4 locator API v2 | Compiled without diagnostics. All 47 test bodies passed, including 14 new locator cases. The launcher returned 125 after a child-presence observation and saved no raw Cargo exit. The exact child, Cargo and supervisor identities were later confirmed absent. [API run outcome](../_build/verification/viewer-refresh-20261117/build/locator-listing-wire-v2-validation/observed-outcome.json) | API cursor wire validation only. The launcher result remains 125 and raw Cargo exit is unknown. |
| Private locator SDK v1 | Matching composed slice compiled cleanly with Cargo and wrapper exits 0. All 58 library tests passed, including nine locator cases. All 100 source checks showed zero drift. [SDK handoff](../_build/verification/viewer-refresh-20261117/build/locator-listing-sdk-v1-validation/handoff.json) | SDK cursor mechanics only. Host patch in progress. Installed provider and unlimited tree UI acceptance remain open. |
| Filename color migration | Three private baseline cases failed solely on the old document and text gray. All 27 `filename_colors` tests passed after the in-place migration, covering loaded rules and preservation of customized colors. The canonical branch now contains the same two tested files (`a26e1266` production and `413c88e3` tests), with formatting and diff checks passed at integration. [Independent review](../_build/verification/viewer-refresh-20261117/root-document-color-migration-v1-actual-red3-green27-review.json) · [Source integration record](../_build/verification/viewer-refresh-20261117/coordination.md) | The running and distributed executable predates the migration. Installation and visual acceptance remain open. |
| Private Source frontier repair | Clean rebuild, 12 headless passes and one pass of the same real installed-provider case. The preserved baseline failed at the final position generation assertion, observing 2 where it required 1. [Headless results](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-headless/HANDOFF.json) · [Installed-provider pass](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-native/handoff.json) · [Baseline failure](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-provider-red-v1-native/handoff.json) | These passes cover the isolated repair. The wider corpus result is recorded below. |
| Private Source corpus run | C++ reached ACK 21 and failed waiting for ACK 22 at `0.56375`. Both pages applied and completed. Window `[42563, 43236)` began at mapped `0.56451339`, and `[42410, 42921)` ended at `0.56307317`. Result 0 passed, 1 failed, 0 ignored, exit 101 in 64.39 seconds. Pre- and post-run guards each checked 7,116 inputs with zero drift, and the test process and observed helpers were gone at closure. [Corpus result](../_build/verification/viewer-refresh-20261117/build/source-window-frontier-repair-v1-corpus/source-frontier-corpus-1/result.json) | Rapid C++, Lua and YAML were not reached. App acceptance and highlighting validation remain open. |
| Source coverage v1 review hold | Source-ready packet held for exact mapless and file-end fallback, queued wheel deltas that can move the ACK frame, and a failure path that can lose newer queued input. An exact retained recovery test is also pending. [Source-ready receipt](../_build/verification/viewer-refresh-20261117/viewer/source-coverage-recovery-v1/source-ready.json) · [Review hold](../_build/verification/viewer-refresh-20261117/coordination.md) | Compilation and runtime validation remain pending. Product qualification remains open. |
| Additional HTML diagnostic, h1 | Archived harness `3666f4e2`, C9297, same single test and deadlines. The profile measured 209 UTF‑16 code units. Sixteen snapshots sampled two renderer identities. Header `(1, 1, 5)`, Loading 1 and Loaded 5 arrived. The run recorded no captured requests, completed or offset-zero source reads, or retained frame. It failed in the initial stage with 0 passed, 1 failed, 0 ignored and exit 101 in 20.83 seconds. Both guards checked 2,511 entries with zero drift across 2,487 install files, and no owned processes or monitor errors remained. [Run result](../_build/verification/viewer-refresh-20261117/build/office-native-phase-observation-v1-native-prep/h1/0/result.json) | The earlier 231-unit attempt received no header. Scratch path and run conditions differed. Path-length causation, content and paint acceptance remain open. |
| Source Stage A | Source recovery post-review fixes passed **64 unique checks / 0 failures across seven groups**, strict Clippy/format 0; prior 99 is pre-fix evidence, not rerun or additive. Real P2 RED→GREEN: target-boundary comparison count **67,117,056 → 16** in one callback; six offscreen pending cases **16 reads / 262,144 B / Ready=false → 1 read / Ready=true**. Retained-syntax upper bound **49,848 B** includes an 8KiB backward ring/overhead; measured giant-line incremental Rust heap peak **566,776 B**. | Preparation source SHA `74ee6b89…` retains its historical scope. The later current native Source run actually failed 0/1/0 at the frame deadline; see the current native row below. Earlier fixture/allocator observations do not qualify RSS, native rapid-scroll or the final app. Local API/SDK 0.2 and host 98 retain their scopes. |
| Canonical SDK packaging | Canonical packager **9/0/0** (lib 4+compatibility 5), strict Clippy/format 0. Normal packaging entrypoint exited 0, preserved exact 0.1 inputs and reproduced issued 0.2 checksums. Relocated upload payload with a fresh Cargo cache resolved Basic/Markdown 0.1 and Source 0.2 locked/offline without app/API source trees. | This supersedes the earlier private-registry-only limitation. Sparse-consumer proof is dependency resolution, not plugin execution, a new remote-CI run or a new app. Packager final source snapshot lacks a pre-build packager-source inventory; public/release-input before/after guards are separate. |
| Archive reader | Seek-preserving buffered reader and **50/0/0** selected plugin tests. Controlled 2,048-member fixture: **4,101 reads / 580,977,649 B → 3 reads / 402,507 B**, sampled IDs/contents unchanged. Exact user ZIP direct-plugin OLD/NEW pair: index **761.5608 / 92.0232 ms**, File reads **10,617 / 59**, returned bytes **2,449,672,583 / 14,990,974** (**163.41×** reduction). Ordered root metadata/pagination match; read/seek errors and storage calls 0; both explicitly selected ignored probes exited 0 once, children exited. | One unoptimized OLD-first pair, unknown/warm cache; snapshot CPU 73/76% and 30/33 foreign compiler-related processes (not active-job counts), none owned; no user process stopped. File-interface counters are not physical disk/broker I/O. Metadata stable, without source hash/native file-ID proof or member reads. This is not isolated latency, production native-capture or GUI opening qualification. |
| Comparison modes/themes | UI handoff confirms Side by side / Swipe / Onion skin buttons and restoration of all **26 historical theme selection fills**, retaining cursor outline/stripe and contrast policy. Five exact headless selectors, visual review, format/diff checks and scoped Clippy passed; lease released. The earlier shared gate includes **27 comparison-engine checks** covering typed whole-input identity; the later routing gate is separate below. | Requested mode is distinct from acknowledged rendered mode. Typed UI outcomes, real Options input and the final prominent green verdict still need their integration gate. |
| Configured comparison routes | The 27-file freeze with 30 newly authored tests now has a targeted gate: **77 passed / 0 failed / 2 ignored**, actual raw log verified. Strict Clippy/format **0/0**, source drift empty and protected app unchanged. Architect/Perf reviews found no additional actionable defect. | Do not add 77 to prior 275: scopes overlap. Four ignored Windows native comparator cases are authored, not qualified. Native Unknown outcomes and ReadAt counts remain unqualified; UI/Explorer and actual native-process acceptance remain separate. |
| Office preview | User chose built-in readable contents; XLSX was the first service slice. Prior scoped Core/common/Office Wave 1 **162/0**, QName **RED 1 → GREEN 9/0** and the 22 Viewer adapter cases within prior 275 retain their dated scopes. The coherent six-format service 27-file and Viewer first-content/native fixture six-file freeze now compiled cleanly and passed **113/0/0**: service lib 68, XLSX integration 18, Viewer adapter/frame-guard 27. | Formatting 54 files passed. Three reader `collapsible_if` lint repairs are returned; strict Clippy rerun pending. These overlapping scopes are not additive. DOCX/XLSX/PPTX/ODT/ODS/ODP have scoped backend evidence, not completed native/app support. Initial readable contents on open is required; security/Linux/package and remaining native-policy acceptance stay open. |
| Office recorded runtime checkpoints | Navigation and independent close/cancel retain their actual native passes. Latest parent diagnostic `office-9fe36f20a6` remains **0 passed /1 failed**: completed real Find returned 0, Copy failed and Session stayed live. | The blank 902×602 diagnostic image is **not a successful fresh content/resize proof**: resize_ok=false and fresh_contrasted_frame=false. Original Find failure remains final; retained old bitmap is not live DOM proof. Source refusal remains mandatory; provider/Find/loading/correlation cause is not established. |
| Promoted Office revocation repair | Host 47 +Viewer 44 =**91/0/0** retains its preserved source binding. The later exact test-assertion lint repair passed strict Clippy 0/format 13 files 0 and two overlapping resource tests. | Do not add those two into a 93-unique total. Current parent diagnostic is still RED despite live Session; no new validated resized-content proof or full native Office acceptance. |
| Office performance/allocation | Amended **four canonical + two private cases passed / 0 failed**; seven result records include the private compiler, not a seventh test. Actual method 8 combined-work budget rejection and same-session small second-sheet render recovery passed. The 10,000-entry / 1,048,512-name-byte fixture observed open requested-live Rust heap peak increment **11,322,758 B**, retained **6,013,594 B**, first-render peak **192,183 B** and after-session-drop increment **0**. The production service freeze is **RELEASED**. | Preserve the prior three passes/one compression-level fixture setup failure as historical evidence. These are synchronous quiescent requested-allocation observations, not RSS, exact concurrent peaks, a universal 32MiB bound or leak proof. Allocator-instrumented timings and foreign compiler contention prevent broad speed claims. The historical performance-test Clippy 101/format 0 remains recorded; the newer 60-check gate has scoped Clippy 0, not an all-targets claim. No native Office or app qualification follows. |
| PDF rule migration | Corrected fresh-artifact full filename_colors suite **20/0/0**, strict Clippy/format 0. Exact legacy combined PDF/document rule migrates locally to PDF **#EF5A5A**, preserving unrelated custom rules and the full 32-rule list if splitting has no room. | Retain the initial 12/8 stale-library attempt caused by Copy-Item mtime, not a behavior regression. Corrected library/harness fresh=false compilation established the final result. Descendant-style transport remains separate. |
| Shared host, Viewer and comparison gate | Root verified **275 unique headless passes / 0 failures / 0 ignored**: native-filesystem host 41, plugin host 24, web host 29, comparison 27, Viewer 146 plus 8 new image-input/worker cases. **325 executions include 50 repeats**, not 325 unique passes. Final affected strict Clippy/format passed after owner repairs; initial compile/lint failures remain in dated receipts. | Viewer/comparison source closures were **RELEASED**. These results qualify the preceding stable slice only; later configured routes/UI outcomes are uncompiled. Do not add the earlier Source, Office, host 98 or UI subsets into this total. |
| Native PDF | **Final 2/0** functional cases used Viewer harness `f00316db…` and the real C PDF worker, including mixed-size continuous pages/rapid camera replacement. Root read the receipts, confirmed no remaining child workers and unchanged app. | The earlier `b43d0280…` pre-lint run is a separate checkpoint. Captured-file fixtures do not qualify production native-filesystem capture, full-app input or performance. |
| Engine policy bridge | New targeted gate passed **141 unique tests / 0 failed / 0 ignored**: five configuration integration tests plus 136 Engine lib tests, counted by actual names. Compilation qualified with **0 warnings**; post-test source drift is empty and the app unchanged. The earlier `FsError` → `PreferencesApplyError` compile error is corrected, with the failed attempt retained. Scope includes policy save/ownership, checked retirement, external capture/launch, admission/lifetimes and direct settings boundaries. | Initial strict Clippy 101 exposed five private-preferences `result_large_err` findings, now closed by the separate repair gate below. The 141 passes retain their frozen source identity. Native process, GUI and Explorer composition remain unqualified. Engine contracts retain no Core changes, the exact two agreed GENERAL allowances and unchanged session/desktop/process limits. |
| Engine private error-boxing repair | Separate new gate passed **5 configuration +22 preferences tests / 0 failures**, with tests, strict Clippy and formatting all **exit 0**. Compilation has zero warnings; post-test/quality source guards have no drift and the protected app is unchanged. | Closes the five private error-boxing findings. This is overlapping scoped revalidation, **not 168 distinct Engine tests** and not a rerun of all 141 on the repair. Native/GUI/Explorer acceptance stays open. |
| Current selected UI integration | Independent continuation **31/0/0**: MainUI 28 +Viewer footer 1 +keyboard 2. Combined with the first comparator-options run 7 pass/1 fail, the selected 39 now has **38 passes /1 Options-init-focus failure**. Unchanged CompareUI 7 was retained by identical EXE hashes, not rerun or added. | The exact prior footer regression and two keyboard tests now pass on this checkpoint. Options initialization must still focus an existing error; that failure is open. This is not all 159 or native/app qualification. |
| MainUI/CompareUI unit checkpoints | Earlier 55/0 retains its source. P3 MainUI units initially 20/1; corrected draft exact selector now **1/0**. Other 20 were not rerun. Diagnostic compile qualified 0/zero warnings/errors, 1890 source guards/323 feature sets and unchanged binding/app. | No fresh 21 or 159 passes. Exact Options focus remains **0/1**: OK focused while expected pattern is visible. All overlapping snapshots remain separate. |
| Native comparator and Source provenance | `comparator-d6ce28b8eb` ran all **four actual Windows native cases: 4/0**, unchanged components and remainingOwned[] each. Actual raw parent/argv/cwd, NotStarted cleanup, two-credit/cancel/no-child and non-Unicode Automatic fallback passed. Source DLL 9915 cached reuse/16 fresh artifacts remains separate provenance. | The old settings-timeout failures stay historical. Corrected faithful fixture 16/6/10 is not an implemented production-pool fix and does not supply native ReadAt counts. Native Source acceptance and final product/deployment gates remain open. |
| Work-branch retirement accounting | Architect's exact source identities were independently matched. This checkpoint had **330 payload slots / 8 reusable waiter tokens**. Relative to the older 324 table: comparator store 1 +policy pins 2 +desktop 1 +shell 2 =6 additional payload slots. Comparison has two counter-only credits, desktop six capture slots/four shared launch permits, and process trackers eight shared/sixteen opener slots; none is an extra retirement domain. | Supersedes older present-tense 324/seven statements retained below as historical ledger content. The capacity correction changes documentation only; it does not close exhaustive LIFE-01 owner/first-poll/shutdown acceptance or prepared-Rename engine/UI integration. |
| Common/Engine canonical integration | Corrected staged Engine libtest+configuration compile 0/0 warnings/errors; **51 disjoint passes** =initial 7 +watch/tree 39 +configuration 5, no repeats/failed/ignored. Postguards drift[]/app unchanged. Historical eight E0308 attempt remains. | These passes retain staged Engine source; new coherent graph compilation/UI/runtime/native/performance qualification pending. |
| Prior Viewer 46 scoped checkpoint | Earlier canonical fc4 helper: **46/0** actual, exact former failure 1 pass then other 45 pass. All 9 document+19 image cases including immediate-open errors passed. | Earlier 0 complete/timeout 45 unrun and 45/1 remain historical; counts overlap. No all 159/native/full-app/resident qualification. |
| Office provider navigation contract | Office V3 exact 16 retains its canonical checkpoint; latest post-drain finished-oracle 710bea fixture is canonical at freeze 6610deda. Prior 71 Office+F5 exact 1 remain their earlier source evidence. | Exact new helper unit and five actual native cases pending a legitimate complete NEW C product; no new Office/native success. |
| Old-source production-pool RED | Actual 7e458befac run, original 2 async/16 blocking: 13 resident readiness/four windows; 20,000ms prepared callback not entered/second Compare incomplete. Closing 2 Viewers recovered SAME save (one callback/raw publication coherent) and SAME Compare. Strict cleanup/exclusive readers checked before intentional assert 1159; raw test 23.02s. | Genuine old-source RED, not migration GREEN/performance/unrecoverable deadlock proof. No owned PIDs/monitor errors; 3941 guards unchanged, unconsumed 4917→F5 fixture exception separately recorded. Current resident/opening-palette implementations remain private/unqualified. |
| Coherent source canonical integration | Source 2 native fixture cleanup and UI 8 corrections now canonical. Full 34 Source assertions/workloads preserved; Runtime/Owner 5 shutdown/unwind fenced. UI 8 preserves deadlines/exact assertions, strengthens 6; production Options popup focus ordering+full frame helpers applied. | Main bindings legitimate regeneration required; Viewer/Compare Slint unchanged. Build owns --keep-going source/Cargo lease; no new full graph/runtime/native success. Historical compile 101/UI 22-5 retained. |
| Maximum-state new-product route | Architect selected genuine Cargo-built libtest at NEW complete C product root before freeze, beside real filesystem packages. Root authorized normal deploy-tool default-off validation payload route. | Actual assembly/qualification/child resolver+typed ZIP gate/native execution pending. No Docs deployment or new EXE qualification; setup decision is not migration GREEN. |
| Process-sampler prototype | Root actual functional review 12/12; all 7 processes exited naturally, cleanup passed. | Collector functionality only; no application old/new performance or sampling-overhead qualification. |
| Scoped Perf source review | Closed with no actionable finding; 114 canonical inputs matched reviewed base 07417133. Retained authentic generated Rust 69,289,347B; no qualified old baseline/delta. | Bounded-page palette reshaping/preexisting Compare UI pixel copy remain measurement limits. Later compiler-only overlay separate; no application responsiveness/performance acceptance. |
| Next-container design status | Private H/R v2 freeze 3d6a34ee preserves 8 postimages/baselines and 30 v1 artifacts+manifest. New private 50 exact PASS/0/0 =15 new+35 direct existing; compiled once 39.157s, 8 compiled postimages exact, artifact 9fe3a727 unchanged/guards-clean. Registry 42 pins match; metadata 42/compiled 39 features no expansion; raw 11 whole-row flags preserved as dependency-list pruning only. Root accepted R3.2 Metadata 1 proposal/private implementation. | PRIVATE ONLY; earlier 15 UNRUN handoff remains historical. Metadata 1 adds one resident domain to Engine 13/Viewer 5/Compare 2, proposed 21; transient 2 async/16 blocking unchanged. No canonical full A1/Core/history integration, native/full feature or executed M11 acceptance. |
| A1 shared contract accepted | Root accepted frozen shared contract f319802d…ec06e/API 64bfa555; 153 unique owner paths/current baselines/decision documents/nine read-only guards verified. Engine config obtains vacant ServiceSlot before ownership transfer, no replacement; duplicate/unused-slot tests required. | Design accepted, implementation incomplete. B0 privately dispatched; Backend app-integration 5 rows after B0, V/U private adapter/fixture preparation parallel, attach immutable B0 before compile. No new dependencies/canonical A1 writes. R2/M1-M11/later B-C/native/full 54 remain required; canonical build/new C/native priority. |
| Checked-spool private candidate | Root accepted 7-file candidate privately, review 5bb6d4b3; 31 unit declarations/common 10 Windows 13 Linux 8+3 docs were UNRUN at freeze. Actual partial review 79ff1271: Linux 41 PASS/0 fail+3 docs; Windows 47 PASS/1 setup RED+3 docs, raw logs/guards verified. | Windows RED unqualified/Backend diagnoses raw Win32 path-setup. Per platform overlap, no summed unique total or addition to H/R 50. No canonical code/M7 staging/graph actor integration/full native close acceptance; current app producer priority. |
| Direct UI runtime checkpoint | Retry 4 exactly 23 unique passes /0 failed /0 ignored: immutable 20, separate named caret 1, separate Options Transaction 2. Clean source guards/owned cleanup. Root viewed the new complete initial-focus Options BMP and complete paged/continuous 320x200 footer captures. | Earlier 18/20/UI 22-5 checkpoints overlap and retain their dated source. Selected visual/UI acceptance does not qualify all layouts/platforms, normal app/native runtime or performance; old captures preserved. |
| Retry 4 complete compilation | Actual complete workspace/all-target no-run compilation passed exit 0; raw Cargo ends build-finished success=true, zero errors/warnings, closing drift[]/appUnchanged=true. | Earlier Clippy 101 Explorer issue fixed by canonical 4-file wiring fec5c57f…d303cc/immutable Viewer freeze dd204ddc…d1a. Same strict retry 1 advanced past Explorer, then 101 on 2 CompareUI duplicate_mod diagnostics. CompareUI 4-file cfg test fix now canonical 6bccd5af…d1d7fc; 14 test bodies/helpers/raw Unicode/runtime unchanged, 2,288 guard otherwise clean/fmt 0. New lint/affected harness/direct 7 checks pending. CEF 1-line spelling canonical ad55e66a…322d06 retains locked packages/SDK/licenses/features; warning-free standalone replay pending. No new native/normal EXE qualification; compile/UI 23 dated/full 54 open. |
| Latest strict Clippy checkpoints | Actual terminal 101 on 3 Compare Engine test pixel chunks uses; Root-requested v1 iterator correction 386f10ae then triggered retry 3 manual_contains 101. Root promoted pixel v2 c92cae2a…ac50f3a: same 3 membership checks use contains, same RGBA/remainder/assertion values/runtime/production/dependencies. | Pinned isolated probe before 1/after 0 only. Whole-workspace strict/direct runtime pending, no strict PASS/native/normal EXE qualification. Failed retries retained; future A1 adapters preserve accepted v2 baseline. |
| Open cleanup receipt seams | Viewer reported explicit erased-resource and abandoned-layout cleanup receipt gaps; Architect owns additive seam decisions. | No implemented cleanup guarantee inferred; full LIFE-01 remains open. Checked-spool 7 has partial private platform results/Windows RED, not canonical/full native close acceptance. |
| Preliminary normal producer and later UI test-lint promotion | Normal Explorer/elevation producer actual exit 0/build-finished true/no diagnostics-warnings/2,288 source guard clean. New test=false me.exe 110,218,752B/c487eff2…5d3a9c1; elevation 325,632B/b761e65f…1c62657. Main binding Fresh/hash unchanged. Root then promoted Main/Viewer UI 4 test-lint files 5c166af1 with inverse/helper/fmt guards; new app unchanged during promotion. | Historical f311 intentionally replaced by authorized producer. Current-source strict/direct/final normal invocation pending; no complete C deployment/native/full 54 qualification. |
| Latest strict retry 5 and private lifecycle assignments | Actual strict retry 5 terminal 101/one Office HTTP fixture collapsible_if; Root promoted exact 1-file correction 679bcf78/HTTP fixture 545ff640←e053fc1c, normal c487 unchanged/source guard otherwise clean. Pinned CHECK help 43 supports keep-going; strict replay uses CHECK pass-through, no lint/coverage weakening. | Refresh test-only input epoch before actual Office native replay; old receipts not rebound. No strict/direct/final normal PASS yet. Private DR Core-native/PO extra Viewer reservations dispatched; no canonical lifecycle implementation/full goal closure. |
| Checked-spool Windows fixture v2 | Exact 1 setup correction preserves production/other 6 postimages/29 v1 artifacts. Actual v2 now 48 exact PASS+3 docs; Root f274e348 checked 100 raw log hashes/7 exe/7 postimages, zero errors. | Original v1 Windows RED retained; Linux v1 separate 41+3 unchanged scope, no summed unique total. Perf disposition pending/no canonical promotion/M7/LIFE01 closure. |
| Prior strict retry 6/B0 private scope | Keep-going retry 6 actual 101/3 test lints in 2 files before the final canonical correction and retry 7 PASS below. B0 immutable 16 rows+typed recipe Root review 09c428a3/private compile authorized. Host validate() loses local cancel under Viewer lock; Architect minimal additive hook decision pending. | Source review is not tests/canonical actor feature; 14 inline tests do not prove end-to-end cancellation. Canonical new normal C/native pipeline priority; full 54 open. |
| Full strict Clippy retry 7 | After canonical UI 1+WorkBudget 2 test-only lint batch 99eb030d, actual full workspace/all targets strict PASS exit 0/1001 JSON events/build-finished true/0 warnings-errors/2288 source guards 0 drift/app c487 unchanged. Root review 368be4ef verified actual raw receipt. | Compile/lint only, no broad runtime claim. All 16 direct cases+separate Office unit remain required; six-target producer active then actual MainUI wrapper libtest 2 cases required, only giant bindings crate libtest excluded. No test dropping; final normal 359 graph/new C/native pending. |
| Host notice completion-v3 | Eleven omitted-license sources resolved; 662 stage files/61 override files/0 missing. Root 75f68311 verified exact crate checksum-VCS commit+27 saved official response/copy hashes. Module Root 10be6a5a review 1821 runtime files/1945 inputs/7 historical wrappers clean. | No dependency/project license/license selection change. Final normal 359 graph comparison/new C copy-freeze/native Source Office MAX pending; authored unrun archive actor not implemented/full 54 open. |
| Current exact boundary results RED | Seven-target linked producer 2 exit 0; original runtime 1 first 10 PASS then case 11 Viewer UI palette 300 s TIMEOUT/PID 37548 killed/counts absent. Separate 6 continuation PASS gives 15 direct PASS/1 Viewer TIMEOUT +separate Office 710 unit PASS, overallAccepted=false. Root b66594d0 verifies 51 log-summary/5 capture hashes (not visual)/7 EXE/exact 17 outcomes. | Separate 60 s nocapture diagnostic RED exposes appearance.rs 508 restore_palette.unwrap Err("Viewer is closed."); original 300 s RED retained/no acceptance retry/timeout reduction. Backend stop cause unobserved/closed-stopped-catalog startup under diagnosis. Private UI failure-only mail logging/source review; Architect PreparedCatalog seams. No guard weakening/fake runtime/canonical fix/full closure. |
| Private B0 v1 and composed Viewer v2 | B0 v1 actual 34 PASS=14 new+20 direct/d32d1ff2, one deprecated fetch_update warning retained. Later host delta 9ddee287/Core-recipe 29a13391 composed GREEN v2 freeze 51c32688/inventory 58ff82bd/Root 7e910068 27 sources accepted privately. | Source checkpoint 104 Viewer/11 Host+3 RED declarations were UNRUN; later 3 RED/14 GREEN verify cooperative boundary only, not full 104/PO. UI extra 5 private consumers b72a7ef2 beyond UI 30/original freeze unchanged. PO v2 P1 nonintegratable/no canonical generic resource/native/B1/lifecycle/full goal closure. |
| New normal 673 and mailbox diagnostic v3 | normal-host-validation-input-2 actual producer PASS/new D app 673084bb…bbc95/test=false/359 graph matches notices. Later mailbox-original-graph-v3 same original 7 target compile 0/zero W-E +diagnostic selector PASS 1/0.21s/no mailbox error output. | No runtime acceptance/root cause fix; original closed panic/300 s timeout retained/unexplained. UI approved private ACK-close fixture validity change underway, not proven root cause fix. New C/native/full 54 open. |
| Independent v3 review and PO v2 source P1 | Root 2b311c88 verified 3 raw log pairs/all 2288 sources/Cargo feature-profile variants equivalent/one exact diagnostic PASS 0.21; no acceptance/root cause. Root 77c65185 confirms missing agreed typed preownership admission in private PO v2. | Old v2 immutable/NONINTEGRATABLE; Viewer additive corrected epoch pending/Architect caller boundary review. Token before handoff/serialized seal-admit/late failure must not mutate completed receipt; caller retains already owned refused source. The 3 REDs/14 narrow GREEN below are separate from this PO P1; full goal open. |
| Actual cooperative 3 RED and sealed UI fixture candidate | Root 63867970 independently verified 3 exact behavioral REDs/9 raw log-list-summary hashes/actual EXE 42422763; rescue+both joins before final assertions. RED compile 0/1 historical fetch_update warning/not strict green. Palette helper one file e3bc→1e73 Root 085a review 25 records/19 old tests unchanged/2 new UNRUN/rustfmt 0. | Scoped 14 GREEN subsequently passed below; typed PO validation separate. UI promotion initially held for C assembly, later canonical 9c25 below/runtime pending/no proven original stop cause fix/no whole LIFE-native acceptance. |
| Actual narrow cooperative GREEN 14 | After preserved 3 REDs, actual 14 PASS/0 failed/0 ignored=11 Host+3 previously RED Viewer. Root d5d14402 independently verified 4 raw logs/2 EXE/all 1294 source-proof-absence guards across 3 schemas. Test compile 0/zero W-E. | Private cooperative-cancel boundary only; ordinary test compile NOT full Clippy-D qualification. No canonical promotion/full 104 Viewer/typed PO/LIFE01/native App. C assembly PID 54224 actual normal 673+test cf411/epoch 9297 inputs populating destination/not frozen accepted; UI helper hold persists. |
| Completed C9297 input and new helper source epoch | C validation assembly exit 0+normal-only registration; Root c14c67c8 exact 2487 files/752,608,762B: 6 Viewer/3 FS/2 thumbs/662 notices/normal 673+test cf411; freeze/postregistration hashes identical. After actual Build release Root promoted 1 helper e3bc→1e73/fdf06c25, 2288 guards/fmt 0/normal 673 Main unchanged. | No app/test native launch or acceptance. NativeRoot copied verbatim from actual identities/completion. C bundle stays 9297; new canonical source 9c25, same 7 graph+3 selectors pending 300s. Do not rebind old C/failed Epochs or claim original cause fixed/full 54 closure. |
| Private PO v3 source admission (historical) | Root 2db73d40 verified 33 sealed references/all 27 sources; 5 Engine changes/22 unchanged/canonical baselines exact. Actual token-before capture/caller/neutral Finished paths source read; private Build authorized. | At admission, 3 compatible admission RED+129 GREEN were expectations only. Later actual Viewer 118=116 PASS/2 FAIL and Host 11 prior separate are recorded below; no canonical integration. Old v2 immutable/nonintegratable. Actual UI collector/App/DR/native LIFE remain pending; full 54 open. |
| Fixture 1e73 actual partial run/source flaw | Original palette 1 PASS; new normal teardown 300 s TIMEOUT/20 UNRUN/owned cleanup clean/current EXE 5b3. Root 9642bcbe verifies 6 raw pairs and pinned Slint 1.18.1 api 503/1086: show retains component until hide; new test never hides, Rust UI handle drops insufficient precondition. | Source flaw PROVEN/sole wait cause NOT proven. Private one file hide+weak UI disappearance checks authorized/preserve actual Session live before Runtime Drop; old packet/run retained/no acceptance. Engine PO/UI collector independent; full goal not blocked. |
| Helper v2 affected boundary accepted | Canonical helper 012aff/source e0df; original seven-target compile exit 0, zero warnings/errors, six Fresh/only ViewerUI rebuilt 934254c5. Actual 22/0/0: original palette +2 teardown +19 existing, separately exact 1 under 300s. Root 637eb22f verifies 66 raw pairs/44 invocations/2,288 clean guards/seven executable identities; parent checks 77 pairs/all 45 owned PIDs absent. | Affected fixture only. Earlier v1 timeout and original closed panic preserved; original stop cause unexplained. C9297 stays at its frozen epoch. No whole-product/native/full 54 acceptance. |
| Genuine native MAX corrected pass | Actual frozen-C cf411 payload: 1 PASS/0/0 in 7.95s; ZIP available, four Viewer/two Compare frames, settings/navigation progress 7ms, pools joined and native/viewer/archive/report readers plus controlled clipboard cleaned. Root a9a6a4fb verified 24 raw pairs/2,288 source hashes/all 2,487 C files identities-size-mtime-hash-exact set. | Original setup failure was missing native-work before fixture canonicalize; new immutable adapter adds only MAX mkdir, no app/C changes. No all 20 simultaneously busy/image spare decoder Ready proof or old-source genuine pressure RED claim. Source/HTML/XLSX failures remain; no whole-product acceptance. |
| Native Source C++ actual failure | Actual 0/1/0, exit 101, libtest 61.56s; assertion at source_native_acceptance_tests.rs 742: required Source frame absent before shared absolute 45s deadline. Initial/closing 4,807-entry guards clean, owned cleanup true/remaining[], private metadata unchanged. Metadata records expected automatic Source mode 2 route, no first-destination report. | Missing phase unknown; no Source paint acceptance. Outer 300s watchdog did not fire. DIAG 1 one-test-file metadata promotion 410e/source e39e preserves all 34 assertions/45s, format 0, production unchanged; matching seven-target compile 0/zero diagnostics and reuse 501a7 approved; later DIAG 1 actual 0/1/0 exit 101/61.68s, control ACK step 4/45s/C++ trial 0 target .82; expected Positions gen 1 seq[3,0] vs last gen 2 seq[0,0]/four frames. Position context is not Worker-open generation/file-reopen proof; cause unknown. Root 3a853e4c verifies 41 pairs/4835+2288+2498 clean guards/owned cleanup/Cpp metadata unchanged. C9297 epoch unchanged. HTML/XLSX/later Office navigation failures retained; Office total 2 PASS/3 FAIL/0 UNRUN. Source mapped-prefix actual 3 RED->18 GREEN private, canonical production unchanged; new retained-v2 exact 1 PASS; 25af/d394 now canonical, current canonical 24/new normal/new C/native 34 validation pending. |
| Source private behavioral and retained proof | Root 83a3 verifies actual 3 behavioral RED->18 GREEN, 119 original/copy pairs/42 invocations/both preserved EXEs/feature-profile variants/all 1,099 private sources; only production 07f->25af/tests identical. False automatic classifier flag preserved; raw Reload panic genuine before Completed, GREEN reaches completion/pixels. Root 861e reviews five prior 1e5 controls plus new 7ca exact retained Source pixel/gutter test 1 PASS 0.09s, 59 pairs/all 1,099 v2 sources. | Prior 18+5 carried with explicit closure, not rerun. This qualifies authored corrected-frame/crop unit behavior, without live Session/actual provider retained-line allowance/installed rapid Drag or native acceptance. Canonical promotion and new validation below are separate. |
| Source production canonical promotion | Root cb8821b9 promotes exactly worker_position.rs 07f->25af and tests 2e8d->d394, 2,288 clean guards/fmt 0/current full source c500b7e0. Previous normal 673/Main unchanged at promotion; C9297 never rebound. | Build assigned matching new normal/current notice graph/new complete C installation, current canonical 24 targeted integration then original Source 34 real corpus/WAIT 45/300s. All pending; prior private passes and old normal/C do not qualify changed production. No native acceptance/publishing/full 54 closure. |
| Private PO v3 failure and v4 actual passes | V3 compile 0/no compiler warnings; 118 actual Viewer cases=116 PASS/2 FAIL/0 ignore, failures preserved. Root c57676ba accepted private v4: 28 postimages/41 refs/three-file delta/25 v3 rows unchanged/original real-session prefix preserved. V4 now 128 distinct PASS/0/0=focused 12+disjoint 116; Root 6dc45cea checks actual raw names/counts/three EXEs/1,387 reconstructed guards. | Host 11 prior evidence separate/not replayed. Ordinary warning-free compile is not full strict Clippy. Opaque final Generated wrapper retained/unconfirmed; no full cleanup guarantee, Core API change or canonical promotion. |
| Private Backend 109 actual compiler failure | Baseline 109 compile actual exit 101/614.234s/14 errors+4 warning messages, archived unchanged. Root 4322124b reserves exactly five extra private Backend consumer rows for actual missing-field/old-API fixture corrections. | All 74 runtime cases UNRUN; compiler failure is not behavioral RED or normal strict qualification. Preserve baseline; fresh coherent composition/compile required before follow-on 13 new+59 direct=72 unique runtime. |
| Private collector v2 actual 11 passes and remaining design work | Root c19577ea independently accepts actual private Engine v4/UI collector v2 boundary: 11 exact PASS/0/0, each 300s; 33 raw original/copy pairs, 22 invocations, actual Cargo/UI EXE a60aa5ae, 1,099 private sources and 3,894+6 runtime guard composition checked. Original compile/raw drift preserved; explicit unconsumed 410e cfg test exception separate. | Private boundary only; no canonical/full LIFE/original production failure cause acceptance. Cross-graph registered-idle bridge accepted for existing Backend 3 private files over v3. Future Generated closed checked-spool amendment source-only/unreserved; history+idle follow-on Root d0ff source-reviewed/13 new UNRUN. Preserve 109/74 baseline; additive 109+5 compile and 13 new+59 direct=72 unique remain required. No measured Perf gain/final cleanup acceptance. |
| Additional cleanup — 18 November | Sealed additional receipt: C 412 files/3,255,585,645 logical B; D 642 files/7,250,716,439 logical B/0 skips, protected EXEs unchanged/164 default symbols+QA retention preserved. | Separate from earlier cleanup; logical sizes not net physical reclaim. No process kill/ACL change/feature qualification. |
| Follow-up cleanup — C then D | C 132,982 old staging-cache-log files/3,263,580,433 logical B deleted, 90 skips=88 in use+1 changed+1 already gone. D 318 explicit owner released intermediate-PDB files/660,271,043 logical B/0 skips; 155 RED entries identity-size-mtime rechecked. Installed rustc+3 normal EXE SHA unchanged; 18 audit files ~166 MB verified on D/task temporary C audit only removed. | Prior 642 not recounted; logical sizes not physical reclaim. No ACL/process kill/system memory/source-user docs-settings-chats-active build change. Cleanup only, no feature/test acceptance. |
| Current normal module staging | Handoff all 9 modules+2 thumbnailers compiled/staged; newer Root review completed 1,821 module hashes/1,945 input checks/zero errors. | Root notices/normal host/complete C assembly/native acceptance separate; no full goal closure. |
| Renewed cleanup — 18 November | C 373 files/3,208,506,127 logical B/observed free+2,828,046,336B. D exact 6 obsolete FQA roots/1,795 files/3,558,657,330 logical B; current C-D app/Main hashes unchanged. | D was already compressed (~483 MB prior allocation), concurrent builds mean no 3.56 GB physical reclaim claim. Separate from earlier cleanup totals; no feature/native qualification. |
| Native harness/driver work | Source 2 owner/unwind cleanup canonical; current native Source frame-deadline failure preserved. Office finished-after-drain unit prior PASS/headless 99 retains same-artifact scope. Genuine frozen-C MAX actual 1 PASS/7.95s after wrapper-only mkdir, separate from original setup failure. | Native Office all 5 executed: 2 PASS/3 FAIL/0 UNRUN. HTML roundtrip timeout, XLSX Find and later-navigation failures retained; parent-change-before-data 1 PASS 5.88s and close/cancel 1 PASS 3.73s, clean guards/owned closures. Source DIAG 1 promoted, compile 0/zero diagnostics/reuse approved, native DIAG 1 failed 0/1/0 in 61.68s; position-context metadata narrows phase, cause unknown. No complete native product or production leak claim. |
| Office phase observer canonical test source | After final Build release, Root b2a6 promotes exactly office_preview_native_tests.rs 710bea->50eb/96,406B; 2,288 clean pre/post guards/fmt 0/normal 673+Main 3 unchanged, production unchanged. Current full source 2ae892; five prior 0bb EXE/depfile/producer/Office 710 copies preserved/readback verified. | Independent original 7 Office phase producer actual compile 0/zero diagnostics/new Engine 3666/normal 673/source 2ae; both metadata observations FAILED initial wait-loaded/0 events-requests-frames: HTML 20.29s/XLSX 20.38s, clean 4809+2288+2498 guards/owned cleanup; startup/qualified-worker cause unknown, UI tracing; distinct from original navigation failures. Explicit unconsumed canonical 50eb exception may be bridged while private 710/raw guards remain immutable. No extra Root approval, no unchanged unit/prior 2 PASS replay; 20s/300s/oracles/policy preserved. C9297 not rebound; all 5 Office outcomes 2 PASS/3 FAIL/0 UNRUN remain. |
| Validation payload tooling | Root accepted exact 3 tools and frozen 22 payload tests; prior 13 unchanged direct boundary passes retain scope. | Older combined 34 overlaps; do not add again. Tooling ready is not assembly/deployment/native MAX success. |
| Scoped retry 2 headless checkpoint | Actual 99 distinct PASS/0 fail/0 ignore: Compare 17+Viewer 26+Host 47+checksums 4+5. Finished-after-drain unit PASS; 1400 guards/owned cleanup clean. | Host 47 overlaps prior 71/epoch results; no cross-epoch sum or synthetic full graph success. Actual native 5/new product remain pending. |
| Source-size guard after promotions | Root actual PASS: 2542 source files <=100000B. | Source policy check only; not compiler/runtime/native/all 54 qualification. |
| Disk cleanup — 18 November | COMPLETE. C 4 unused runtime backups+obsolete uv indexes: 677,447,900 logical B/observed 390,033,408B. D 1390 obsolete rlib/rmeta files from 7 old targets: 2,500,434,725 logical B/observed 2,427,236,352B. | Initial D preflight interrupted before deletion/all 1565 present before HTML exclusion. HTML/default target/qa/verification/EXE/DLL/native SDK/screenshots/current C runtime preserved; 3 protected app Source hashes unchanged. No cleanup holds/source or ACL changes. No feature/qualification completion. |
| Disk cleanup — 17 November (historical) | C 10 obsolete QA installs: 1,826,040,781 logical B/observed free delta 1,168,015,360B. D obsolete caches 659,051,203 logical B/observed delta 577,359,872B; historical captures NTFS allocation saving 3,078,541,312B with bytes/paths/mtime retained. Runtime/D EXE preserved. | Concurrent writes prevent exclusive volume-delta attribution. Later Root check C≈21.3GB/D≈27.9GB free; C pagefile 23,426,166,784B untouched. No new EXE qualification, Git integration, Office success or 54-item completion. |

Evidence: `_build/verification/viewer-refresh-20261117/` contains
`viewer/stage-a-handoff.md`, `viewer/stage-a-green.log`,
`backend/qualification-packet.json`,
`backend/archive-probe/perf-pair-20261117T110013Z/{report.md,comparison.json}`,
`ui/handoff.md`, `ui/green-results.json`,
`ui/lease-state.json`, `viewer/host-engine/gate/test-results.json`,
`backend/pdf-migration/lease-state.json`, `build/publication-handoff.md` and
`architect/office-preview-phase0.md`,
`build/integration-gate/{five-lib-headless-proof.json,postlint-proof.json,viewer-compare-completion.json}`,
`build/integration-gate/native-pdf/{results.json,completion.json}`,
`build/integration-gate/office-performance/runtime-attempt1/results.json`,
`build/integration-gate/office-performance/{completion.json,results.json,production-release.json,quality.json}`,
`backend/office-preview/performance/runtime-review-fixes-ready.json` and
`build/engine-gate/{compile-summary.json,guard-after-tests.json}` and the two
`build/engine-gate/headless/*/test.log` files,
`build/engine-gate/{quality.json,clippy.jsonl}` and
`build/compare-routes-gate/{headless-counts.json,quality.json,guard-after-tests.json}`
with its raw headless test log,
`build/office-six-format-gate/{compile-summary.json,headless-counts.json,quality.json,guard-after-tests.json}`
and `build/office-six-format-gate/native-cef-5d4cf7b040/{results.json,0/test.log}`.
Later receipts: `build/engine-repair-gate/{completion.json,quality.json}` and its
two `review-evidence/*-test.log` files,
`build/ui-explorer-batch/integration/{compile-summary.json,guard-after-compile.json}`
and `build/office-six-format-gate/html-source-release.json`.
Latest checked receipts: `build/office-repair-gate/headless-completion.json`,
`build/ui-runtime-worker/independent-integration-1/{result.json,owner-fix-required.json}`,
`build/native-worker/office-bfefc8ba02/{handoff.json,resource-diagnostics.json,0/test.log}`
and `architect/engine-lifetime-capacity-correction.md` with its source identities.
Newer checked receipts: `build/office-repair-gate/favicon-lint-1/{quality.json,strict-quality-verified.json,headless/summary.json}`
and `build/native-worker/office-00d13f4a3c/{results.json,handoff.json,0/test.log,1/test.log}`.
Later checked receipts: `build/ui-runtime-worker/main-compare-units-prep/review-evidence/{execution-receipt.json,copy-index.json,verification-index.json}`,
`backend/office-web-host/current-source-freeze.json`,
`viewer/office-source-revocation/canonical-three-file-freeze.json`,
`build/explorer-diagnostic-gate/{compile-summary.json,guard-after-compile.json}`,
`build/native-worker/source-candidate-preflight/normal-producer-1/qualification.json`,
`root-visual-review/format-conversion-receipt.json`,
`build/native-worker/office-0620fdaef3/{results.json,handoff.json}` and
`build/native-worker/comparator-47756c6cba/{results.json,0/test.log}`.
The later revocation gate is `build/office-host-revocation-gate/{quality.json,guard-after-tests.json,headless/summary.json}`;
its preserved Host source is `backend/office-web-host/retained-dom-before-test-lint-source-freeze.json`.
Root owns
`coordination.md`; the Office decision above supersedes its earlier pending-choice
entry. Do not change the user ZIP/source fixtures or the protected running app.

Current checked deltas: `build/ui-runtime-worker/p3-integration-continuation-1/execution-receipt.json`,
`build/ui-runtime-worker/p3-integration-1/owner-fix-required.json`,
`build/ui-p3-main-units-gate/owner-fix-required.json`,
`build/native-worker/comparator-d6ce28b8eb/completion.json`,
`build/native-worker/office-9fe36f20a6/handoff.json`,
`build/office-host-revocation-gate/lint-1/completion.json`,
`architect/production-design-and-appearance-handoff.json` and
`_build/verification/disk-cleanup-20261117/{c-obsolete-qa-cleanup.json,root-postcheck.json}`.

Latest diagnostics: `build/ui-runtime-worker/p3-integration-continuation-1/root-review.json`,
`build/ui-failure-diagnostic-gate/compile-summary.json`,
`build/ui-runtime-worker/failure-diagnostic-1/{execution-receipt.json,focus-handoff.json}`
and `architect/office-provider-navigation-contract-decision.md`. Viewer teardown and
production repro findings are source review, not executed product root-cause proof.

Setup-only production repro and canonical fixture evidence:
`build/native-worker/production-pool-27587e6553/{handoff.json,test.log,post-guards.json}`
and `ui/viewer46-fixture-fix/canonical-promotion.json`. The separately corrected
[LIFE-01 backlog introduction](IMPLEMENTATION_BACKLOG.md#lifecycle-follow-up-found-on-12-november)
records the work-branch 330/8 checkpoint and dates the older 324/7 and 308 accounting; the
historical ledger below is preserved verbatim.

Later scoped receipts: `build/ui-runtime-worker/viewer-fixture-repair-1/execution-receipt.json`,
`ui/viewer-shared-fixture-fix/canonical-promotion.json`,
`build/resident-common-private-gate/{completion.json,snapshot.json,type-overlay-snapshot.json}`,
`viewer/qualified-catalogs-old-scheduler-canonical-freeze.json` and
`backend/blocking-pool-production/catalog-update/canonical-application.json`.
The catalog consumer promotion is newer than its Root source release; actual
adoption/compilation and controlled-open replay remain unrun.

Cleanup quantities are from `_build/verification/disk-cleanup-20261117/c-obsolete-qa-cleanup.json`
and `build/cleanup-preflight/{deleted-caches-receipt.json,capture-compression-receipt.json,final-receipt.json}`.
Later free-space/pagefile observations are Root-reported, not new cleanup or feature evidence.

Current Viewer 46 all-pass receipt: `build/ui-runtime-worker/catalog-helper-viewer-1/execution-receipt.json`
with two raw logs 1+45. Catalog gate 6 has actual 5 pass/1 fixture-precondition fail;
test-only raw-OsString repair F5 is canonical, replay pending. The later 7e458befac pressure run now establishes old-source production RED;
this does not qualify the migration. Opening-palette getter amendment is in
`architect/opening-palette-revision-contract-decision.md`, private implementation
in progress. The scoped [dependency handoff](THIRD_PARTY.md#private-resident-worker-dependency-handoff--17-november-2026)
records optional work-budget/Tokio 1.54.1 MIT and consumer feature edges; Build owns
the coherent lockfile edge/captures after source release. No migration PASS follows.

New actual old-source RED: `root-production-pool-red-review.json` and
`build/native-worker/production-pool-7e458befac/{handoff.json,test.log,post-guards.json,excluded-unconsumed-test-after.json}`.
Viewer combined-v2 opening getter/F5 tests and the UI 41-file freeze are private
source-review checkpoints; Root's 3-delta/92-file and 41-file checks add no executed
migration passes. Full 54-item acceptance remains active.

Newest bounded evidence: `build/html-navigation-private-gate/handoff.json`,
`build/resident-common-round4-private-gate/completion.json`,
`build/private-ui-codegen/final-result-v2.json`,
`viewer/office-provider-denial/office-v3-canonical-freeze.json` and
`architect/engine-resident-exit-observation-contract-decision.md`.
Pure navigation 34/0 and common round 4 29/0 are distinct source scopes; default 13
were filtered in round 4. Slint generation 3/3/194 guards 0 leaves generated Rust
uncompiled. Office V3 exact 16 canonical/132 nonowned guards 0 is source promotion;
native 5 require a legitimate complete new C product. No full 54/Git/new EXE/deployment
completion follows. Engine 200ms follows actual ticket drain inside existing 2500ms.

18 November integration receipts: `backend/resident-workers/common-engine-canonical-stage/canonical-source-ready.json`,
`build/engine-resident-stage-gate/{failure-handoff.json,compile-summary.json}`,
`backend/resident-workers/engine/watch-tree-fixture-canonical-application/canonical-source-ready.json`,
`ui/appearance-bridge-implementation/review-checkpoint-v3/{source-manifest.json,constructor-consumers-completion.json}`,
`viewer/office-provider-denial/finished-oracle-v1-canonical-freeze.json`,
`build/office-v3-catalog-coherent-gate/completion.json` and
`architect/maximum-state-archive-discovery-decision.md`.
Corrected Engine compile, new oracle helper and native 5 remain pending; source
approval/canonical application is not runtime success. All 54 IDs remain active.

Later 18 November receipts: `build/engine-resident-stage-fixture-fix-gate/{compile-summary.json,initial-seven-result.json,guard-after-initial-seven.json}`,
`viewer/resident-appearance/rebased-stage-v3-files.json`,
`root-coherent-integration-rebased-viewer-review.json` and `root-process-sampler-review.json`.
Corrected Engine compilation/two artifacts and initial 7/0/0 are actual. Full Engine/
Viewer/UI/native remain unqualified; source integration 112 writes+2 guards is only
authorized after the consumer boundary. Sampler 12/12+7 natural exits is not app performance.

Latest applied coherent source: `root-coherent-source-integration/canonical-source-ready.json`
SHA 07417133be995b56d020525765eb37c45970c7378c9689b70933816175c2802a.
All 112 writes+2 guards are canonical; 114 postimages/7 protected hashes verified.
The staged Engine gate's additional 44 (watch/tree 39+configuration 5) are in
`build/engine-resident-stage-fixture-fix-gate/{watch-boundaries-result.json,guard-after-watch-boundaries.json}`;
initial 7+44 are 51 disjoint passes, not a coherent-graph runtime result. Coherent
lock generation/compilation/UI/native qualification and the original 54 remain open.

Latest coherent compile: `build/coherent-final-gate/{compile-result.json,compile-summary.json,compile.jsonl,guard-after-compile.json}`.
Overall exit 101/E0639 and one dead-code warning are retained; five fresh normal
library artifacts advance generated-only status for those libraries. The full
graph and coherent runtime/native remain unqualified; app F311 is unchanged.
Two-file UI correction is in preparation with no Slint/assertion change claimed.

Latest compiler-only source overlay: `root-coherent-source-integration/ui-compile-fix-v1/canonical-source-ready.json`
SHA e0739b3c1acfb4f63607b3f2c1bf4ca7dc4b144a57caf0f59711df4214be9494;
two files compose with 07417133, not an additional application qualification.
Perf receipt `perf/coherent-render-review-v1/review-receipt.json` closes source
review only, with 114 base-checkpoint matches/69,289,347 retained source bytes and
no measured old/new delta. Warm compile/runtime/native remain pending; original 54 open.

Latest consumer overlay: `root-coherent-source-integration/consumer-fix-v2/canonical-source-ready.json`
SHA 91150f0e9549694e0a781ed6e886aa8c21d8af8c191f071516465dfd71a94c25;
seven additional paths compose 121 with original 114/UI 2 override. Interim missing
copy_nested_retirement compile failure is preserved. Retry 2's observed fresh
four-example/two-harness artifacts are partial only; the captured log has no
errors/warnings/build-finished record or terminal result. Full graph/runtime/full 54
acceptance remains open. Next-container fixed 2/1GiB limits are returned proposal
items, not A1 authorization or completed implementation.

Measured 18 November cleanup: `_build/verification/disk-cleanup-20261118/{final-result.json,c-result.json,d-result.json,d-preflight.json,d-candidates.json,d-candidates-without-html.json}`.
C logical 677,447,900B/observed 390,033,408B; D 1390 files/7 targets/logical 2,500,434,725B/
observed 2,427,236,352B. HTML fully preserved; interrupted preflight deleted nothing,
all 1565 original candidates confirmed before exclusion. Three app/Source hashes unchanged;
no holds, source/ACL changes or broader implementation/qualification closure.

Latest runtime/fixture/tool/proposal records: `root-ui-runtime-overlap-review.json`,
`root-coherent-source-integration/ui-literal-fixture-v1/canonical-source-ready.json`,
`build/coherent-final-retry-2/{compile-result.json,compile-summary.json,guard-after-compile.json}`,
`root-validation-payload-tool-review.json` and `architect/tree-container-next/handoff.json`.
Retry 2 terminal 101; scoped UI 22/5 and clean guards are actual. Literal 7713 replay,
capture-helper/source-native fixtures, fresh 5 case Office driver+unignored unit/new C
qualification remain pending. Tool 22+prior 13/overlapping 34 and R2 source 164/0 drift
do not close native/performance/A1/full 54 acceptance.

Newest scoped evidence: `root-headless-retry2-review.json` actual 99 distinct PASS
(Compare 17/Viewer 26/Host 47/checksums 4+5), finished-after-drain unit passed,
1400 guards/owned cleanup clean; Host 47/older 71 overlap is not additive.
`root-coherent-source-integration/{source-native-retirement-v1,ui-scoped-corrections-v1}/canonical-source-ready.json`
records actual Source 2/UI 8 promotion, not passing replays. Full 34 Source assertions,
workloads/deadlines retained; UI strengthens 6 assertions. Main bindings regeneration
is legitimate, Viewer/Compare Slint unchanged. Build --keep-going lease active;
source-size actual PASS 2542 files<=100000B is separate. Private R2 H/R staging seam
only; full A1/history/four Perf proposal obligations remain open, as does original 54.

Latest retry 4 evidence supersedes the pending compile/Options status above:
`root-ui-retry4-runtime-final-addendum.json` verifies actual full workspace/all-target
no-run compile success, exit 0 and clean source/app guards. Exactly 23 unique targeted
UI cases passed /0 failed /0 ignored: immutable 20 plus separate caret 1 plus separate
Options transactions 2, with clean guards and owned cleanup. Earlier 18/20 receipts
overlap and are not added again. Root viewed the complete new initial-focus BMP:
focused error field visible inside the scroll body, readable Options/Comparators
layout. `root-render-review/retry4/conversion.json` binds the complete 320x200 footers;
old partial captures remain historical. Strict Clippy subsequently exited 101 on
one clippy::duplicate_mod finding: Explorer's resident_test_runtime helper loaded
three times. Root subsequently promoted the exact 4-file cfg(test) single-definition/
import correction at `explorer-test-module-lint-v1` (`fec5c57f…d303cc`). Root's
inverse byte guard proves original test bodies/helper/runtime behavior unchanged;
the complete 2,288-source guard is clean outside the intended 4, helper/app unchanged.
Viewer's immutable freeze dd204ddc…d1a matches the canonical receipt. Same-command
strict Clippy retry 1 advanced past Explorer, then terminal 101 on 2 CompareUI libtest
duplicate_mod helper declarations (raw platform paths and tests/support), with
2,288-source drift[]/appUnchanged=true. Root subsequently promoted the reviewed 4-file
CompareUI shared cfg test definition/import correction (`6bccd5af…d1d7fc`). All 14 test
bodies/helper bytes/raw Unicode fixtures/runtime limits remain unchanged by inverse/
body guards; full 2,288-source guard otherwise clean and Root 4-file rustfmt 0.
New strict Clippy, affected harness compilation and direct 7 checks remain pending
with Build. Source/format checks are not new runtime/lint qualification. Root separately
promoted the approved 1-line HTML CEF requirement-spelling correction at the terminal
boundary (`ad55e66a…322d06`): metadata suffix removed, same locked cef/sys
156.4.0+156.0.26/checksums/selected MIT/native SDK/sandbox features. Dependency resolution,
inventory and notices are unchanged; no inventory/notice update is required for
this spelling-only correction. Warning-free standalone Build replay remains pending;
historical Cargo warnings are retained, not yet demonstrated removed.
Build owns further qualification; strict lint/corrected Explorer harness/native/
new normal EXE stay open. Earlier compile/UI 23 retain their pre-fix source; no new
pass is claimed. No normal
application/native runtime acceptance follows from no-run compilation or UI 23.
Private H/R `backend/decoded-work-r2-20261118/fixture-path-v2/source-ready-v2.json`
SHA 3d6a34ee binds 8 postimages/baselines/exact 2 fixture-root corrections and preserved
30 v1 artifacts+manifest; 15 tests UNRUN at that historical handoff. New private runtime
review `root-decoded-work-private-v2-runtime-review.json` (`4219ebdc…98ec2`) records
one 39.157s compile and 50 unique exact PASS/0/0: 15 new+35 direct existing. Root checked all 50
raw stdout/stderr hashes/outcomes, exact 8 compiled v2 postimages, unchanged 9fe3a727 artifact
and clean cleanup/source/Root guards. Required 42 registry pins match; metadata 42/
compiled 39 features show no expansion. Raw 11 whole-lock-row flags remain intact and
are explained by pruned dependency lists only, not repins. PRIVATE ONLY: no canonical
A1/full feature/native acceptance or summing with unrelated earlier passes.
Root separately accepted the 7-file checked-spool/native Root candidate for PRIVATE
qualification only (`root-checked-spool-r3-private-review.json`, 5bb6d4b3), after
source/native ownership review/exact pre-post-legacy diff guards. Its 31 authored units
(common 10/Windows 13/Linux 8) plus 3 compile fail doctests were UNRUN at source freeze.
New actual partial review 79ff1271 records Linux 41 PASS/0 fail+3 docs and
Windows 47 PASS/1 setup RED+3 docs; Root verified raw logs/source pins feature guards. Windows
failed construction remains unqualified; Backend diagnoses raw Win32 path/setup only.
Do not sum overlapping platform cases as unique or add to H/R 50. Current app producer
retains priority; no canonical code/M7 staging/graph actor integration/full native close
acceptance follows.
R3.2 explicitly declares one app-wide Archive Metadata 1 domain at Explorer bootstrap,
outer guard ownership and ordered drain/join after native retirement; existing
Engine 13/Viewer 5/Compare 2 and transient 2 async/16 blocking remain unchanged, proposed
resident total 21. Root accepted the contract at proposal level and authorized only
isolated private checked-spool/native-root implementation. Production implementation,
M11 execution and canonical A1/Core/history integration are not qualified.
Root subsequently accepted the completed shared A1 contract, not implementation:
`architect/tree-container-next/a1-freeze-v1.json` SHA
f319802d9ecf7ddc0fafce62650dd9a07b41092df952d6602678e7fe834ec06e,
shared API 64bfa555. Root review `root-a1-shared-contract-v1-review.json` verifies 153
unique owner paths/current baselines/decision documents/nine readonly guards. The corrected
Engine interface uses existing vacant ServiceSlot before ownership transfer, never
replacement; duplicate/unused-slot tests remain required. Backend B0 privately
dispatched, app-integrator 5 rows assigned Backend after B0; Viewer V/UI U prepare private
adapters/fixtures in parallel and attach immutable B0 before compilation. No new
dependency or canonical A1 write is authorized. Full R2/M1-M11 and later B/C remain
required; canonical build/new complete C/native retain priority, full 54 incomplete.
The accepted baseline now also includes Root's 2-file/3-expression Compare Engine
pixel iteration correction 386f10ae…1e951: same RGBA/ignored remainder, original
assertions/runtime/production/dependencies unchanged, source guard otherwise clean/
app f311 unchanged. Strict replay is pending, not PASS. Viewer identified a concrete
frozen ArchiveViewer/source_identity declaration omission; Architect/Backend are
preparing an additive B0 clarification. Original frozen artifacts remain immutable;
no locally invented source IDs or adapter compilation success is authorized/implied.
Root-requested pixel v1 iterator shape then triggered strict retry 3 manual_contains 101,
retained as failed history. Root promoted pixel v2 contains correction c92cae2a…ac50f3a:
same RGBA/remainder/assertion values/runtime/production/dependencies. Build's pinned
isolated probe passed (before 1/after 0), not whole workspace/direct runtime acceptance;
strict replay remains pending. Checked-spool 7 was authorized/UNRUN at that source
checkpoint; later partial platform results retain Windows RED/no promotion. Viewer reported erased-resource/abandoned-layout cleanup
receipt gaps; Architect owns additive seam decisions and full LIFE-01 remains open.
Latest normal milestone: preliminary Explorer/elevation producer exit 0 with
build-finished true/no diagnostics-warnings/2,288 source guard clean; actual Main binding
Fresh with unchanged artifact hashes. Normal test=false me.exe 110,218,752B SHA
c487eff2c29469262f0a9ed3c978d162af8ae842dc5f6e02d9c8663bb5d3a9c1;
elevation 325,632B SHA b761e65ff3a70a88c2bb7570711f8d5343f27613701e856ef0fc9037d1c62657.
Earlier f311 is historical, intentionally replaced by the authorized producer.
After released producer boundary Root promoted Main/Viewer UI 4-file test-lint batch
5c166af1 with inverse/helper/fmt guards, otherwise clean 2,288 source guard and new app
unchanged during promotion. Current-source strict/direct/final normal invocation
pending; no complete C deployment/native/full 54 qualification follows.
Strict retry 5 subsequently 101 on one Office HTTP fixture collapsible_if; Viewer assigned
exact short circuit-equivalent correction, no PASS claimed. Pinned cargo check help 43
advertises --keep-going; Root authorized Clippy CHECK pass-through next replay to collect
independent failures, not an unsupported cargo TEST flag or actual successful result.
Private DR Core/native and PO extra Viewer reservations dispatched to exact contracts;
no canonical lifecycle implementation/full goal closure. New normal c487 preliminary
remains actual test=false/source guard clean/Main binding Fresh, no final C bundle/native.
Root subsequently promoted Office HTTP fixture 1-file short circuit correction 679bcf78,
545ff640←e053fc1c, complete source guard otherwise clean/normal c487 unchanged. Refresh
test-only input epoch for actual Office native replay; old receipts not rebound.
Strict replay uses supported CHECK keep-going/no lint coverage weakening, no PASS claimed.
Root also approved checked-spool Windows fixture v2 private replay 908d8f8b: one setup
expression corrected, production/other 6 files/29 v1 artifacts unchanged. V2 UNRUN/no
canonical checked spool promotion; original Windows RED retained. OS 123 failure proven,
mixed separator-verbatim cause remains inference pending exact Win32 probe; earlier
Linux 41+3 keeps its unchanged consumed-source epoch.
Latest Windows v2 actual 48 exact PASS+3 docs is now independently Root-reviewed f274e348:
100 raw log hashes/7 exe/7 composed postimages, no errors. V1 Windows failure is retained,
Linux v1 separate 41+3 unchanged scope; Perf disposition pending/no M7/LIFE01 closure.
B0 immutable 16 rows+typed recipe Root source/hash review 09c428a3/private compile authorized,
not canonical actor feature. Host validate() breaks local cancel on Viewer lock; Architect
minimal additive hook decision pending, not end-to-end cancel qualification.
B0's immutable ArchiveViewer declaration/constructor and typed fixture clarification
now addresses the earlier missing declaration, with original A1 freeze/API unchanged;
this does not close the separate Host validate/local cancel hook gap or prove runtime.
Keep-going strict retry 6 collected 3 test lints/2 files, actual 101/owners fixing/no full PASS.
Module handoff all 9+2 compiled staged; newer Root review completed 1,821 module hashes and
1,945 input checks/zero errors, no C assembly/native acceptance.
Renewed cleanup receipt `_build/verification/disk-cleanup-20261118-current/final-result.json`:
C 373 files/3,208,506,127 logical B/observed free+2,828,046,336B; D exact 6 obsolete FQA roots/
1,795 files/3,558,657,330 logical B. D prior compressed allocation ~483MB/concurrent builds:
do not claim 3.56 GB physical reclaim. Current C-D app/Main hashes unchanged; earlier cleanup
totals remain separate. Source/PDF/Office/new normal C assembly/native MAX/full 54 open.
Root promoted final UI 1+WorkBudget 2 test lint batch 99eb030d; actual whole workspace/all target
strict Clippy retry 7 PASS exit 0, 1001 JSON events/build-finished true/0 warnings-errors,
2,288 source guards/drift 0/app c487 unchanged. Independent Root report 368be4ef...38dd
binds actual raw JSONL; earlier failed retries retained/no broad runtime PASS.
Six-target producer actually running at Root checkpoint; actual MainUI wrapper libtest
then allowed/required 2 cases, only bindings crate giant libtest excluded. Corrected scope
retains all 16 direct cases+separate Office unit; no test dropping. Authored unrun archive actor
is not implemented by a lint pass. Final normal 359 graph/new C freeze/native Source Office MAX pending.
Host notice completion-v3: 11 omitted license packages resolved/662 files/61 override files/
0 missing; Root 75f68311 exact crate checksum-VCS commit+27 saved official response-copy hash
review complete. No dependency/project license/license selection change. Current 9 module
plus 2 thumbnail staging Root 10be6a5a 1821 runtime files/1945 inputs/7 historical wrappers clean,
not C distribution/native qualification. Full 54 stays open.
Current linked 7 target producer 2 exit 0 but runtime 1 RED: first 10 PASS, case 11 Viewer UI
appearance palette selector exceeds existing 300s; PID 37548 killed/counts absent,
cleanup/source guards clean. Six remaining cases separately authorized/unrun in receipt;
not 16 PASS. One exact artifact-selector nocapture 60 s diagnostic authorized to expose
potential buffered primary panic, not acceptance retry/validation timeout reduction.
Private B0 v1 actual 34 PASS (14 new 20 direct)/Root d32d1ff2 retains 1 deprecated fetch_update
constructor warning/not strict qualified. Later actual host delta 9ddee287 and Core typed recipe
correction 29a13391 forwarded; composed Viewer GREEN v2 freeze 51c32688/inventory 58ff82bd
Root 7e910068 27 source verification accepted for private 104 Viewer+11 Host and 3 separate
original B0 v1 RED declarations, ALL runtime UNRUN/compile failure or zero cases not behavioral RED.
UI 5 extra production consumer reservations b72a7ef2 beyond UI 30 authorized privately;
original A1 freeze unchanged. No rebind of B0 v1 34 passes to later epochs. Production generic
resource/native/B1/full current goal remain open; no canonical lifecycle closure.
Root final partial runtime review b66594d0 now binds all 17 selector outcomes: 15 direct
PASS/1 Viewer TIMEOUT plus separate Office 710 unit PASS, overallAccepted=false. Six
independent continuation cases actually passed; original 10 PASS/300 s timeout retained.
Root checked 51 readback log-summary hashes/5 capture hashes (not visual review)/7 actual EXEs.
Separate 60 s nocapture diagnostic terminal RED exposed primary panic appearance.rs 508:
palette.restore_palette.unwrap Err("Viewer is closed."). Original 300 s RED unchanged;
not acceptance retry/validation timeout reduction. Backend stop cause NOT observed;
closed/stopped/catalog startup remains under source/runtime diagnosis. UI owns private
failure-only mailbox logging and fixture/lifecycle review; Architect reviews existing
PreparedCatalog seams. No production guard weakening/new fake runtime/canonical fix.
Full UI/new C/native/B1/DR/PO/full 54 remain open.
New normal validation-input 2 passed/current normal D app 673084bb…bbc95; 359 graph matches
notices, no runtime acceptance. Same original 7 target mailbox diagnostic v3 compile 0/
zero W-E and one actual selector PASS/0.21s/no mailbox error output; original closed panic/
300 s timeout remains unexplained, not fixed/accepted. UI approved private ACK-close
fixture validity change underway, not proven root cause fix. Additional sealed cleanup
`_build/verification/disk-cleanup-20261118-additional/final-result.json` records
C 412 files/3,255,585,645 logical B and D 642 files/7,250,716,439 logical B, protected EXEs
unchanged/164 default symbols+QA retention preserved; no net physical reclaim claim,
process kill or ACL change. Full 54/new C/native/B1/DR/PO remain open.
Additional Root v3 review 2b311c88 confirms 3 raw log pairs/2288 sources and independent Cargo
feature-profile variant equivalence/one exact diagnostic PASS 0.21; original failure still
unexplained/no acceptance. New Root source P1 review 77c65185 confirms private PO v2 missing
previously agreed typed preownership admission: token before handoff, serialized seal-admit,
immutable terminal receipt/late failure changes no counts results, refused already owned
source in caller checked disposition boundary. Old v2 immutable/nonintegratable;
Viewer narrow additive corrected epoch pending/Architect caller boundary confirmation.
Three Host behavioral RED compile actually live under Build, no runtime PASS/behavioral RED
yet. Preserve live jobs/old inputs and no rebinding; no full 54 closure.
Later Root 63867970 actual 3 exact Host/Viewer behavioral REDs independently verified:
9 raw log summaries/actual EXE 42422763/rescue+both joins BEFORE final assertions. RED compile 0
with 1 historical fetch_update warning retained/not strict green. Scoped 14 cooperative
GREEN v2 tests authorized/no PASS yet; distinct PO v2 typed preadmission P1 still open.
Root 085a source accepted sealed UI helper one file e3bc→1e73/25 records/19 old tests unchanged/
2 new UNRUN/rustfmt 0. Canonical promotion waits actual C assembly hold release and targeted
runtime; only helper tracked Sessions+entry ACK/no production catalog runtime input change.
Original stop cause not explained/fixed/accepted; whole LIFE/native/full 54 remain open.
Subsequent actual narrow GREEN 14 PASS/0/0=11 Host+3 previously RED Viewer independently
Root reviewed d5d14402: 4 raw logs/2 actual EXE/all 1294 source-proof-absence guards across 3
schemas. Private cooperative-cancellation boundary only/no canonical promotion/full 104/
typed PO/whole LIFE/native App. Zero-warning ordinary test compile is NOT full Clippy-D;
retain original 3 RED proof 63867970 and its 1 historical warning. Palette candidate 19 old tests
source preserved/2 new UNRUN still await actual C assembly hold release/promotion/runtime.
Root observed actual assembly PID 54224 with normal 673 and real test cf411/source epoch 9297;
destination being populated/not frozen accepted. Full 54 stays open.
Later C assembly actually completed 0/normal-only registration; Root c14c67c8 rehashed exact
2,487 files/752,608,762B incl 6 Viewers/3 FS/2 thumbs/662 notices/normal 673/test cf411.
Freeze/postregistration hashes match; no app/test launched/native acceptance. Actual
NativeRoot from native-identities/completion, verbatim:
`<user-profile>\MidnightExplorer-validation-20261118-07672814`.
After actual Build source hold release Root promoted ONE helper e3bc→1e73/fdf06c25 with full
2288 guards/fmt 0/normal 673 Main unchanged. Current canonical source 9c25 distinct old C9297;
same 7 graph compile+original palette case/2 new lifecycle cases pending 300s. Old C bundle and
original failed Epochs remain immutable/not rebound; cause unexplained/full 54 open.
Root PO v3 private source review 2db73d40: 33 sealed references/full 27 source/5 new Engine
changes 22 unchanged/canonical baselines exact; token-before capture/caller/neutral
Finished paths read. PRIVATE actual 3 compatible admission RED+129 GREEN expectations
authorized only, NOT results/canonical integration. Old v2 immutable/nonintegratable;
actual UI collector/App/DR/native LIFE pending. C9297 and current canonical 9c25 epochs
remain distinct; full 54 open.
Current fixture 1e73 runtime Root 9642bcbe: original palette 1 PASS/new normal teardown
300 s TIMEOUT/other 20 UNRUN/cleanup clean/current EXE 5b3/6 raw pairs. Pinned Slint 1.18.1
api.rs 503/1086 establishes show retains component until hide; new test never hide,
dropping Rust UI handles does not prove UI-callback-service termination precondition.
Source flaw proven, sole wait cause NOT proven. UI private additive one file hide+weak UI
disappearance correction authorized/preserve actual Session live before Runtime Drop.
Keep old failed packet/run distinct/no acceptance. Engine PO/UI collector work independent;
full 54 remains open, not marked blocked by this fixture boundary.
Full 54/new EXE/native/Git completion is not claimed.

### Clipboard and tree fixture ordering — 17 November 2026

CI [44029256172](https://github.com/TomasJelinek/fm/actions/runs/44029256172)
failed Gate 40 clipboard on both platforms: Linux **8 passed / 4 failed**,
Windows **11 passed / 1 failed**. Clipboard actor counts were **0** and trace
attempts `[6,0,0]`: the exact pre-ACK capture was canceled by independently
arriving startup-maintenance view churn before the capture command. No native
clipboard invocation or refusal occurred. Native subscription startup and initial
observation mark dirty; fixture readiness was not a maintenance-quiescence fence.
Production exact-projection/occurrence guards must remain strict.

Four **TEST-ONLY** diffs on `codex/ci-tree-fixture-ordering-20261117` have
local runtime qualification. An opt-in fixture holds only pure scalar `Changed` /
`DirectoryChanged` hints through native clipboard-transfer terminal, then releases
and replays the original hints before the original reconciliation/external-write
assertions. Native watchers and pre-ACK coverage remain enabled. Watch assertions
require the UI row plus native evidence for each changed directory: a fresh
callback from a live subscription that names it, or the registration of a batch
that contains it; a subscription without the directory and the periodic poll are
no evidence, and fences wait for a live subscription. A refresh admitted after a
callback may coalesce its hint, so a fresh per-mutation hint is not the contract.
A new deterministic Engine unit covers before-admission coalescing; existing
post-admission survival and default-timing tests are unchanged.

Root verified **Windows 37 passed / 0 failed / 0 ignored** and **Linux 37/0/0**:
Engine watch **16** (new before-admission coalescing plus retained post-admission
coverage), clipboard **12**, tree-watch **9**. UI compilation used workspace
feature context; default capture ran at four CPUs/four threads on both systems.
Engine used four Windows threads and two Linux threads. Source postimages were
unchanged; normal UID/SID runs exited all owned children. Both formatting checks
passed. Linux strict Engine `--lib --tests` Clippy exited **0 / 41.16 s** and
scoped workspace UI Clippy **0 / 32.47 s**. Windows strict Engine `--lib` and
scoped workspace UI Clippy exited **0**. The Windows Engine `--lib --tests`
lint supplement also exited **0 / 17.13 s**, covering the new test configuration.
Both platform build leases are released; no runtime checks were repeated for
the supplemental lint coverage.

Fresh full integrated CI remains **PENDING**, not GREEN; run `44029256172`
remains RED history. The original c97 Gate 43 failing predicate remains
**UNATTRIBUTED**: local fixture results do not prove its cause or a watcher behavior
fix. Architect/Perf final source reviews accepted the test-only changes. Windows
production remains unchanged from the qualified c97 EXE; no app rebuild/deployment
is needed. Earlier checkpoint results keep their own dates, inputs and scopes.

### Tree-watch CI follow-up — 17 November 2026

Root verified CI `44018173075` completed **19/20 successful jobs**, including
the full Windows check (release/native viewers). Only Linux complete Test failed,
at gate **43/47** on
`external_root_and_expanded_branch_changes_preserve_left_tree` / `right_tree`:
seven command-reclaim units passed, two native cases failed. The first create
wait at `tree_watch.rs:45` combines callback/hint/UI-row conditions; the failing
predicate and cause remain unattributed. The preceding 42 gates, including the
Engine correction, passed. No newer remote-CI result is claimed.

The resolved graph for `-p midnight-ui` has **415 nodes**, versus CI `--workspace`
**432**. Shared Slint **1.18.1** gains `unstable-wgpu-30`; Tokio `test-util`, X11
and tracing-regex features also differ. Two nine-case Linux diagnostic runs
passed (two threads **1.67 s**, four CPUs/four threads **1.69 s**), but are
**UNREPRODUCED / not CI-feature-equivalent**, with no watcher fix proven.
Root directly verified the **CI-feature-equivalent** workspace/tail receipts:
selected gates **43–47**, **34 artifacts / 131 passed / 0 failed / 1 ignored**,
including `tree_watch` **9 passed / 1.67 s** at four CPUs/four threads and both
`ui_polish` artifacts. The archive-worker case requires its separately built binary.
Source guards passed, all owned processes exited and the Linux lease was released.
Scoped workspace `--test tree_watch` Clippy exited **0 / 46.93 s**; pinned-format
reuse verifies diagnostic postimage `e625…`. This is **WSL/kernel 6.6**, with a
CI-feature-equivalent graph/selection, not the hosted CI environment.

Watcher passes remain **UNREPRODUCED**: the cause is **UNKNOWN**, feature differences
are not an established cause, and **no watcher behavior fix is proven**. Only
failure-only diagnostics were applied on
`codex/ci-linux-tree-watch-observation-20261117`. Windows metadata-only
`cargo clippy --workspace --test tree_watch --locked --offline` (jobs 2,
`-D warnings`) exited **0**, with zero errors/warnings; format exited **0** and
source guards passed for the same `e625…` diagnostic postimage. No runtime or
product rebuild ran; Root released the lease with no compiler processes remaining.
A diagnostic-only commit/new remote CI is pending. The c97 EXE remains valid for its unchanged Windows production source;
no runtime rebuild is required for this test/docs-only change.
[Execution guidance](developer/TEST_EXECUTION.md) requires the resolved feature scope.

The c97 host EXE is independently built and normal-profile deployed: SHA-256
prefix `2d906dea…`, **106,150,912 B**,
`<user-profile>/MidnightExplorer-20261117-044645-b596dbbb/me.exe`.
Inventory checked all 13 components / 1,823 files (only host changed, twelve
retained); validators and registration/pointer checks exited **0** as normal
non-admin, without launching the user app. This deployment does not qualify the
watcher cases or whole CI. Earlier pending states below retain their dated scope.

### CI and concurrent-copy investigation — 17 November 2026

Investigation input: `codex/ci-concurrent-copy-diagnostics-20261117`, following
published main `226848f`. Four modified files cover the host-only known-Closing /
epoch-reservation correction (`copy_namespace.rs` and tests), plus two bounded
UI integration diagnostic helpers. Architect/Root source reviews accepted it;
Perf accepted with a global-epoch-churn progress caveat. Local Linux and Windows
runtime qualification passed; new remote CI and refreshed EXE receipts remain
outstanding. Windows quality status is recorded below.

Linux regression proof: both tests-only cases reached intended **RED** assertions
(exit **101**). Corrected namespace **42/0**, four unchanged native-overlap cases
**4/0**, and complete Engine lib **1370 passed / 0 failed / 9 ignored**, **19.99 s**,
are GREEN. Strict Engine all-targets Clippy and format exited **0**, uid 1000 on
the existing native WSL checkout; source guard passed and all owned children exited.
These fix the two source-proven host races; the original CI result did not
distinguish them. Prior Linux main Test **1366/1/9**, synthetic WouldBlock publishing
`second.bin`, remains RED history.

Windows local runtime: namespace **42/0**, four unchanged native-overlap cases
**4/0**, complete Engine lib **1378 passed / 0 failed / 8 ignored** (1,386 total),
**29.57 s**. The normal non-admin run exited all owned PIDs and passed its source
guard; compilation had zero warnings. Windows strict Engine all-targets Clippy passed with zero errors/warnings;
format exited **0**, before/after source guards passed, and all owned processes
exited with none remaining. The earlier 4-CPU/4-thread diagnostic subset **12 passes / 3.32 s**
remains **UNREPRODUCED**, not independent fix proof.

CI runs `43996256276` and `44005451404` remain RED history. Run `44009395488`
has **all 18 package jobs GREEN** and Windows full Test **PASSED**, with Optimized
release build underway; its Linux main Test failure above remains. The correction's
new remote-CI result and refreshed EXE are **PENDING** at this checkpoint.
The last qualified EXE below predates this host fix; subsequent host-only warm /
private deployment and CI outcomes require their own actual receipts. Earlier
results below retain their own source/environment and do not qualify this fix.

The qualification preparation used feature branch `codex/copy-move-progress-20261113` at merge commit
`a93557d4fc650c6dd8d83ad4ddd66003841a3e5a`, incorporating `origin/main` `ccf7310`.
All 250 WIP files were restored: 237 byte-identical and 13 intentional
combinations. The additional `copy_delivery_tests.rs` combination adapts two old
helper calls without changing assertions; all 61 untracked files were retained.
These are merge/restoration facts, not a new test qualification. Root reports
both independent source reviews passed.

The user requires build/tests followed by commit, merge into `main` and push;
Root owns Git/integration after validation. This **16–17 November 2026** checkpoint
attributes results to each receipt's dated source identity, rather than the
commit that will contain this document. Three-target fixture qualification binds
449+13 freeze `0dd6200d…`; independent qualification binds keyboard450 freeze
`b743552c…`. Clipping qualification binds 452+13 freeze `aaf5b7c0…`;
each receipt retains its full identity and source guard.

Old D `_build` was deleted after hash-verified recovery, recovering approximately
**186.87 GiB**. Recovery manifest SHA-256 is
`e02fc095a219e2d495252a99c06eb6d4c663c3208ff9d2eb6289572ac08ef193`;
final deletion receipt SHA-256 is
`c2a942efdc1068dd1301ccc6671cff7ad30f8c29a5c5d4121b67b1c1b4a38be1`.
Both are retained on C; `final-recovery-file-mapping.jsonl` maps old D evidence
paths to verified backups. Historical paths below describe that recovered scope.
`.tools` was retained; archival binaries are not fresh build inputs.

| Current assignment | Verified scope and remaining boundary |
| --- | --- |
| Build/final QA | Initial **13 components** built; inventory **1,823 files / 641,250,519 B**, accepted exits **0**, zero compiler error lines. Classified **58 warning lines** (viewers 5, FFmpeg 52, thumbnailer 1) plus **8 cmp emissions** remain recorded, including original strict viewer diagnostics FAIL and continuation acceptance. Stopped redundant FFmpeg recheck **-1** earns no GREEN. Latest warm host, **13-component inventory/validator GREEN**; final qualification remains open. |
| Runtime verification | Canonical own-EXE verification defaults **OFF** (flag absent/`0`); `MIDNIGHT_VERIFY_RUNTIME_SHA256=1` opts into strict hashing. Latest Rust plugin-host tests **97 passed / 0 failed / 6 ignored**; all **33 installation selectors** passed. Native PowerShell scripts passed; fresh Rust public-API children consumed a PS-produced pointer with flag **absent / 0 / 1**, all passed. One-time deployment refresh is required for old hash-only registrations in OFF. No default EXE byte-equality claim; loader/path/permission guards remain. [Contract](developer/RUNTIME_DISCOVERY.md). Manual, UNC and Linux qualification remain separate. |
| Independent qualification | All **6 API**, **11 production plugin** and **2 support** boundaries passed, including separate C ABI qualification; root doctests exited **0**, all owned Cargo exited. Receipt `independent-all-green-keyboard450.json` retains original source/config/lock/toolchain guards through reviewed host test-only transitions. These results do not qualify physical desktop or the remaining root UI/native/deployment gates. |
| UI continuation | Keyboard **32 GREEN**, mouse **1 GREEN** and strict scoped checks passed. Clipping fix: **32/0/0** (settings 22, toolbar 3, forms 6, volume 1), unchanged assertions, 452+13 no drift. The **82 harnesses / 460 unique cases** account for **449 − 21 overlap + 32 fresh**; original row-Y61/clip-Y65 failure stays RED history. Scoped quality is GREEN. Desktop10 has no qualified observer: an accepted nonblocking qualification gap. |
| Native25 qualification | Original 14/1/10 and fresh 17.02 s ChangeTime-only RED remain preserved. The corrected first-use image07 passed in 17.00 s, followed by six related Viewer native passes. A separate thumbnail observer ended before a queued request was observed; its test-only correction retains the production two-second timeout and verifies actual queued cleanup (2.008 s). The four-case correction/deferred continuation passed 4/0/0. Together with 14 retained earlier passes, all **25 selected native cases are qualified**. Viewer libraries passed **33/0/4 + 256/0/4**; plugin-host **97/0/6**. The 26 new Viewer definitions are included in these passing suites. Scoped formatting/strict Clippy passed. Host warm, 13-component inventory/runtime validator and actual normal-profile deployment/launcher registration passed. Linux3/UNC/desktop10/manual gaps remain explicit. |
| Fixture-correction qualification | After 23 reviewed test-only fixture corrections, full Engine lib **1375 passed / 0 failed / 8 ignored**, filesystem-host lib **46/0/1** (47 selectors), RealOps **14/0/1**. Receipt `fixture-corrections-green-v1/three-target-result.json` records all three GREEN and **449+13** guards without drift. Original Engine **57 RED**, FIFO **1 RED** and filesystem-host abort remain historical. Eight Windows environment failures reran **8/8 GREEN** with short TEMP and unchanged source. No production queue/security weakening. Later rows record the completed scoped normal/API/plugin/doctest/native/warm/deployment qualification; desktop/manual/platform gaps remain separate. |
| Copy/Move | The specific authority fix passed **five required runtime cases**. AB56 was **78/1/0**: B-pure 71/1/0, g059 2/0/0, required-five 5/0/0; retirement waiter observed **3**, expected **2**, and **four critical selectors were unrun** in that phase. Reviewed test correction is canonical; retain original failure. Native timestamp ownership has meaningful old-RED then GREEN and fixture-v3 GREEN; the early ErrorKind mismatch was not meaningful RED. Final broader validation is pending. |
| Native/Viewer prior results | Native52 parent **5/0/0**, corrected Native53 namespace **9/0/0**, strict scheduler witness **1/0/0** retain their inputs. Latest independent Windows Source qualification is **49 passed**, Markdown **76 passed / 1 ignored**, each across all **13 normal artifacts**; earlier Source 41/Markdown 42 are prior subsets, not additive. Old public-ABI paging failed unchanged **2 MiB**: C++ **3,165,411 B**, JSON **3,154,953 B**; candidate **1,138,446 / 1,122,063 B** passed. Metric: incremental Rust allocation requests, not RSS/general timing or physical desktop/full-app qualification. |
| Comparator | Private Core GREEN **43/0/0** (configuration 30, policy 8, preferences 5), compile **9.7549131 s**, zero errors/warnings; Core remains excluded from canonical. UI baseline was a setup failure. Private Core/UI/Viewer candidates do not implement canonical comparator feature/settings/routing/external launch. |
| Retained evidence | Prior 420-file six-artifact compile, **49/0/0** selected passes and separate native50 **0/1/0** retain their inputs; so do the prior **69-artifact** compiler and interrupted attempt. Archive Options Host 2/UI 16/installed-catalog 1, old normal fixtures 4/5 and corrected catalog51 1/0/0 remain scoped, not full qualification. Recorded strict Clippy v4/source-size passes do not establish final current-source QA. |

Retain the separate **188 producer/UI passes / 0 failed / 1 existing optional
benchmark ignored** and host-foundation **54/0/0 Windows** plus **54/0/0 Linux**
at their recorded inputs (foundation commit `4570154`; Linux functional only).
These are not combined new scheduler/current-head evidence. The earlier private Windows app **`937e9bfc`** retains its original source
and limits below; the refreshed 17 November executable and registration are
recorded next. Imported Linux desktop/runtime notes retain their own scope.


### Final Windows checkpoint evidence — 17 November 2026

The production host passed a zero-warning warm build (114.43 s). Inventory
`final-warm-f450-pin466/final-component-artifact-inventory.json` checked all 13
components and 1,823 files; the separate runtime validator exited 0. The unchanged
12 independent components were retained, without replaying their builds.
Viewer qualification binds 466+13 freeze `a7a32151…`; only the thumbnail native
observer fixture changed for 467+13 freeze `fdc08657…`. No production decoder or
application timeout changed in that correction. Original failed receipts remain.

Receipts under `_build/verification/final-review-tests-20261116/` include
`viewer-pin-visibility-v4-libs/plugin-host-result.json`,
`native11-pin466-first-use/native11-first-use-result.json` (7 passed, then the
thumbnail fixture failure), and
`native4-observer467/native4-observer-correction-result.json` (4/0/0). The retained
14 earlier native passes keep their original source and provenance. This is
scoped automated acceptance, not physical desktop or Linux qualification.

Actual normal-profile deployment and the protected registration plus launcher
pointer were checked in `final-normal-profile-deploy467/verified-registration.json`.
Source and deployed EXE SHA-256: `803a8b72bb98c5d804943905f76fe31bc7d01ee0b5bbfbda5ba0caf51e04629e`.
Installation: `<user-profile>\MidnightExplorer-20261117-012737-57ffdd4e`. No application was launched for this
deployment. Own-EXE SHA verification remains default OFF, opt-in with
`MIDNIGHT_VERIFY_RUNTIME_SHA256=1`. Git/remote CI outcomes are reported separately.

### Retained incoming-main notes and earlier checkpoints

The following main-branch integration notes and Windows pause checkpoint are
preserved verbatim. Their authorization/status wording describes those records;
the explicit active checkpoint above governs current assignments.

The current Linux integration was explicitly requested after this branch was verified.
The current [required-module integration report](verification/2026-11-13-required-runtime-modules.md)
records the merge of main `e77eafa` with Linux head `e6d0dbd`, the schema-32
migration of both legacy schema-31 variants, mandatory runtime packaging and the
archive activation fix. The schema-31 references below describe their original
branch evidence, before the integration migration.
The separate Windows checkpoint below remains historical; merging its source does
not resume its pending Custom Commands/comparator backlog.

The separate Linux branch `codex/filter-linux-drive-list` has Debug-built desktop,
permissions, terminal, activation and status-bar fixes based on merge `2b62996`.
Its [scoped Linux verification report](verification/2026-11-13-linux-desktop-fixes.md)
records 106 passing targeted tests and native X11/startup checks. The first
Colors occurrence was restarted before capture. A later live hang revealed a
Slint popup-borrow panic followed by an idle image-owner runtime shutdown wait;
the report distinguishes those captured stacks, the reproduced tooltip panic
and their source fixes from the unavailable original UI callback.
The shortcut Replace/Cancel dialog, unavailable network-button hiding, tooltip
teardown and panic cleanup pass 68 final targeted Debug tests. Refreshing a stale
installed native filesystem plugin adds 8 passing mount tests; the resulting
Debug installation passes native startup/shutdown with the filtered drive bar.
Three separate presenter tests also passed. Exact follow-up receipts are in the
same report.
The later Options sidebar correction measures the full selected topic labels
and grows with the font; 51 targeted UI tests pass, with font-12/font-24 renders
visually reviewed. Its separate receipts and scope are recorded in the report's
[font-scaling follow-up](verification/2026-11-13-linux-desktop-fixes.md#options-navigation-font-scaling).
The subsequent default-on **Allow previews inside archives** setting includes
strict schema 31 persistence, reversible Options edits and archive-only scheduling/
presentation gating. Its [scoped follow-up](verification/2026-11-13-linux-desktop-fixes.md#archive-preview-option)
records 214 passing targeted tests and a visually reviewed default-checked control;
this does not broaden installed archive-format qualification.
The following [row-layout and notification follow-up](verification/2026-11-13-linux-desktop-fixes.md#panel-row-alignment-and-notification-cadence)
limits list labels to one line and reduces notification debounce to 150 ms quiet /
500 ms continuous activity. It records 55 passing targeted tests, three complete
native rendering probes and Debug startup/shutdown. A hidden mandatory line break
reproduced the clipped text; the original screenshot's filename and build age
could not be established, so its exact trigger remains unconfirmed.
Standard builds and release packaging now require both image/video thumbnailers.
The [required-thumbnailer follow-up](verification/2026-11-13-required-thumbnailers.md)
records the actual Linux Debug installation, pinned FFmpeg source/signature,
151 passing targeted checks and five Windows-only skips. Installed discovery,
sandboxed decoding, cancellation, real package validation and native app startup
passed. Windows build/deployment integration is source-reviewed, not natively
qualified by this Linux run.
The subsequent [local-preview and Copy refresh correction](verification/2026-11-13-local-previews-copy-progress.md)
verifies bounded LVM backing-device classification and independent 250 ms Copy
status publication. A real PNG from the reported Screenshots folder failed
admission before the provider correction and decoded/persisted afterward.
The two dropped-progress regressions also failed before correction. The final
scope records 75 passing targeted checks plus native Debug startup/shutdown;
the native provider was replaced atomically and the running app was not restarted.
The later [file-tile drop correction](verification/file-drag-drop.md#2026-11-13-file-tiles-target-the-current-folder)
lets flat-panel ordinary files and screenshot tiles target the current folder.
Its actual pointer regression preserves the selected view through drag startup;
66 targeted checks pass, including folder/identity/tree guards and backend
containment/admission. Native Debug startup/shutdown also passes; the user's
`www` folder was not moved or copied by verification.
These Linux results do not replace the Windows evidence below.
The later [Linux network investigation](verification/2026-11-13-linux-network-discovery.md)
installed the missing system `wsdd` helper and fixed WSDD namespace handling;
84 targeted tests pass, including installed-plugin discovery through GIO. Native
enumeration currently returns zero devices; remote SMB/FUSE remains unverified.

### Separate Windows checkpoint

**PAUSED by explicit user request on 13 November 2026.** The user requested a
checkpoint of all current work and a push; Root owns review, integration and Git.
No new implementation, design, documentation polish or qualification work is
authorized while paused. Build's final height-fix compile finished with **exit 0,
zero errors and zero warnings**. The 28 direct UI tests and a fresh application
run were not executed. Owned children are stopped and build leases released.

The current 12-file Custom Commands UI overlay includes the latest height fix
(`c99f96db`), but final targeted tests and a new runtime remain **UNQUALIFIED**.
The original parallel test-process abort **`0xC0000409`** remains unresolved.
The isolated reverse-drag case passed **1/1**; the full serial group and original
parallel pair did not abort, but both failed the known alignment assertion.
These diagnostics do not establish that the abort is fixed or unrelated. The
last qualified private executable remains **`937e9bfc`**, with its existing scope
and historical receipts unchanged. This checkpoint includes the four canonical
`performance_probe` examples as **UNQUALIFIED WIP** in the user-requested
checkpoint; they establish no product qualification. Comparator settings remain
**PLANNED ONLY**.

Before this paused checkpoint, Root reported main committed/pushed and
push-verified through
**`4f27e16c6ad233b1294d01d446ae560d8b9a3a9a`**, following `ab1d561` and
`c2a6560`. This is not a new exact-head remote-CI receipt. The registered,
visually reviewed private Windows runtime is now **`937e9bfc`**. Its normal debug
build used base `49312b42ff08027814dff8ce97f8d84e8a5929e4` plus **83 reviewed
overlay paths**, not the later pushed head; [exact identity, targeted evidence
and limits](#current-windows-launch-qualified-runtime-937e9bfc) are below.

Earlier checkpoint history: Root committed/pushed
**`174c5fd6d426d23f873285eb5c70ffaa6ec28124`**:
38 exact source/documentation files including the COPY core/queue layer and
app-private HTML layout handshake. Root committed/pushed the later three-file
format-only correction as **`cdc9226c0ce04f2ee4b492bdf7763479f5500956`**;
its exact-head remote CI remained pending in that record. The earlier debug
attempt at `174c5fd` exited **101** after
**674.914 s**, zero source drift: cached core binary dep-info watched the older
COPY overlay. The origin-audited `retry-mtime-v1` also failed **E0460** after
**759.096 s** because cached `midnight_native_filesystem_host` referenced a
different core build. The retry had zero source drift and produced no EXE
artifact. These failed attempts retain their historical scope. The later coherent
normal debug build at `cdc9226` passed **254.609 s**, zero source drift, with all
**14** consumer origins rebuilt/coherent. The registered private Windows
startup-qualified EXE at that checkpoint was `f5b29eab`, source `cdc9226`;
exact identity and scope are [below](#current-windows-launch-qualified-runtime-f5b29eab).
Original failed receipt: `_build/verification/integrated-debug-20261113-v2/debug-build-result.json`.

| Area | Current source state | Acceptance still open |
| --- | --- | --- |
| COPY-03 preserved links | Internal core/queue layer integrated; default `Reject`, native hooks `Unsupported`, no UI enablement (13 November; since 20 November Linux Copy and Move transfer links as links, see the [current checkpoint](#current-checkpoint)). | Native Preserve proof/hooks, Move/exact-time Preserve and remaining metadata capabilities; [exact scope/evidence](#provider-increments-from-the-architect-review). |
| HTML | App-private token/generation layout handshake integrated; public plugin API unchanged. | App/native opening/layout/performance/deployment evidence; old failed native measurements remain unqualified. |
| Native Rename | Public/native/core-host source integrated. | Engine/UI routing and finish ownership, installed flat/tree/native-provider workflows and deployment. |
| Tree/DirectoryLink | Captured actions, recursive expansion and loaded-link maintenance implemented; cap128 has a passing synthetic-only gate. | Native I/O/UI paint, physical Linux input, larger-than-model paging and archive/provider gaps. |
| Current runtime/CI | Registered, visually reviewed private runtime `937e9bfc`; normal debug build **0 / 584.880 s**, 14 coherent consumers, zero warnings/errors/drift. Source base `49312b4` plus 83 reviewed paths is distinct from Root's pushed main `4f27e16`. Windows **208/0/0** and Linux **40/0/0** targeted evidence; scoped strict Clippy passes. Previous `f5b29eab`/`516fc158` and failed build/CI evidence are retained. Exact-head `9176272` remains the retained remote **20/20 GREEN** receipt. | No newer exact-head remote-CI GREEN or full-suite rerun is established. Complete tree/HTML/native performance/workflows, Ubuntu/Linux graphical Trash opening, Source53 and optional vendor matrices remain unqualified. |
| Comparator settings — **PLANNED ONLY** | Proposed [design](superpowers/specs/2026-11-13-comparator-associations-design.md) and [implementation plan](superpowers/plans/2026-11-13-comparator-associations.md) describe configurable comparator associations. | Separately scoped implementation and qualification remain future work; the current runtime and retained comparison evidence do not qualify these proposed settings. Unrelated backlog remains parked. |

[Archived run detail](verification/2026-11-13-checkpoint-evidence-history.md) retains unique failures, full hashes, deleted-QA
provenance and overlapping-count arithmetic. Existing
[recursive/tree](verification/2026-11-12-recursive-tree.md),
[panel](verification/2026-11-12-panel-interaction-polish.md) and
[CSC](verification/2026-11-11-directory-authorization.md) reports retain their
original qualification; later WSL/software fixtures do not relabel those runs.

## Authority and evidence

Current explicit user requirements and scoped assignments take precedence.
Later accepted refinements supersede conflicting earlier descriptions:

- Cross-volume/merged Move verifies and removes each successful file before
  advancing, then removes empty source directories postorder. Earlier completed
  moves remain after later cancellation/failure; there is no whole-batch rollback.
- [Transfer verification and native Move](superpowers/specs/2026-11-10-transfer-verification-and-native-move.md)
  keeps full SHA-256 as the default, permits explicit size plus first/middle/last
  64 KiB verification, and uses native same-volume rename without content reads.
  Files up to 192 KiB are compared completely in quick mode. Each job freezes its
  policy; source identity, revision, sync, consent and publication guards remain.
- The [11 November dialog policy](superpowers/plans/2026-11-11-dialog-polish-followup.md)
  supersedes blanket native-modal migration. Modeless tools move independently;
  ordinary dialogs are embedded modals with focus/input gating and no taskbar
  entries. The current source includes operation/Copy/Move drafts, Panel Filter,
  Hot Paths, Make File List and blocking notices in that presentation; existing
  Options/comparison/metadata modals remain embedded. Independent modeless tools
  retain native hosts.
- [Archive streaming](superpowers/specs/2026-11-09-archive-streaming-design.md)
  supersedes the old fixed source/member byte ceilings for the negotiated
  streaming protocol. Memory, entry/name/decoder limits, cancellation, integrity
  and private-storage failures remain bounded and explicit.

Source establishes an implementation boundary. A test definition, checked plan,
enabled control, old binary or green subset does not establish current end-to-end
success. Dated reports retain their exact source, environment and scope. Work
below distinguishes **implementation gaps**, **source review**, **performance**
and **qualification** rather than converting every missing receipt into missing code.

## Current role boundaries and exclusive ownership

| Role | Current focus |
| --- | --- |
| Orchestrator | Coordinate validation, exclusive ownership, independent acceptance and Git/main/push after receipts; preserve private/unintegrated boundaries. |
| Architect | Review contracts and qualification limits. Private comparator source remains separate from canonical feature/settings/routing/external launch. |
| Backend agent | Finish formal first-use native11 and deployment evidence; retain scoped Copy/fixture results and unattributed native failure. Private Core remains excluded. |
| Viewer agent | Retain Source 49/Markdown 76+1 ignored independent scopes and prior allocation evidence; desktop/manual/packaged-runtime gaps and private comparator separation remain explicit. |
| UI agent | UI 460 unique cases/82 harnesses and scoped quality are GREEN; retain original RED and private comparator setup-failure history. |
| GitHub build system and build tools developer | Preserve guards/receipts; warm/inventory/validator GREEN. Complete formal native/deployment, then monitor/fix GitHub after Root pushes. Desktop observer gap is accepted. |
| Perf agent | Merged Copy/Move source/cost review and actual measurement boundaries; preserve retained graph/metadata/parent-handle costs, with no general linearity or unmeasured speedup claim. |
| Documentation guru | Preserve all incoming main notes/IDs/history while updating the active checkpoint and exact receipt scopes privately; Root coordinates canonical integration. |

Assignments require acceptance criteria and explicit file ownership; role chats do
not authorize unrelated backlog work. Agree shared API/UI edits with the Architect
and Orchestrator. One owner holds each Cargo target lease; compile compatible
groups once, bound independent headless runs and serialize desktop/input tests.
Root integrates source and owns Git; preserve exact source/environment, failures/ignores and
unresolved work in handoffs. Follow [Agent workflow](developer/AGENT_WORKFLOW.md),
[code style](../CODE_STYLE.md), [security](../SECURITY.md) and
[dependency policy](DEPENDENCY_POLICY.md).

## Integrated baseline and recorded evidence

<a id="current-windows-launch-qualified-runtime-937e9bfc"></a>

### Current Windows launch-qualified runtime `937e9bfc`

The final receipt is
`_build/verification/disk-recycle-combined-20261113/final-qualified-lane-result.json`,
SHA256 `89090e27e82ae4718794a3f8ca4a666d9ab1b4217c4e22c10ec7c2939c803ac6`.
It records normal debug **exit 0 / 584.880 s**, **14** coherent core consumers,
zero compiler warnings/errors/native-build diagnostics and zero source drift.
The registered private EXE is
`<user-profile>/MidnightExplorer-20261113-123202-937e9bfc/me.exe`,
**102,327,296 bytes**, SHA256
`937e9bfc9caeaf107845a8538fd871e0cf0534694ca91eb9c3a4c00b6b6f1cdd`.
The real PE VERSIONINFO, icon and manifest checks pass: file/product version
**0.2.0** and the copyright line of the project's license.

App-build source is base `49312b42ff08027814dff8ce97f8d84e8a5929e4` plus
**83 reviewed overlay paths**. The snapshot binds **3,650 shipping app inputs**;
four unqualified diagnostic examples are excluded from the 3,654 observed inputs.
App freeze preceded the accepted test-only correction to
`crates/midnight-ui/tests/recycle_bin.rs`; production source was unchanged after
freeze. This does not identify the EXE as a build directly from later pushed main
`4f27e16`.

Engine **17.484 s** and native UI **7.328 s** startup smokes exited **0** without
timeout; their owned jobs were released. Registration followed passing smokes.
The exact native startup screenshot was visually reviewed for populated panels,
Unicode, Recycle Bin artwork and surrounding controls. All **1,382** new and
**1,382** previous runtime files were hash-verified; fixtures, preferences,
shortcuts and prior runtimes were preserved. No candidate native plugins were
deployed. The linked `app-debug/delivery/final-qualified-debug-result.json` and
`app-debug/delivery/visual-review.json` under the same lane retain exact scope.

Targeted functional evidence is **Windows 208/0/0** (206 retained passes plus two
corrected Recycle Bin cases) and **Linux 40/0/0** (retained across the reviewed
one-line derivative, with no functional rerun in that follow-up). The earlier
**10-case work-budget** gate remains separate; these counts are not a combined
unique-test total. Scoped strict Clippy passes with zero warnings/errors:
Windows **10 selected packages, libraries plus 15 exact direct integration
targets**, app feature closure and `-D warnings`; Linux **two recorded scopes**.
Full suites were **not rerun**.

This qualifies the private Windows startup/snapshot and the named targeted
gates. Ubuntu/Linux graphical Trash opening remains **unqualified**: Root reports
no owned graphical session. Source53, optional vendor matrices, performance,
complete workflows and newer exact-head remote CI remain unqualified. Comparator
settings are planned only. Earlier runtime/failure evidence below retains its
original source/environment and scope.

<a id="current-windows-launch-qualified-runtime-f5b29eab"></a>

### Previous Windows launch-qualified runtime `f5b29eab`

The coherent normal debug receipt
`_build/verification/integrated-debug-20261113-v2/coherent-final/debug-build-result.json`
records **exit 0 / 254.609 s**, source
`cdc9226c0ce04f2ee4b492bdf7763479f5500956`, zero drift and all **14** consumer
origins rebuilt/coherent. The registered startup receipt is
`_build/verification/integrated-debug-20261113-v2/final-qualified-debug-result.json`.
The private EXE is
`<user-profile>/MidnightExplorer-20261113-083454-f5b29eab/me.exe`,
**102,148,096 bytes**, SHA256
`f5b29eab9776990bb15c82b60b0fe5fa4b19421c05d089a5313a4478f9dc8a88`.

Engine **12.101 s** and native UI **6.386 s** startup smokes both exited **0**,
without timeout; registration followed the passing smokes. Fixtures/preferences
and the desktop shortcut are unchanged. All **1,382** new and **1,382** previous
runtime files were hash-verified. The build snapshot matches **3,636 app inputs**;
four dirty diagnostic examples are excluded from the 3,640 observed inputs.
Root independently checked all 14 consumer origins and the EXE hash, and visually
reviewed both panels, Unicode and footer. The previous qualified
[`516fc158` runtime](#current-windows-launch-qualified-runtime-516fc158) is retained.

This qualifies private Windows startup/snapshot identity only. Complete tree,
HTML, native performance and workflow qualification remain open, along with
physical Linux UI and installed native Rename deployment. Candidate native
plugins were not deployed. At that checkpoint, exact-head remote CI at `cdc9226`
remained pending.

<a id="current-windows-launch-qualified-runtime-516fc158"></a>

### Previous Windows launch-qualified runtime `516fc158`

The receipts under `_build/verification/integrated-debug-20261113/` record a
fresh debug build, **102,041,600 bytes**, source
`d6455a0133df5e279ffbdab62b86691007c2c5be` and zero drift. The private binary is
`<user-profile>/MidnightExplorer-20261113-051356-516fc158/me.exe`,
SHA256 `516fc158567c2179087f4c166d23c8b51387c8c077236fc87bcabfa684226018`.

`qualified-debug-result.json` records engine **17.882 s** and native UI **7.901 s**,
both exit **0**, without timeout/fallback. Root/Build visually reviewed both
panels, Unicode text, toolbars, command bar and footer. The recorded build snapshot
matches **2,327 application inputs**; three unqualified diagnostic examples are
excluded from the 2,330 observed inputs. All **1,382** new and **1,382** retained
previous runtime hashes were verified. Fixtures/preferences/registrations and the
desktop shortcut remain unchanged; the previous `c6c371bc` runtime is preserved.
Owned jobs and source/target/native-input/performance leases were released.

This is private Windows startup/snapshot qualification. Native Rename candidates
were not deployed. It does not establish full workflows, native tree/viewer
performance, physical Linux UI or remote CI GREEN. PID 29940 remains unclassified;
these startup runs do not prove generic natural auxiliary-process lifetime.

Source/HTML performance qualification remains **failed/unqualified**. Source v4
(`viewer-20261113/inactive-source-gate-v4/result.json` under `_build/verification/`)
passes compile/scoped Clippy with zero drift, but native gesture submission fails
on hidden/minimized/invalid geometry; no completion report or qualified first
publication is available. HTML's `html-queue-build-owned/final-manifest.json` under
the same viewer directory retains diagnostic exit **1**, default skipped, zero
accepted first publications/draws and zero completed wheel rounds. No queue
attribution or tracing-overhead comparison is established.

### Image-spare shutdown gate

Root committed the seven-file source correction as
`3e2238da6b26ddd61dab5cddb91660158a710b76`. The
[lifetime contract](developer/ENGINE_LIFETIME.md#image-viewer-spare-shutdown)
describes spare stop/terminal acknowledgment before runtime cleanup and retry.
The combined receipt (`_build/verification/viewer-20261113/image-startup/shutdown-gate-result.json`)
records **36 headless + 3 native passes**, default/diagnostics compile and strict
Clippy **0**, zero source drift, released source/target/native-input leases and
removal of the owned staged executable. The three native cases cover specific
Windows worker boundaries, including suspended-child Job membership and cleanup
after the last inner handle closes. This qualifies source-free spare retirement
and those worker boundaries only. PID 29940 is unclassified; general natural
auxiliary-process lifetime remains unqualified. At that source gate the private
EXE was `c6c371bc`; the later `516fc158` startup includes this correction without
broadening the shutdown gate's scope.

### Native link presence and Find gate

The native captured-root DirectoryLink admission correction passes
**18/0/0 = 12 presence + 6 durable Find regressions**, compile 0/zero drift.
Its prior presence RED is **6/2/0**. The earlier exact-user-fixture diagnostic is
**13/0/0**, with unchanged raw path/kind/identity/revision; the six final Find
regressions overlap it and did not rerun that user fixture. The original Open
report's build/capture is unknown: no production Find fix or false-stale cause is
proven. [Exact fixture, source pin and receipts](verification/2026-11-13-checkpoint-evidence-history.md#native-link-presence-and-find-gate)
remain retained.

<a id="tree-scaling-candidate-gate"></a>

### Tree scaling candidate gate — aggregate failed

Historical cap16 passes **95 unique functional cases/Clippy 0**, but its opt-level 0
synthetic scaling aggregate **failed**. The stale initial artifact was rejected;
unrepeated/censored baselines, exact times and callback/publication counts remain
in the [historical scaling record](verification/2026-11-13-checkpoint-evidence-history.md#tree-scaling-candidate-gate).
The separate cap128 result below does not rewrite that failure.

### Recursive tree cap128 gate

Cap128 source `2b28cc7` is integrated in D merge `02b2f2f`. The recursive batch
bound is **128 returned results or 50 ms**, with elapsed time checked only after
an actual provider callback; it is not a callback timeout. The
[contract](developer/TREE_PANEL.md) preserves exact Commit, cancellation and
captured-action safeguards.

The recorded cap128 build/functional gate passes **25/0/0**, compile **0** and
strict Clippy **0**, with zero source drift. The cap128 scaling receipt records
one passing **opt-level 0/debug synthetic aggregate**: an untimed warmup and four
timed fixtures, the same workloads and unchanged **60 s** cutoff, no retries and
no new baseline run. `N` includes the invisible root; `K` is settled cached sizes.

| Fixture | Wall time | Publications | Returned results / callbacks |
| --- | ---: | ---: | ---: |
| Wide `N=1,000`, depth 1, `K=0` | 110.1866 ms | 8 | 999 / 999 |
| Wide `N=10,000`, depth 1, `K=0` | 7,140.3042 ms | 80 | 9,999 / 9,999 |
| Wide `N=10,000`, depth 1, `K=1` | 6,904.8673 ms | 80 | 9,999 / 9,999 |
| Broom `N=10,000`, depth 32, `K=0` | 8,731.4107 ms | 79 | 9,999 / 9,999 |

All four timed fixtures complete with `failureNone=true`. The aggregate is one
harness case (**1/0/0**), not four additional functional tests. The retained cap16
binary is unchanged; source/target/performance leases were released and owned
job cleanup completed. Exact evidence is under
`_build/verification/backend-tree-recursive-cap128-20261113/` (`result.json`,
`cap128-build-result.json` and `cap128-scaling/result.json`). This measures
synthetic backend work, with no native filesystem I/O or UI paint. It does not
qualify release performance, SSD latency, physical Linux UI or visible smoothness.
The retained `c6c371bc` runtime excludes this source. The later `516fc158` runtime
contains it, with startup-only qualification rather than native tree timing.

### Integrated source and targeted gates

| Gate | Exact retained scope and limit |
| --- | --- |
| HTML layout handshake | `174c5fd` source; compile/strict Clippy 0, **135 selected/executed/passed, 0 failed/ignored**, **35.656 s**, artifact/source drift 0, owned jobs released. Core tests opt-level 0; engine/UI opt-level 1 with debug assertions. Headless evidence only: no app/native measurement or deployment. [Contract](developer/WEB_VIEWER_API.md). |
| COPY core/queue | Integrated internal layer; [80-unique accounting, prior 77/3 and queue 17 retry](#provider-increments-from-the-architect-review) are retained in COPY-03 below. Native hooks/UI/Move/exact-time Preserve remain unavailable. |
| Original recursive/CSC/thumbnail | [Tracked original report](verification/2026-11-12-recursive-tree.md): Windows/software UI 69; broad Backend 158/1 and overlapping refined 11/0; thumbnail 12/2 → 14/0. CSC mocked/denied/cancelled helper paths do not prove privileged access. |
| Later Linux fixtures | Exact `1714d99` real symlink **1/0/0** and updated frozen DirectoryLink fixture **1/0/0** are separate WSL/tmpfs receipts. Helper UI **13/0/0 = clipboard 5 + copy 2 + move 6**, after an initial missing-`pkg-config` compile with no tests. They are not a 69-case native Linux rerun or physical desktop qualification. [Source/CRLF-wrapper provenance](verification/2026-11-13-checkpoint-evidence-history.md#selected-local-gates-and-linuxprobe-provenance). |
| Worker test-only CI correction | `38800aa`: strict `--lib --tests` default **18.3488 s**, diagnostics **1.6885 s**, both exit 0/libtest artifacts; **9 preserved inputs/0 drift**, rustfmt 0 reported. No runtime tests/app rebuild; not remote GREEN. |
| Linux alias/octal compatibility | `9176272`: strict all-targets Clippy exits 0, including library `cfg(test)`, **13.8297 s**, zero drift. Runtime tests were not rerun; no capability semantics change. [Receipt/source context](verification/2026-11-13-checkpoint-evidence-history.md#native-rename-gate-history). |

<a id="qualified-ui-checkpoint-35a159c"></a>
<a id="image-prewarming-source-and-evidence"></a>

### Earlier bounded evidence

The [recorded evidence note](verification/2026-11-13-checkpoint-evidence-history.md#earlier-bounded-evidence) retains the original
hashing anomaly, Markdown 69/2 → 72 and debug A/B costs, failed Source/PNG admission,
7.192-second control, prewarm compile/Clippy 101, R2 34/9 → 43 unique and R3
44 unique scopes. The five native controls/BMP pair are R2-only. Source
integration/one same-candidate host observation does not qualify application F3,
all images, steady idle resources, native scrolling, Source gesture or RSS.

### Retained remote CI boundary

Build's exact-head watch records **`9176272`: 20/20 GREEN**. The historical
`174c5fd` result is **not green**: both format checks failed **13 hunks across
3 files**. After Build released the failed-build source lease, Root applied the
three-file format-only correction, committed/pushed as `cdc9226`; its exact-head
remote CI remained pending in that record. Root now reports main pushed/verified
through `4f27e16c6ad233b1294d01d446ae560d8b9a3a9a`, following `ab1d561` and
`c2a6560`; no newer exact-head remote-CI GREEN receipt is established here.
The separate registered `937e9bfc` startup receipt is
[above](#current-windows-launch-qualified-runtime-937e9bfc); it does not establish
remote CI or complete workflow/performance qualification. The
[older remote/local failure chain](verification/2026-11-13-checkpoint-evidence-history.md#retained-remote-ci-boundary)
retains `02b2f2f` run 43367992268/job 131699200987, 43345016027/43312251721,
exact `1714d99`, failed-unreached targets and fixture/lint correction provenance.

### Completed build-script integration

The [recorded build-port/merge evidence](verification/2026-11-13-checkpoint-evidence-history.md#completed-build-script-integration)
retains exact branch/input identities and non-additive test scopes. Historical
source integration is not a current workspace/remote test pass.

### QA artifact cleanup

The [completed QA cleanup record](verification/2026-11-13-checkpoint-evidence-history.md#qa-artifact-cleanup) retains authorization,
681 roots, non-additive free-space observations, protected executable hash and the
original Markdown 62/allocation handoff. `_build/qa/` was removed; historical paths
are provenance, not required extant files or new command recommendations.

### Retained private runtime history

<a id="previous-windows-launch-qualified-runtime-c6c371bc"></a>
<a id="previous-windows-launch-qualified-runtime-26ff61f2"></a>
<a id="previous-windows-launch-qualified-runtime-269f3419"></a>
<a id="previous-launch-qualified-runtime-d2643b49"></a>

Exact earlier runtime identities/startup scopes are preserved in the
[private-runtime history](verification/2026-11-13-checkpoint-evidence-history.md#retained-private-runtimes) and original linked
verification notes. The unclassified PID 29940 and old hashing anomaly remain
unattributed; later clean owned-job samples do not identify them.

## Current source/contract boundaries

| Area | Current source / contract | Boundary to preserve |
| --- | --- | --- |
| Filename palette | [Defaults](../crates/midnight-core/src/filename_colors.rs), [palette contract](developer/FILENAME_COLOR_OPTIONS.md) | Twelve ordered categories; PDF base `#EF5A5A`; source code, Word documents and plain text/Markdown share `#06A4C7`. Rendered contrast may adjust bases. Filename coloring does not imply decoder support. Exact historical defaults migrate; custom rules survive. |
| Selection defaults | [Shortcut catalog](../crates/midnight-core/src/shortcuts/catalog.rs), [tree row policy](../crates/midnight-core/src/tree_view.rs) | Ctrl+D clears every mark, including folders/expanded descendants; Ctrl+Shift+D has no default. Ctrl+A/Ctrl+Shift+A replace top-level selection and clear nested marks; Ctrl+Shift+I inverts top-level entries while retaining descendant marks. Saved deliberate overrides remain authoritative. |
| Captured inline actions | [Tree panel contract](developer/TREE_PANEL.md), [current status](FEATURE_STATUS.md) | Typed native consumers exist for transfers, clipboard/drag, focused editor, mutations/metadata, Properties/context menus, checksums, file lists and comparisons. F3/file comparison use visible root files; inserted branches still contain folders. Archive capability limits remain explicit. |
| Transfer policy and order | [Verification enum](../crates/midnight-core/src/transfer_verification.rs), [copy worker](../crates/midnight-engine/src/copy_worker.rs), [native Move](../crates/midnight-engine/src/move_native.rs), [transfer contract](developer/TRANSFERS.md) | Serial bounded copy, immutable full/quick policy, same-volume rename and per-file source removal. Neither quick mode nor native rename weakens identity/consent/publication checks. |
| Filter/drive/status | [Panel controls](../crates/midnight-ui/ui/panel-view-controls.slint), [filter presenter](../crates/midnight-ui/src/presenter/filter.rs), [drive information](../crates/midnight-ui/src/presenter/drive_information.rs), [folder totals](../crates/midnight-ui/src/presenter/panel_details.rs) | Filter icon/applied state, backend volume information and omission of unmeasured folder totals already exist. Their current native usability and qualification are separate from implementation presence. |


## Requirements and evidence backlog

Priorities retain the existing requirement IDs. P0 means an integrity prerequisite
before enabling a dependent action; P1 means functionality/reliability; P2 means
qualification or measured optimization. Owners below are the appropriate roles
for the next assignment, not permission to start unreserved edits or builds.

### Implementation, contract and performance work

| IDs / priority / kind | Verified basis and remaining work | Next acceptance / role |
| --- | --- | --- |
| VFS-01, PFS-01 / P1 / public API/SDK source integrated; native/host cutover open | Public [ABI 1.3](../filesystem-api/midnight-filesystem-plugin-api/src/abi.rs) and [operation family](../filesystem-api/midnight-filesystem-plugin-api/src/model.rs) now include optional prepared Rename alongside browsing/reads/storage and Activate/Refresh/ResolveText. Task 1 plus public docs are merged into D `e5cba5d`, with exact 33-file equality. [App composition](../crates/midnight-explorer/src/main.rs) still injects native MutationService/WatchService separately; Windows/Linux native 0.2 source is integrated in D `5baee471`. | Architect + Backend: retain the source-integration boundary while qualifying native Task 2/3 and later host routing. No new provider Rename capability/deployment or host cutover is claimed. Reject stale/foreign/unsupported targets without native/TEMP substitution; plugins share only public APIs and own tests. |
| TREE-01, ARCH-02 / P1 / implementation | [Tree contract](developer/TREE_PANEL.md) includes implemented bounded recursive expansion (`ce9eff3`, Windows UI 69/0 and overlapping Backend refined 11/0) and captured consumers; physical Linux desktop qualification/larger-than-model paging remain open. Production container discovery/attachment and archive watches remain separate gaps: [attach API](../crates/midnight-engine/src/providers/tree.rs) has test callers. Since 20 November flat panels open archives inside archives for the session, up to 4 levels: the enclosing mount stages the member as a verified lease that a [nested mount](../crates/midnight-filesystem-host/src/nested.rs) reads as its source, revalidating the enclosing chain on every read ([archives inside archives](ARCHIVE_FORMATS.md#archives-inside-archives)); inline branches do not expand archives. | Backend + Architect: qualify archives inside archives with installed workers (native → archive → archive → viewer, outer-source changes, concurrent solid readers), decide whether inline branches expand them, and add container attachment, archive watches and cancel/retry; qualify supported tree actions without removing unsupported-action guards. |
| TREE-01 / P1 / DirectoryLink maintenance implemented; Windows qualified | Parent-first loaded native-link refresh, exact acknowledged watch scope, physical-identity notification deduplication with separate logical aliases, and retarget-aware subscription/poll validation pass the Windows gate: 51 selected/passed, 0 failed/ignored, compile 0, zero drift. Disjoint groups: provider 10 + refresh 25 + watch 15 + real Windows junction 1. `build-green-result.json` and `build-green-runtime-result.json` under `_build/verification/backend-tree-link-refresh-20261113/` establish this scope. Prior RED 2/3/0 is historical. | Retain generation/exact Commit, failed-parent descendant suppression, cycle/retarget and logical-action safeguards; no write/ACL/elevation changes. Updated Linux-native fixture passes 1/0/0 on WSL/tmpfs with frozen overlays and zero drift. The separate local Linux UI gate passes 13/0/0; historical `269f3419` debug compilation and registered private Windows startup pass. The earlier exact-`1714d99` Linux 1/0/0 fixture does not qualify this updated source. |
| TREE-01 / P1 / cap128 source integrated; synthetic gate passed | [Cap128](#recursive-tree-cap128-gate) at `2b28cc7`, integrated by `02b2f2f`, passes 25 targeted functional cases, strict Clippy 0 and one opt-level 0 synthetic aggregate. Four timed fixtures complete with 8/80/80/79 publications; the preceding cap16 95/0/0 and failed aggregate remain historical. | Perf + Backend: retain cycle/retarget, exact Commit, cancellation and captured-action guards. No new baseline, release/native filesystem, UI paint or visible-smoothness qualification is established; the `c6c371bc` runtime predates both increments. |
| COPY-03 / P1 / implementation | [Metadata policy](../crates/midnight-core/src/transfer_metadata.rs) exposes ContentOnly and required exact modified time for up to 64 native regular-file roots. Since 20 November Linux Copy (with Follow links cleared) and Move transfer symbolic links as links, and Linux Copy and cross-volume Move give new entries the source's permission bits masked by the umask ([links](developer/TRANSFERS.md#links-in-copy-and-move)). Full metadata/directory metadata and Windows link-object copying remain unavailable. | Backend: define capabilities for remaining times/attributes/ACLs/xattrs/streams and postorder directory metadata; unsupported kinds fail before writes where knowable. Verify required values before publication and retained sources on failure; Windows link copying must preserve links without following referents. |
| TERM-01 / P1 / implementation | [Terminal service](../crates/midnight-core/src/desktop.rs) is launch-only and [Linux launch](../crates/midnight-platform-linux/src/desktop.rs) spawns a new process/TTY. The [reusable-session requirement](superpowers/specs/2026-11-04-terminal-sessions-design.md) remains open. | Backend: one unprivileged bounded broker with kernel-verified process/UID/lifetime identity; distinguish Idle/Busy/disconnected/unknown outcome and never replay uncertain commands. Qualify a concrete terminal; sudo owns authentication/expiry, with no stored password, policy change or keepalive. |
| FIND-01/02 / P1 / Open report unreproduced; durable regressions pass | The [validator](../crates/midnight-engine/src/find_worker.rs) compares raw path/kind/native identity and six revision words. The [current diagnostic and durable gate](#native-link-presence-and-find-gate) records 13/0/0 with the exact user fixture unchanged and no differing identity/revision, followed by 6/0/0 durable regressions with production Find unchanged. The original report's build and captured metadata are unknown; no false-stale cause or production fix is proven. | Backend + Build: if the report recurs, retain exact runtime/session identity and captured/current raw path, kind, identity and revision words before selecting a correction. Preserve replacement/stale-session rejection. UI Details is the query report, not a second file-Properties action. |
| NET-01 / P1 / implementation | Discovery and typed discovery Hot Paths exist; [Linux GIO](../plugins/midnight-filesystem-network/src/linux/gio.rs) explicitly aborts interactive password challenges. Live SMB/GVfs interoperability is a separate qualification gap. | Backend + Architect: host-owned ephemeral authentication/cancel flow with no credential persistence/logging. Cover stale/canceled auth, reconnect and provider removal; then qualify an owned live endpoint. |
| FILTER-01/02, UI-01 / P1 / targeted positive-input gate passed | The retained replacement-popup focus assertion passes. Native Filter 17/0, Filter library 2/0 and exact operation-lifetime 1/0 cover 20 distinct tests; exact-first replay is not added twice. Find geometry passed with unchanged source. Private engine/native UI startup subsequently passes. | Preserve verified input/focus/lifetime and source identities. The original shared hashing anomaly remains preserved/unattributed after a passing engine retry; full native/layout/platform acceptance is not closed by these focused tests or startup smokes. Retain real pointer/key, occurrence identity, history/clear and stale-token coverage. |
| SELECT-01 / P1 / fixture correction implemented; subset passed | [Flat](../crates/midnight-ui/tests/bulk_selection.rs) and [tree](../crates/midnight-ui/tests/tree_bulk_selection.rs) definitions match all-marks Ctrl+D and default-unassigned Ctrl+Shift+D in `2e0e554`; their recorded binary runs passed 1 and 8 tests. The earlier Linux CI failure remains historical and the combined current diagnostic failed elsewhere. | Build: finish the targeted both-pane/native/archive integrations and record exact source/artifact/commands/results. Preserve explicit files-only action/remap coverage and the wider native/layout/drag qualification requirements; this subset does not establish a full selection or remote CI pass. |
| CMD-02 / P1 / corrected Windows gate passed | Checkpoint `21c9c7a` records the six accepted UI files. Corrected gate: 37 passed / 0 failed / 0 ignored across five artifacts, zero source drift, completed 21:16:16 CET. Natural FocusOut/caret and explicit active-size-bit fixtures pass locally; initial 31/5 setup failures remain historical. Captured Custom Commands remain separate; the current private `937e9bfc` EXE has startup/snapshot qualification, with command workflows separate. | Preserve literal insertion, acknowledged occurrence identity, raw names, focus, stale rejection and shell-draft grammar. Build: retain the exact receipts and qualify remaining Linux/native command-workflow boundaries separately. Do not derive authority from displayed archive paths or execute inserted text; no remote CI fix is inferred. |
| STATUS-02, PFS-02 / P1 / terminal styling implemented; targeted UI tests pass | Completed partial totals retain `≈`, yellow warnings and details with normal metadata number styling; only actively unfinished totals are highlighted, even while another root runs. Earlier recorded size/model library tests, `size_errors` 2/0, `panel_warning_details` 4/0 and `tree_folder_sizes` 7/0 passed. | Preserve these guarantees through remaining UI fixes without extending those earlier receipts to changed source, including mixed running/finished, warning-completed and cancelled/incomplete states. Full native/layout/platform qualification remains in the UI acceptance row below; these software UI passes do not close it. |
| SCROLL-02, VIEW-PERF-01/02, MEDIA-03 / P1 / source integration accepted; native qualification open | Corrected Markdown is committed at `255fbdc`, with 72/0 and strict Clippy. Perf accepts source integration from 32 paired debug profiling passes with exact output equivalence: large-fixture request wall time falls about 23 ms → 11–15 ms, while per-request allocator peak rises 383,320/461,739 bytes and small-fixture p95 rises 0.038–0.298 ms. [Retained-seek report](verification/2026-11-12-retained-complete-page-seek.md) retains its dated XML/refill and native boundaries. | Viewer + Build: establish matching packaged-worker/native evidence. Native scrolling/deployment, release/frame pacing, process memory/RSS, DPI/viewport and scroll-back qualification remain open under coordinated leases. Matching deployment/interactive evidence is still required; backend wall-time measurements do not close these requirements. |
| SCROLL-02, VIEW-PERF-01/02 / P1 / native Source drag unresolved | The archived one-off Source probe completed 1,098 operations/732 exact replays without mismatches; it measures broker/codec work, not the user's 134-line C++ scrollbar input-to-paint path. Portable panel regression tests `3bee1dc` 4/0 and thumbnail correction `8e205c0` 14/0 do not qualify that separate viewer path. | Viewer + UI + Build/Perf: capture the actual native scrollbar gesture, dispatch, request, layout/render and presentation phases on an owned unchanged fixture; attribute the observed delay before choosing a fix. Preserve source identity, cancellation and bounded provider I/O; do not infer a rendering improvement from protocol replay. |
| LIFE-01 / P1 / source review + qualification | [Lifetime contract](developer/ENGINE_LIFETIME.md) and later service/mailbox/report/configuration reports supersede the first retirement plan's narrower status. Exhaustive closure remains unestablished. | Architect + Backend: reconcile each retained owner/task/channel/last-release path against current source; exercise first-poll/drop, blocked dispatch, shutdown and completed-unconsumed outputs with native-before-provider destruction. A blocked destructor retains real capacity; no weakening or unbounded cleanup fallback. |
| IO-01 / P2 / benchmark prerequisite + implementation | [Transfer contract](developer/TRANSFERS.md) and copy worker remain serial with bounded 256 KiB steps. [Same-SSD probe](verification/2026-11-10-ssd-copy-probe.md) is incomplete and no-go for enabling concurrency from that run. | Backend + Build: complete stable 1/2/4-stream full-verification measurements, then positive local-SSD endpoint classification and shared physical-device budgets. Unknown/HDD/network stay serial. Cover aliases, A↔B fairness, small/mixed/large files, UI p95/p99, failure/cancel and retained in-flight ownership before raising defaults. |
| PLAT-01 / P1 / implementation + native dependency | [Application platform selection](../crates/midnight-platform/src/lib.rs) rejects non-Windows/Linux targets; a separate [read-only Darwin provider](../plugins/midnight-filesystem-native-macos/IMPLEMENTATION_PLAN.md) already exists. | Architect + Backend + UI + Build: reuse that provider and implement separate macOS app services, secure loaders/containment, input/clipboard/windows/terminal, mutations, packaging and CI. Native hardware/runner evidence must establish supported OS/architectures; its absence does not block independent source work. |
| PLAT-03 / P2 / implementation | [Linux external drag](../crates/midnight-platform-linux/src/file_drag_drop.rs) explicitly requires X11/XWayland; native Wayland lacks the same-connection seat/data-device bridge. | Backend + UI: implement bounded raw-identity/admission/cancel/lifetime integration and qualify native compositors. Internal dragging and XWayland results do not establish native Wayland external transport. |

### Provider increments from the Architect review

<a id="native-rename-audit-gates"></a>

Public prepared-Rename API/SDK source is integrated at `e5cba5d`; native
Windows/Linux 0.2 source at `5baee471` from C `42d45def` (25 paths matching frozen `4eea72bc`);
Task 4A/4B core-host source at `02b2f2f`. These are included in `174c5fd`.
Native source integration leaves engine/UI cutover, installed-provider/native
runtime qualification and deployment open. The retained `516fc158` runtime
predates the new native plugins. The earlier Linux 38-case, Windows 21/1 → 22/0,
Task 4A 44-case and Task 4B preceding 79 per platform → focused two per platform gates keep their original scopes;
[exact freezes, failures and overlap accounting](verification/2026-11-13-checkpoint-evidence-history.md#native-rename-gate-history)
are retained separately.

The public [native protocol](../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md)
explicitly grants no mutation authority to read handles/revisions. Its existing
Refresh invalidates discovery TTL caches; public watch subscriptions/general change
events remain missing, rather than all invalidation being absent. Archive staging
writes host-owned private storage and does not enable archive mutation.

| Gap | Finite scope and required future evidence |
| --- | --- |
| Prepared actions / first VFS-01 Rename slice | Accepted: optional, versioned single-item, same-provider, same-parent prepared Rename. Explicit preparation captures identity before confirmation; it does not guarantee identity from listing time or change the namespace. Opaque source/parent authority retains raw basename, prepared identity/revision, bounded admission/cancel and worker-owned cleanup through refusal, stale delivery and shutdown. Public/native/core-host source is integrated as summarized above; [dated native gates](#native-rename-audit-gates) do not establish installed-provider qualification, deployment or the unimplemented engine/UI cutover. No-replace is the baseline; replacement must preserve current pinned/versioned/recoverable consent or remain Unsupported. No delete-first/copy/cross-device fallback. Public API/SDK and independent native plugins precede host cutover; no deployed native capability is established by these selected gates. Require compatibility/absent-capability/malformed-handle tests, native prepare/cancel/race/collision/raw-name/case-only/alias tests and installed flat/tree confirmation/queue/cleanup tests. Delete, metadata, transfer and watches stay separate slices. |
| Public watches/general invalidation / VFS-01 | Migrate existing [native tree watch scope](../crates/midnight-engine/src/tree_watch_scope.rs) through explicit provider subscription/revision/event/overflow contracts. Preserve bounded eligible loaded-branch observation, fair polling fallback where supported, dirty epochs through reconciliation and no implicit activation/authentication. Require expanded-empty growth, overflow, >32-branch fairness, late navigation/revocation events, worker cleanup and backing-archive revision invalidation checks. Existing explicit Network Refresh remains supported. |
| Preserved links / COPY-03 | Task 1's internal core/queue source is integrated in `174c5fd`. Queue retry **17/0/0**, **11.562 s**; strict core+engine `--lib --tests` Clippy exits **0**, **44.6691 s**, with zero source-freeze drift. Current unique functional evidence is **80 = 63 retained nonqueue passes + 17 queue rerun**; the earlier 80-case **77/3** remains historical, not 97 unique cases. Default stays `Reject`; native Preserve hooks remain `Unsupported`, with no UI enablement. Move/exact-time Preserve remains unavailable. No dependency or public API/SDK change. Remaining native requirements: A distinct no-follow link proof captures raw payload/kind/flags, exclusively stages and independently reads back the link representation, revalidates source and publishes safely without opening the referent. Link Move needs separate retained finalization authority; it cannot reuse a regular-file content proof. Require relative/absolute/dangling/cyclic links, junction/kind/privilege limits, Unicode/raw names and untouched external sentinels, plus mismatch/replacement/conflict/cancel/cleanup failures. Hardlink topology and broader metadata remain separate capabilities. Update 20 November: the Linux source now implements these hooks; the Copy/Move dialog sends `Preserve` for Copy with Follow links cleared and for every Move, and Move moves the link itself (renamed, or recreated and then removed across volumes; [links](developer/TRANSFERS.md#links-in-copy-and-move)). Windows still refuses `Preserve`, and exact-time Preserve remains unavailable. |

Proposed ownership: Backend owns public API/SDK/header, separate plugin/platform
implementations, host adapters and engine authority/proof integration; UI owns
capability presentation, drafts/confirmations and flat/tree UI regressions; Build
owns packaging/CI. Shared composition/`lib.rs` edits receive one explicitly named
owner. The Architect review itself was read-only; later source progress does not expand its recorded qualification.

### Qualification of implemented workflows

These are evidence tasks, not claims that the existing feature is absent or
broken. Commission only the missing pathway/environment after reconciling prior
receipts; injected failures, owned native probes, UI bridges and real application
gestures prove different boundaries.

| IDs / area | Evidence to retain | Remaining acceptance / role |
| --- | --- | --- |
| COPY-01/02/03, MOVE-01 / transfers | [Transfer contract](developer/TRANSFERS.md), [current full/quick/native Move evidence](verification/2026-11-10-transfer-verification-and-native-move.md), [panel batch](verification/2026-11-12-panel-interaction-polish.md) | Backend + Build: final integrated/release tests for same/cross-volume, merge/replacement/recovery, alias/source-change races, full/quick boundaries and corruption, exact required time, retry/cancel/cleanup. Real disk-full/disconnect coverage must be distinguished from injection and native Shell probes. Final Slint app → running Explorer drag remains separate from owned-window/Shell success. |
| TREE-01, FILTER-01/02 / captured action parity | [Per-action tree contracts/evidence](developer/TREE_PANEL.md), [bounded mailbox qualification](verification/2026-11-12-panel-interaction-polish.md) | Backend + UI + Build: installed native/archive workflows for each enabled consumer, both panes, root/descendant/mixed/hidden/stale targets and queue rejection. Native clipboard desktop interoperability, Linux desktop input and complete search/filter/sort/action parity remain separate. Do not reimplement consumers merely because old audits list them as guarded. |
| ARCH-01/04, HOTPATH-02, MEDIA-01/03 / archives and previews | [Archive API/limits](developer/FILESYSTEM_API.md), [streaming workers](verification/2026-11-09-archive-streaming.md), [cooperative thumbnail cancellation](verification/2026-11-10-archive-thumbnails.md), [native Hot Paths manager](verification/2026-11-11-native-tool-windows.md) | Backend + Viewer + Build: real-plugin full-app menu/shortcut/restart/restore/unavailable-source Hot Paths, large/ZIP64/solid/all-format paths, private-storage failure and release packaging. Qualify actual native panel scroll-back/cache reuse and stale provider revisions; protocol-3 installed worker checks do not cover every Linux/archive-panel desktop workflow. |
| FIND-01/02, HASH-01 / tools | [Find](developer/FIND.md), [captured Find actions](verification/2026-11-12-find-actions.md), [checksums](developer/CHECKSUMS.md) | Backend + UI + Build: current installed tool windows, roots/history, exact captured result actions, manifest restrictions, export/partial consent, stop/stale/shutdown and small-window behavior. Search inside archives under native roots, content regular expressions and Feed to panel are implemented since 20 November; archive search is verified only with a fake provider and still needs installed-plugin qualification. Find rooted in an archive panel and provider-wide checksum support require separate implementation contracts. |
| CMD-01/02/03, OPTIONS-02, VIEW-03 / persisted settings and commands | [Tree commands](verification/2026-11-12-tree-custom-commands.md), [Cmd scope](verification/2026-11-12-cmd-custom-commands.md), [viewer preferences](developer/VIEWER_PREFERENCES.md) | UI + Backend + Build: actual Add/Edit/Preview/Cancel/Save/restart, exact argv/cwd and selected/focused identity, icon lifecycle and terminal behavior. Cmd `_ARG` direct argv evidence does not qualify arbitrary CALL/builtin reparsing. Viewer manager recreation is narrower than an app restart. |
| DIALOG-02, META-02, SELECT-01, PANEL-02, NAV-01/02, NOTICE-02, THEME-02, ICON-02, DRIVE-02, STATUS-02 / UI polish | Embedded modal/focus/input source and corrected selection fixtures are in [current checkpoint scope](#current-checkpoint); earlier [panel/selection/palette evidence](verification/2026-11-12-panel-interaction-polish.md) retains its dated scope | UI + Build: qualify normal/320×200, desktop/touch, fonts/DPI/themes, input/focus/IME, resizing, X/Escape and captured callbacks. Ordinary modals stay embedded; independent tools retain native hosts. Preserve warning Copy/retry/close and the tested finished-partial-size presentation. Render matrices do not substitute for native/compositor checks. |
| VIEW-01/02, SCROLL-02 / viewers and comparison | [Viewer presentation](developer/VIEWER_PRESENTATION.md), [image limits](developer/IMAGE_VIEWER_API.md), [HTML contract](developer/WEB_VIEWER_API.md), [foreground evidence](verification/2026-11-12-foreground-scroll-resume.md) | Viewer + UI + Build: native text/hex/source/Markdown/image/PDF/HTML opening/navigation/search/copy/zoom/font/persisted settings, exact user fixtures, native/archive content and text/image comparison. Qualify Linux frame pacing, physical IME, native Chromium Wayland and configurable browser key arbitration separately; preserve bounded page selection and inert content. |
| NET-01, PLAT-02 / native environments and permissions | [Network contract](developer/NETWORK_FILESYSTEM.md), [system targets](SYSTEM_REQUIREMENTS.md), [directory authorization](verification/2026-11-11-directory-authorization.md) | Backend + UI + Build: controlled live SMB/GVfs/FUSE, Windows 10, physical Linux desktops/distribution packages, GPU/driver/filesystem and multilingual/RTL/IME evidence. CSC warning/UI and Engine mock gates pass, but actual privileged CSC access, accepted UAC/polkit prompts and physical Linux desktop workflows remain unqualified. The original recursive symlink fixture is verified 1/0/0 under WSL/tmpfs at `1714d99`, with compile/test exit 0. Record exact environment, reduced-RAM measurements and artifact notices; WSL/compilation is narrower evidence. |
| CI / complete current validation | [Retained remote boundary](#retained-remote-ci-boundary): exact-head `9176272` is **20/20 GREEN**. The historical `174c5fd` result is **not green**: both format checks failed 13 hunks/3 files. Root's three-file format-only correction was committed/pushed as `cdc9226`; its exact-head remote CI remained pending in that record. Main is now pushed/verified through `4f27e16` per Root, without a newer remote-CI GREEN receipt here. [Earlier failures and correction provenance](verification/2026-11-13-checkpoint-evidence-history.md#retained-remote-ci-boundary) retain their dated scopes. HTML 135, COPY 80, C native-plugin and WSL/software gates remain selected local evidence; the current lane adds only its [named targeted/build/startup scope](#current-windows-launch-qualified-runtime-937e9bfc), with full suites not rerun. | Build + Root: obtain fresh frozen-source targeted/workspace and remote receipts. Structural workflow validation, selected local gates, C native-plugin gates, WSL fixtures and qualified `937e9bfc` startup do not establish full-suite/desktop success. The earlier cap16 functional 95/Clippy 0 and failed aggregate remain historical; the later integrated cap128 passes only its recorded functional 25 and single synthetic aggregate. Native provider/host Task 4, viewer performance and manual permissions remain separate. |
| Dependencies / release inventory | [Dependency policy](DEPENDENCY_POLICY.md), [third-party inventory](THIRD_PARTY.md) and distribution notices | Every role + Documentation + Build: review actual pinned direct/transitive/native/test/build dependencies and enabled features on changes, verify free intended use/distribution and required notices/source offers. No trial/paid-only path or silent project-license change. No dependency changed in this documentation pass. |

### Retained extension contracts

The broader design still records these capabilities. They require separately
scoped contracts and ownership before enabling controls; existing intentional
bounds are not regressions. Candidate optimizations are not silently promoted
into mandatory product features.

| Existing IDs | Next concrete contract / evidence |
| --- | --- |
| ARCH-03 | Archive write/auth/export lifetime, outer-container staging/publication, independently verified members and ephemeral credentials. More formats/methods/multivolume support need specific requirements and fee-free dependency review; no automatic RAR obligation. |
| REMOTE-01, REG-01 | Typed remote endpoint/auth/read/watch/seek/mutation capabilities and a distinct Windows Registry key/value model. Network discovery is not FTP/SFTP transport; Registry values are not ordinary file-copy sources. Depend on VFS-01 and bounded, independently verifiable destination contracts. |
| VIEW-01, MEDIA-02, HASH-02 | Any whole-file selection, animation, wider PDF/provider comparison/editing, additional codecs/algorithms/hash-plugin API or persistent history needs a bounded contract and actual license/feature review. Existing page-local copy, still-image/codec and five-hash limits remain disclosed. |
| PERF-01 | Single-file parallel blocks, GPU hashing/decoding, BLAKE3 policy substitution, parallel mutations and universal process isolation remain proposals/evaluations. Require acceptance, measured benefit and ownership/security contracts; do not replace current SHA-256 merely because a microbenchmark is faster. |

## Corrections to stale staged references

These notes qualify unowned/historical documents; this batch does not rewrite
their evidence or authorize edits to another role's files.

| Reference | Current correction / unresolved handoff |
| --- | --- |
| [5 November stages](superpowers/plans/2026-11-05-follow-up-stages.md), stage 3 and opening sequence | Whole-batch-before-delete and unconditional hash/copy wording is superseded by per-file Move and the accepted full/quick/native-rename policy. Historical stage order does not suspend the current parallel seven-role assignments. |
| [Implementation backlog](IMPLEMENTATION_BACKLOG.md), tree introduction/consumer text | Native Delete/Rename/metadata, clipboard, focused F4, checksums, Properties/context menus, Make File List and native comparisons have captured paths. In particular the later sentence calling directory comparison guarded is stale; root-level native comparison is implemented. Generic provider writes, archive watches, container attachment, archives inside archives in inline branches and complete native qualification remain open; flat panels open archives inside archives since 20 November. |
| [11 November gap audit](verification/2026-11-11-open-requirements-audit.md), tree/dialog/media/network rows | This is the named older baseline. Later reports/source add captured consumers, protocol-3 cooperative archive staging cancellation and discovery Hot Paths. Blanket native-modal migration is superseded; interactive Linux auth remains a gap. Preserve the audit as history. |
| [Feature status](FEATURE_STATUS.md), Dialogs row, and [dialog ergonomics](developer/DIALOG_ERGONOMICS.md), native-modal examples | Corrected; checked on 18 November: both now describe the embedded modal migration of `2e0e554`. Operation/copy/filter/Hot Paths/file-list/notice forms are modal popups embedded in the main window with the focus/input gates above, independent modeless tools keep native hosts, and no native host is owner-modal. The positive-input gate passes the retained-focus assertion and selected boundaries; private startup and shared engine retry pass their recorded scopes; the initial shared hashing anomaly is preserved/unattributed and broader native qualification remains open. |
| [Initial retirement plan](superpowers/plans/2026-11-12-runtime-independent-retirement.md) and LIFE-01 backlog ledger | Later service/checksum/command/mailbox/report/configuration work exists. The backlog states the current 329 slots and keeps the older 308 figure as history; the [current lifetime contract](developer/ENGINE_LIFETIME.md) states 329 payload slots and eight reusable waiter tokens, which an engine test checks against the code. Architect/Backend must reconcile the current ownership ledger before closure, rather than declaring all retirement work missing or complete from checkboxes. |
| [Tree Move contract](developer/TREE_PANEL.md#move-and-transfer-reconciliation) | Corrected in this scope, including Copy/Paste wording: copied files use the frozen Full SHA-256 or explicit quick size plus first/middle/last 64 KiB policy; same-volume native rename reads/hashes no contents. Source identity, conflict consent, per-file removal and tree reconciliation guarantees remain intact. |
| [Transfer service](developer/TRANSFERS.md), ordinary Move/metadata descriptions | Corrected in this batch after source inspection: native same-volume rename precedes content proof; copied-file removal follows the immutable full/quick mode, with metadata/identity guards retained. Full SHA-256 remains the default, not an unconditional requirement for every Move. |
| [Worker SDK](developer/WORKER_SDK.md), startup/version descriptions | Corrected against package/source: worker SDK 0.2.0 accepts legacy MWS1/MWR1 and explicit MWS2/MWR2 resource policies. Thumbnail API remains 0.1.0 while its SDK is 0.2.0; released viewer API/SDK 0.1.0 is a separate package family. No dependency or released package bytes were changed. |
| [Panel qualification final handoff](verification/2026-11-12-panel-interaction-polish.md) | Its final “ready for main” and paused-backlog text describes that handoff. The current Orchestrator assignment establishes integration and resumed scoped work; it does not expand the report's executed test/native/CI scope. |
| [Bulk selection fixtures](../crates/midnight-ui/tests/bulk_selection.rs), [tree bulk selection fixtures](../crates/midnight-ui/tests/tree_bulk_selection.rs) | Former expectations caused the recorded Linux CI failure. Fixture source is corrected in `2e0e554`; the completed-artifact diagnostic passed 1 flat and 8 tree tests. The combined result failed elsewhere; no all-integration or remote CI pass is inferred. |
| [README Controls](../README.md#controls) | Corrected under the explicit scope extension: Ctrl+D clears every mark, including inline descendants; Ctrl+Shift+D is unassigned by default. Ctrl+A/Ctrl+Shift+A and Ctrl+I/Ctrl+Shift+I retain the verified files-only/top-level-all scopes. |
| [Windows directory rename refusals](verification/2026-11-18-windows-directory-rename-refusals.md), Scope and Limitations | Its statement that recycling has no Retry/Skip/Cancel decision is superseded by the 20 November work: the delete worker (`crates/midnight-engine/src/operation_delete.rs`) offers a refused Trash or Delete target Retry, Skip, Skip all errors and Cancel. Its statement that the Shell error is shown with its code is superseded by `9d25722e` and `4ab0b16f`: 0x80270021 and 0x80270027 are reported as `ResourceBusy` with a readable message that keeps the code and the path and names this application's panels, expanded folders and viewers as possible holders. The remaining limit is TRASH-01 in the [backlog](IMPLEMENTATION_BACKLOG.md). |

## Evidence required to close work

An owner handoff supplies exact files and source/build identity, behavior/capability
scope, the actual commands and test counts including ignores/failures, environment,
artifact paths under `_build/`, native versus substituted boundaries and remaining
requirements. Behavior changes receive targeted failure/cancel/race tests at direct
integration boundaries; full validation follows the planned integrated batch.

Documentation closes an implementation claim only when source/API behavior matches
the latest requirement. Native/performance/platform claims additionally need their
own evidence. A completed review or local test group does not close the full goal;
unavailable native hardware is a qualification dependency, not a reason to stop
independent authorized implementation.
