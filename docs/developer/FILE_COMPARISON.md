# File comparison and manual input

`Ctrl+Shift+C`, F10's Compare files and the shared toolbar invoke the same comparator.
The action remains available without a file target: it can ask for both inputs.

## Planned comparator settings

Configurable comparator associations remain **in development**. The
[design](../superpowers/specs/2026-11-13-comparator-associations-design.md),
[UI wireframes](../superpowers/specs/2026-11-13-comparator-associations-ui.md) and
[implementation plan](../superpowers/plans/2026-11-13-comparator-associations.md)
cover ordered BOTH-basename routing to existing built-ins or a trusted external
program. Configured backend routing now has a targeted **77 passed / 0 failed /
2 ignored** gate, with strict Clippy/format0, empty source drift and unchanged app.
It consumes the 27-file freeze with 30 newly authored tests and overlaps earlier
comparison checks. The canonical P3 packet is promoted (104 sources/159 source-derived selections).
The selected 39 integration cases are 38 passed/1 Options-init-focus failure.
P3 units initially20/1; the corrected draft exact selector now passes once, with
the other 20 not rerun. Native comparator4/0 is scoped below; no full159/native
input or ReadAt acceptance follows. See the [current checkpoint](../ACTIVE_WORK.md#current-checkpoint).

The corrected faithful native comparator fixture ran **4 actual Windows cases /
0 failures** (`comparator-d6ce28b8eb`): raw parent/argv/cwd, NotStarted cleanup,
preworker cancellation/two session credits/no child, and non-Unicode Automatic
fallback. Components were unchanged and every case left remainingOwned[]. This
closes the old fixture settings-timeout for those cases; historical failures remain
recorded. Fixture 16 blocking/6 resident/10 transient-holder qualification does not
implement the production-pool repair or supply native ReadAt counts.

The current pairing and ownership behavior below remains implemented.

## Comparators in Options (in development)

This help describes the new source workflow. Use it after the coordinated UI and
application gates pass; it is not yet a qualified feature of the running app.

1. Open **Options → Comparators**. Add a rule and set **Enable this rule**.
   Disabled rules stay saved and are skipped during matching. Move a rule up or
   down to change priority; the list also supports **Alt+Up / Alt+Down**.
2. Choose **Wildcards** or **Regular expression**, then enter the pattern and
   optional case-sensitive setting. Wildcards support `*` and `?`, `;` between
   masks and `|` before exclusions. A regular expression is searched in each
   basename; use `^` and `$` for anchors. **Both basenames must match the same
   rule.** The first enabled matching rule wins.
3. Choose **Automatic**, **Built-in text**, **Built-in image** or
   **External program**. The fixed final **Other regular files → Automatic** row
   handles pairs with no matching override. Automatic uses the current built-in
   decoder assignments, then text for other regular files, and bytes for pairs that
   are not text or not two images (see Byte comparison). This fallback does
   not launch an operating-system file association. The read-only built-in types
   list describes the current session; plugin changes awaiting restart do not
   update that session's decoder coverage.
4. Select **OK** to validate and save the Options draft. **Cancel**, Escape or
   closing Options discards the draft. Disabled rules must also be valid. New
   comparison sessions use the accepted settings; existing sessions retain the
   settings captured when they opened.

For an external comparator, choose an absolute native executable. Enter **one
literal argument per row**, including a complete `{left}` row and a complete
`{right}` row. For example:

| Argument row | Value |
| --- | --- |
| 1 | `--diff` |
| 2 | `{left}` |
| 3 | `{right}` |

Only whole arguments equal to `{left}` or `{right}` are replaced by file paths;
`--left={left}` stays literal. Arguments are not shell-split or appended
automatically. Quotes, spaces and empty rows are literal argument values.
**Argument help** previews arguments without running the program. An optional
absolute **Working directory** overrides the default executable parent directory.
The external program receives the original mutable files and may edit or delete
them. A failed explicit external launch is reported instead of silently choosing
a built-in comparator; an unknown launch outcome must be resolved before retrying.

## Pair resolution

`Presenter::file_compare_launch` captures immutable `FileCompareSource` snapshots,
never enumerates selections or accesses files. Two active-panel selections retain
their visible order. Otherwise File A represents the left panel and File B the
right. An explicit opposite selection wins; with none, the worker resolves a unique
exact native basename in the captured visible listing. An unrelated opposite cursor
is never substituted. Loading/unstable selection, parent, directory, virtual/archive
or ambiguous rows do not supply implicit native paths. Plugin preferences must be
loaded before opening a session so disabled providers cannot be bypassed.

`midnight-compare-engine/file_choice.rs` preserves the unambiguous part of a pair
and returns `Output::ChooseFiles(FileChoice)` for everything else. Empty paths mean
unknown inputs. The session stays alive for explicit `Input::Compare([PathBuf; 2])`
or `Input::Browse { side, paths, owner }`. Invalid paths, missing/nonregular files or
comparison errors return the form with both draft paths intact. Native paths are
absolute and literal: no shell, quotes, expansion, cwd inference or writes.

## Ownership and threading

Production composition supplies `Service::with_native_capture` with the shared
installed-native-filesystem callback used by F3 and thumbnails. Both automatically
resolved and manually supplied inputs are captured on the comparator worker. A
provider error returns to the input form; it never tries a direct native opener.
The callback receives live cancellation, checked again before content use. Session
capacity remains reserved until an in-flight callback returns after cancellation.
The standalone `Service::new` constructor retains its default capture behavior for
embedding/tests; the application explicitly uses the injected provider constructor.
See [native media verification](../verification/2026-11-08-native-media-capture.md).

The app injects its existing `Arc<dyn PathPickerService>` into `Service`; it also
provides the existing platform-specific `native_path_picker_owner` to the UI.
Windows uses its Common Item Dialog and Linux the desktop file chooser portal.
Only the backend calls these implementations. The comparator permits two sessions,
one worker and one picker at a time per session, with bounded input/latest-output
mailboxes. Closing cancels both the comparison and the picker's shared cancellation
token. Normal picker cancellation retains the draft and returns to the form.

The form disables editing, browsing and Compare while a request is pending, but
retains Cancel/native close. This prevents an old picker result overwriting newer
drafts. Selection results cannot be replaced by resize/status traffic: the choice
loop consumes geometry silently until another explicit user request. Retry retains
the latest viewport geometry/revision. Large file/selection cleanup remains on the
worker. The UI stores original PathBuf values separately from lossy display strings;
an unchanged field keeps the exact native bytes/UTF-16. An edited field explicitly
supplies a new Unicode path. No MAX_PATH buffer is introduced.

## Reading, fonts and input

Text is read in 256 KiB blocks and image sources by the restricted image viewers;
every read is admitted by the process-wide I/O gate. The application installs
`Service::with_storage_hint`, which answers on the worker from the installed native
filesystem's storage kind of each file's folder: only a positively identified local
SSD is admitted as shared I/O, so a long copy cannot hold back such comparisons.
Unknown, rotational and network storage, and a service without a hint, stay
exclusive. Installed image viewers are discovered by the first comparison that needs
them (status *Checking installed image viewers…*) and kept for the service; opening an
image session still revalidates its package. A failed discovery is not kept: text
comparison continues, the failure is shown on the status line and added to the message
of a comparison that then fails (for example two images read as text), and the next
comparison discovers again.

Text must be UTF-8, or UTF-16 with a byte order mark. Tab, line breaks, vertical tab,
form feed (page breaks in sources) and escape (coloured logs) are text; the last three
are shown as the control pictures ␋, ␌ and ␛. The explicit bidirectional embedding,
override and isolate controls (U+202A-U+202E, U+2066-U+2069) are shown as escapes such
as `\u202E`, so neither file can reorder its displayed lines; the diff still compares
the raw lines. NUL and other control characters mark the file as binary.
Text comparison admits files up to 32 MiB, with at most 1,000,000 lines and lines up
to 16 KiB (`crates/midnight-compare-engine/src/text.rs`; the line limit bounds the
shaping of one painted row). A pair it does not admit (binary content, another
encoding, or a limit) is not refused: it is compared as bytes, and the summary names
the reason, for example *File A: not UTF-8 or UTF-16 text (binary or another
encoding); compared as bytes*. A file over 32 MiB is not read as text at all, and a
file whose first 256 KiB hold a control byte is refused after that first block.
Reading a large file reports its progress (*Reading File A: 12 of 30 MiB…*), then
*Matching lines…*; both stop at the next block or alignment step after Escape or Close.

The installed fixed-pitch families are scanned on a `comparator-fonts` thread, started
when a text comparison begins reading; the scan reads every installed family and can
take seconds on a loaded machine. At most one scan runs per service: a comparison that
starts while it runs, in either window, waits for the same scan. Before its first frame
a comparison resolves the preferred family (case-insensitive match, else Consolas, else
generic monospace): against the kept catalog when the service has one, otherwise by
checking only those one or two families with the catalog's own name filters and
fixed-pitch test (`midnight_viewer_render::fixed_pitch_family`, one face load per
installed candidate), so even the first comparison of a session paints in the resolved
family at once. The scan leaves its catalog in each waiting session's inbox and queues
a wake, which wakes the worker to publish the chooser list; the chooser stays disabled
(*Loading fonts…*) until then, and the worker does not poll while it waits. The reply
never waits for room in a queue, so a window whose worker is busy with a full queue
cannot delay the other window's chooser list, and it reads its own reply once it has
taken its inputs. Each reply names the request it answers: a late reply to a comparison
that failed before it arrived is dropped and cannot end the next comparison's wait. A
completed catalog is kept for the service. Closing a comparison cancels the scan once no
other comparison waits for it; a cancelled scan keeps no partial catalog. A scan that
fails, or panics, answers every waiting comparison with no catalog: it keeps its family
without a chooser list, and the next comparison scans again. Whole-input identity
verification starts once the first frame has been taken and publishes its verdict in a
new frame, so the chooser list can arrive before, with or after the verdict; every frame
carries the latest verdict and, once it has arrived, the chooser list.

Arrow keys move exactly one row (`Input::Lines`), Page Up/Down move by the rows that
were fully visible (`Input::Pages`), and wheel distances are converted with the painted
row height, so paging neither skips nor repeats lines at any font size. Images pan 24
logical pixels per line and one viewport per page.

An input that follows a quiet interval is sent to the worker at once. Inputs that
arrive within 16 ms of a sent batch wait and go together, so continuous input reaches
the worker as at most about one batch per display frame; inputs the full session queue
(32 entries, `INPUT_QUEUE`) refuses (`Refused::Full`) are retried every 16 ms until it
takes them. A worker that has stopped after an internal error refuses every input
(`Refused::Stopped`): the window drops what waits for it and arms no timer. The UI
keeps at most 64 waiting inputs. Repeated line, page and scroll steps accumulate. State
inputs (geometry, image mode, font, Fit or Actual pixels, and a file-choice request)
are never dropped: a newer one replaces the queued one of its kind and is queued after
the navigation that preceded it. A full queue therefore always holds navigation (scroll,
line and page steps, seek, difference and zoom), and sheds the oldest such input first.
An idle worker blocks on its input queue instead of polling, also while the font
catalog loads. Before each blocking receive it reads its inbox under the lock that
`Session::close` and the scan's reply also take, so closing the window stops the worker
and a catalog reply reaches it even when the full queue refused the Close input or the
reply's wake: the worker then still has queued inputs to take, and it reads the inbox
again before it blocks.

## Byte comparison (hex view)

A pair is compared as bytes when text comparison does not admit it (above), when
Automatic routing finds an image opposite a file that is not one, or a document (PDF),
and on request: the **Hex** button of a text comparison switches between its lines and
its bytes, and back. The button is checked and disabled for a pair that can only be
compared as bytes. An explicit *Built-in image* rule still reports a pair it cannot
decode instead of comparing bytes, and image pairs keep their image modes.

Both files are shown side by side in rows of offset, hex bytes and characters (`·` for
bytes outside printable ASCII). A row holds 16 bytes when the pane is wide enough, else
8 or 4; resizing keeps the first visible offset. Rows that hold a difference are tinted
like changed text rows, and the differing bytes themselves, in the hex and character
columns, are marked more strongly. A byte differs when the files hold different values
at its offset or only one file is that long, so files of different lengths compare; a
**region** is a maximal run of differing bytes. The summary on the status line gives the
number of differing bytes and regions (or *Bytes identical*), the first difference as hex
and decimal offsets, both sizes and the first visible offset.

The worker scans both captured files from the start, with at most one 256 KiB read of
each file per turn (the native filesystem's largest read) and one non-queuing admission
of the process-wide I/O gate per turn, like whole-input verification. Input is handled
between turns, so the view scrolls while the scan runs; the summary shows *Comparing
bytes… 37%* and the counts found so far, repainted at most every 250 ms. Escape or Close
cancels the scan with the window, and a changed file fails the comparison like any other
read. The tail of the longer file is counted without being read. For a pair compared
only as bytes the completed scan, with both captured revisions validated, is the
whole-input identity verdict (one pass, no second read); the Hex view of a text pair
keeps the text's verdict.

Memory is bounded: the scan holds two 256 KiB blocks, exact 64-bit counts and an index
of region starts for navigation with at most 262,144 eight-byte entries. When a pair has
more regions, the index keeps only starts at least a doubling spacing apart, so nearby
regions share one navigation stop, and the summary says *regions closer than N bytes
share a stop*. Painting reads only the bytes around the visible rows (three screens, at
most 12 KiB per file) through the I/O gate.

Difference navigation uses the keys and buttons of text comparison: Next and Previous
difference (F7, N, Alt+Down or Alt+Right; Shift+F7, P, Alt+Up or Alt+Left) move to the
next or previous row that starts a region and stop at the first and last one; Alt+Home
and Alt+End go to the first and last region. A region the scan has not reached yet is
shown as soon as it is found (*finding the next difference…*), and Alt+End waits for the
end of the scan. Arrows, Page Up/Down, Ctrl+Home/End, the position bar and the wheel or
touchpad (with the same gliding and momentum as text) scroll by rows. The hex view has no
Find. Tests: `midnight-compare-engine/src/binary_tests.rs`,
`midnight-compare-engine/src/capture_bytes_tests.rs` and
`midnight-compare-ui/tests/hex_comparison.rs`.

## Large text comparison

Lines are interned to integer ids first, so the alignment deadline covers only the
alignment. The optimal (Myers) alignment gets 750 ms. When Myers gives up at that
deadline (many scattered changes in a large file), lines that occur exactly once in each
file and keep their order anchor a second pass (their longest increasing subsequence, as
in patience diff) that runs Myers only between neighbouring anchors, with another
750 ms. A gap Myers gives up on after that, or a pair without such lines, is shown as
one changed block, and the status says that line matching timed out. Whether Myers gave
up decides, not the clock afterwards: an alignment it finished is kept, and a gap it
resolves without a search (one side empty once the common ends are trimmed) stays exact
after the deadline. No pass pairs unequal lines as equal.

Both passes collect consecutive changed lines as one group while they run, at a constant
cost per Myers step. A group that only deletes or only inserts then slides through the
equal lines around it, up and then down as far as they repeat its lines, and joins a
group it reaches, so that a change Myers split reads as one group; a group that changes
both files never moves. Groups move by at most twice the lines of both files in all, so
this stays linear. Collecting, sliding and building the rows all check Escape and Close
between bounded steps. The retained text is one string and a `u32` line end per line for
each file, and 12 bytes per aligned row (`text_align.rs`, tests in `text_align_tests.rs`
and `text_tests.rs`).

Measured on 2026-11-20 with synthetic logs of numbered 70-byte lines (a standalone
build of this alignment code and of the previous `similar::TextDiff` pipeline) on a
shared 32-CPU build machine with a load average of 7 to 13. Times cover the alignment
and the retained rows after reading; memory is the peak beyond the two texts.

| Per file | Lines | Changed lines | Previous, optimized | Current, optimized | Current, debug | Memory, previous → current |
| --- | --- | --- | --- | --- | --- | --- |
| 16 MiB | 238,828 | 2 | 198 ms | 72 ms | 420 ms | +65 → +16 MiB |
| 32 MiB | 477,686 | 2 | 473–523 ms | 169 ms | 855 ms | +131 → +32 MiB |
| 64 MiB | 955,305 | 2 | 996 ms | 453 ms | not run | +282 → +64 MiB |
| 4 MiB | 59,706 | 10% (5,392 groups) | 520 ms | 516 ms | 886 ms | +4 MiB |
| 32 MiB | 477,686 | 1% (4,793 groups) | 919–958 ms, out of time: 2,397–3,596 groups | 688 ms, all groups | not run | +131 → +32 MiB |
| 16 MiB | 238,828 | 10% (21,324 groups) | 833 ms, one changed block | 865 ms, all groups | 1,347 ms | +65 → +16 MiB |
| 32 MiB | 477,686 | 10% (42,846 groups) | 967 ms, one changed block | 1,012 ms, all groups | 1,895 ms | +131 → +32 MiB |
| 64 MiB | 955,305 | 10% (85,850 groups) | 1,098 ms, one changed block | 1,329 ms, all groups | not run | +284 → +64 MiB |
| 32 MiB | 477,686 | unrelated files | 956 ms, one block | 1,066 ms, one block | 2,270 ms | +131 → +82 MiB |
| 64 MiB | 955,305 | unrelated files | 1,468 ms, one block | 1,442 ms, one block | not run | +282 → +164 MiB |

These measurements predate the development profile's optimization level 2 for `similar`
and this crate (`Cargo.toml`); the debug app (`_build/debug`) now runs them optimized.
Reading, UTF-8 validation and the control check add to these times (they are linear;
the control check reads eight bytes at a time). 32 MiB and 1,000,000 lines per file keep
the worst case near one second (two in a debug build) and the peak memory near two and a
half times the texts; 64 MiB pairs would still align in under 1.5 seconds, but at twice
the memory. Larger pairs, which a text view would not make readable, are compared as
bytes with bounded memory.

The table predates the collection and compaction described above. They were measured
on 2026-11-20 in the project's Linux build container (test profile, which builds this
crate and `similar` at optimization level 2) on the same shared machine, with a case
from review: File A of 250,000 blocks `U<i>`, an empty line and `X<i>`, and File B the
same with `Y<i>` and one more empty line (1,750,000 lines in all). Myers gives up, and
the anchored pass emits about a million operations. It aligns in 1.0 s
(`many_changes_between_anchors_align_in_bounded_time`). The previous code, which merged
those operations through `similar`'s `Compact` with a vector insertion or removal each,
took 290 s, and a cancellation requested after 30 s did not stop it.

## Presentation and tests

The same native comparator window hosts the File A / File B form and the resulting
text, image or byte comparison. It retains main-window placement/size, native resizing/moving,
Escape and the close button. Each input has Browse; Enter submits both nonempty
fields. Short forms center within the window, scroll at 320×200 and keep Compare and
Cancel outside the scrolling area. File fields own their editing/navigation keys.

The main branch's wake-driven path avoids idle polling on a platform with an
event-loop proxy. A focused file field can still blink its text cursor. When the worker stores an output and no earlier wake is
outstanding, its `Mailbox` calls the wake that `Service::open_with_wake` installed; the
window's wake schedules one `output-ready` callback on the event loop
(`slint::Weak::upgrade_in_event_loop`). That callback's `Mailbox::take` ends the
outstanding wake before it takes the output, so a burst of outputs costs one callback
and no output is left without one. The wake holds only a weak window handle: a worker
never keeps a closed window alive, and a callback that runs after the window is gone
does nothing. Changes of the content width or height or of the window scale
(`1px / 1phx`) raise `geometry-changed`, and the window sends one `Resize` when the
physical geometry changed, also for a DPI change at the same logical size. Only the
input batch timer remains, and it runs only while inputs arrive or wait for a running
worker. On a platform without a cross-thread event-loop proxy (the headless
integration-test platforms) the window polls its mailbox every 16 ms instead.
`midnight-compare-ui/examples/idle_wakeups.rs` opens a comparison in a real window and
prints the event-loop wakeups while it is idle.

`midnight-compare-ui/src/wake_tests.rs` runs the wake path on a platform with an
event-loop proxy and a frozen clock: outputs are presented by wakes alone; idle windows
in the file form, and a presented comparison once its font catalog has arrived, arm no
timer and cause no wakeup in a simulated second; inputs held behind a full session
queue are retried until delivered; a resize and a DPI change each send one `Resize`;
inputs for a worker that stopped after an internal error are dropped without a retry
timer; and closed windows are released while their worker still runs. Waiting for the
installed fixed-pitch font scan dominates its run time. Slint keeps that proxy
process-wide, so it is the only test in that binary that installs a platform.
`midnight-compare-engine/src/mailbox_tests.rs` covers the wake gate, including a race of
`take` against a concurrent output, and `capture_tests.rs` the two refusal reasons of
`Session::send`, the inbox (a close or catalog reply that a full queue refused, and a
late reply to an earlier request), a window with a full queue next to one waiting for
the same scan, a failed earlier comparison's reply arriving while the next one
opens its files, and a catalog that arrives after the whole-input verdict and keeps it.

Image comparison exposes **Side by side**, **Swipe** and **Onion skin** buttons.
Their primary style and accessible checked state reflect the requested mode.
The image composition continues to use the engine-acknowledged rendered mode:
the previous composition stays visible until the new result is ready. Fit/zoom
and divider/opacity controls occupy the next row; Close and content focus remain
reachable in compact windows and at large font sizes.

Five selected headless UI/theme checks plus visual review and scoped quality
checks passed in the 17 November branch handoff. They cover pointer selections,
requested/acknowledged mode separation and adjacent layouts; they do not qualify
the entire ui_polish target or a new application. A prominent green identical
result still requires an input-bound whole-input typed backend verdict; neither
status text nor equal image dimensions/pixels is a substitute. A separate work-branch selection recorded 38 passes and one Options initial-focus failure.
The diagnostic handoff records OK focused while the expected pattern is visible.
The corrected draft exact selector passed once after initial 20/1; the other 20
were not rerun. Prior 55 unit passes retain their own frozen source, and no full159
or native input qualification follows. See the
[current checkpoint](../ACTIVE_WORK.md#current-checkpoint) for exact receipts.

The corrected faithful native comparator fixture ran **4 actual Windows cases /
0 failures** (`comparator-d6ce28b8eb`): raw parent/argv/cwd, NotStarted cleanup,
preworker cancellation/two session credits/no child, and non-Unicode Automatic
fallback. Components were unchanged and every case left remainingOwned[]. This
closes the old fixture settings-timeout for those cases; historical failures remain
recorded. Fixture 16 blocking/6 resident/10 transient-holder qualification does not
implement the production-pool repair or supply native ReadAt counts.

The work-branch appearance handoff defines the contract for Viewer and Compare.
Merged paint and native input validation remain open. Application UI font is separate
from document font. Frontend apply_appearance consumes actual applied/preview Theme,
density and UI font scalars for existing/new windows; backend restore_palette accepts
foreground/background/selection/accent only. Accepted revision is not painted/native
acknowledgement. Bounded latest requests, A→B→A, rollback and frame-generation fences
must preserve source/layout/position without recapture, relaunch or source I/O.
Source/Markdown semantic roles must recolor active/cached content, gutters and raster;
Office/author HTML CSS and original image/PDF pixels remain content-owned. Reuse
supported UI controls and preserve input/accessibility; no SDK expansion is selected.

Tests cover existing automatic Unicode/raw-native pairing, partial physical order,
empty/ambiguous/pending panel inputs, repeated invalid paths and retry, unchanged
source contents, picker thread/owner, cancellation, invalid picker results, raw path
presentation, real Slint field input, busy-state duplicate prevention and compact
rendering. See `midnight-compare-engine/src/file_choice_tests.rs` and
`midnight-compare-ui/tests/file_choice.rs`.

Salamander reference: `src/plugins/filecomp/dialogs.cpp` (`CCompareFilesDialog`) and
`src/plugins/filecomp/lang/lang.rc` (`IDD_COMPAREFILES`). The two-path/Browse behavior
is adopted; no C++ implementation, artwork or MAX_PATH restriction is copied.

The accepted opening-palette amendment adds host `Session::palette_revision()`
and `WindowSession::palette_revision()` getters of the current accepted revision.
They remain readable after close/disconnect, perform no admission or mutation and
are not paint acknowledgements. Frontends merge the four actual app colors before
open, then seed opening fences from the real getter before publishing/consuming
the session; `restore_palette` is for later changes. Preserve document fonts and
typed terminal ExternalStarted/Unknown outputs. Do not hardcode a universal initial
revision: current Viewer and Compare conventions differ. This is accepted private
implementation in progress, without Core DTO/public plugin SDK changes or passing
integration evidence.
