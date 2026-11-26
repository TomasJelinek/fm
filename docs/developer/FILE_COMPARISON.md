# File comparison and manual input

`Ctrl+Shift+C`, F10's Compare files and the shared toolbar invoke the same comparator.
The action remains available without a file target: it can ask for both inputs.

## Comparator settings

Options › Comparators holds an ordered list of rules that choose, by both basenames,
how a pair is compared: Automatic, the built-in text or image comparison, or a trusted
external program. A comparison session pins the accepted rules when it opens and keeps
them (`routing::select` in `midnight-compare-engine/src/routing.rs`, through the
engine's comparator policy source in `midnight-explorer/src/comparator_external.rs`);
later edits apply to new sessions. Design records: the
[design](../superpowers/specs/2026-11-13-comparator-associations-design.md),
[UI wireframes](../superpowers/specs/2026-11-13-comparator-associations-ui.md) and
[implementation plan](../superpowers/plans/2026-11-13-comparator-associations.md).

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

`midnight-compare-engine/src/file_choice.rs` preserves the unambiguous part of a pair
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
The callback receives live cancellation, checked again before content use. The
session's worker remains busy until an in-flight callback returns after cancellation.
The standalone `Service::new` constructor retains its default capture behavior for
embedding/tests; the application explicitly uses the injected provider constructor.
See [native media verification](../verification/2026-11-08-native-media-capture.md).

The app injects its existing `Arc<dyn PathPickerService>` into `Service`; it also
provides the existing platform-specific `native_path_picker_owner` to the UI.
Windows uses its Common Item Dialog and Linux the desktop file chooser portal.
Only the backend calls these implementations. The comparator permits any number of
sessions, each with one worker and one picker at a time, with bounded input/latest-output
mailboxes (see [Sessions and workers](#sessions-and-workers)). Closing cancels both the comparison and the picker's shared cancellation
token. Normal picker cancellation retains the draft and returns to the form.

The form disables editing, browsing and Compare while a request is pending, but
retains Cancel/native close. This prevents an old picker result overwriting newer
drafts. Selection results cannot be replaced by resize/status traffic: the choice
loop consumes geometry silently until another explicit user request. Retry retains
the latest viewport geometry/revision. Large file/selection cleanup remains on the
worker. The UI stores original PathBuf values separately from lossy display strings;
an unchanged field keeps the exact native bytes/UTF-16. An edited field explicitly
supplies a new Unicode path. No MAX_PATH buffer is introduced.

## Sessions and workers

Through 0.6.0 the service admitted two sessions and refused a third (*Close a comparator
before opening another (maximum two).*). Since the owner's decision of 26 November 2026
nothing counts sessions: `Service::open` never refuses for a count, and
`ComparatorWindows` opens any number of windows. What was bounded by the two sessions
is bounded as follows instead:

- **Workers.** Each session keeps one resident worker for its lifetime, mostly waiting
  for input. The application starts the comparison pool with
  `resident::Owner::start_elastic("midnight-compare", 2)`: two resident threads, and one
  more thread whenever a session finds none waiting; an extra thread ends when it finds
  no work, and shutdown joins every thread. Only a failure to start a thread refuses an
  opening (*A comparison worker thread could not be started. Try again.*;
  `Rejection::ThreadUnavailable`).
- **Heavy work.** Preparing the content (reading and aligning text, starting and
  decoding in the image workers) runs under the service's preparation gate
  (`computing.rs`): at most `COMPUTING` = 2 sessions prepare at once, first come first
  served, as many as the two sessions could before. Opening both files and resolving the
  text font come first, outside the gate, so an open that blocks on an unresponsive share
  holds back no other comparison. The image workers start inside the gate: each may grow
  to its 1 GiB job limit, which is the memory the gate bounds. A session that waits shows
  *Waiting for other comparisons to finish…*, then *Preparing the comparison…* once
  admitted; it stops waiting when it is closed, and when a replacement pair is chosen
  (it then opens the new files and waits in its own turn). The presented comparison
  (scrolling, find, identity and byte scans, image renders) runs outside the gate; byte
  scans and reads still pass the process-wide I/O gate.
- **Memory.** Each open comparison keeps what its files need: up to about 2.5 times its
  two texts (at most 64 MiB each), the byte-view and scan buffers, or two image workers
  (each at most 1 GiB, with at most 256 MiB of decoded RGBA, `MAX_DECODED_BYTES` of the
  published image-viewer API) that live as long as the comparison. Many large
  comparisons open together therefore add up; preparation peaks are bounded by the gate.
- **Comparator policies.** Each session pins the settings generation it opened with; the
  backend store keeps one shared set per distinct pinned generation (16 MiB matcher
  budget each) and drops a set at the next settings save once no session pins it
  ([engine lifetime](ENGINE_LIFETIME.md#capacity-and-ownership)). A pin reserves no
  cleanup slot.
- **External programs.** A launch holds one of the four shared desktop Open/Edit launch
  permits only while it captures and starts the pair; the started program holds one of
  the eight shared terminal, editor, Custom Command and external-comparison process
  slots until it exits. When those are taken, the launch is refused with the shared
  message and the comparator offers its file form again; the window stays open.

`computing_tests.rs` opens six comparisons at once, holds the admitted preparations and
checks that the next waits, shown as waiting, and starts when one finishes (with the
application's two and with six permits), and that closing a waiting comparison ends it
without opening a file.

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
Text comparison admits files up to 64 MiB, with at most 1,000,000 lines and lines up
to 16 KiB (`crates/midnight-compare-engine/src/text.rs`; the line limit bounds the
shaping of one painted row). A pair it does not admit (binary content, another
encoding, or a limit) is not refused: it is compared as bytes, and the summary names
the reason, for example *File A: not UTF-8 or UTF-16 text (binary or another
encoding); compared as bytes*. A file over 64 MiB is not read as text at all, and a
file whose first 256 KiB hold a control byte is refused after that first block.
Reading a large file reports its progress (*Reading File A: 12 of 30 MiB…*), then
*Matching lines…*; both stop at the next block or alignment step after Escape or Close.

Both files are captured exactly, change time included (see
[content reads](CONTENT_READS.md#change-time-drift-view-05)). When a text source fails
revalidation and a fresh capture of its path differs only in the change time (endpoint
protection adding a stream to a fresh file, for example), the comparator reads that
capture completely, as the first read does, and adopts it only when its decoded text and
encoding equal the retained ones; whole-input verification then starts again
(`revalidation.rs`). An interrupted first read is repeated once on the fresh capture.
Any other change, and image and byte comparisons, report that a compared file changed
(COMPARE-02 in the [backlog](../IMPLEMENTATION_BACKLOG.md)).

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

### Scrolling text and bytes

Compared text and the hex view scroll as the text viewer does, to the pixel: a wheel
notch glides Chromium's distance on the shared wheel glide, touchpad motion follows the finger
with the shared momentum, and the keys move the text viewer's distances at once (Up and
Down 28 logical pixels, Left and Right 40, Page Up and Page Down the whole view;
`midnight_ui_input::key_scroll`, which the viewer uses too). Options › Appearance ›
Scrolling glide and Scrolling speed apply as everywhere (the comparator runs on the
application's UI thread), and the window reads the system's wheel setting and its wish
for less motion from the host (`ComparatorWindows::set_wheel_setting`,
`set_motion_preference`; the application passes `midnight_ui::system_wheel_setting` and
`system_reduces_motion`) with its first native event and on each focus gain, as the
viewer windows do. One screen per notch moves 87.5 % of the content's height
(`NativeScroll::set_content_page`). Home and End (Ctrl+Home/End), the position bar and
difference navigation jump, as before; the position bar lands on its share of the
rows to the pixel, not on a whole row.

The worker paints a **surface** larger than the view: one view height above and below
and 256 logical pixels beside each pane's view (at most half the pane), cut at the
content's edges, at 768 rows and at 8 megapixels (`midnight-compare-engine`
`scroll_view.rs`). The frame describes it (`Frame::scroll`, `ScrollView`: the view's
position and the farthest one in logical pixels, each pane's view in the surface, the
margins, and the sum of every `Input::Scroll` the worker has taken). The window
(`midnight-compare-ui` `text_view.rs`) shows each pane's clip of the surface
(`source-clip-*` of two `Image`s) and moves both clips at once by every wheel glide frame,
touchpad delta and scroll key, within the content's edges: one view moves both panes, so
they cannot drift apart. It sends the worker the scrolling only when the view has moved
past half a margin towards content beyond it, or after it has rested for 50 ms
(`SETTLE`), and before any other input; the worker then paints a surface around the new
place. A frame says how much of the sent scrolling it includes, so presenting it never
moves the view. Should the view reach the end of the margin before the next surface
arrives, it waits there, as the text viewer waits at the end of its prepared strips.
Touchpad momentum reads the shown position only at the text's edges, as in the viewer,
so a wait for a surface is not taken for the end of the text. The text's position is a
row and the part of it above the view (`TextComparison::fraction`); a horizontal scroll
of *n* logical pixels moves *n* × the scale in physical pixels (before, it moved *n*
physical pixels).

`Input::Lines` (one row) and `Input::Pages` (the rows that were fully visible) remain for
other senders; the window no longer sends them for text or bytes. Images pan by pixels as
before: 24 logical pixels per line key and one viewport per page.

Measured on the real window with its engine and a controlled clock
(`midnight-compare-ui` `scroll_parity_tests`; [record](../verification/2026-11-23-compare-scroll.md)):
the shown offset of every presented frame is within 1 px of the text viewer's for one
notch, five quick notches, a reversal, Scrolling glide Off and Very long, Scrolling speed
Very slow and Very fast, a touchpad flick and its momentum, arrows, Page Down, a
horizontal notch and Right; the position bar lands within 1 px of its share; both panes
show the same pixels during and after a glide. Before, the text moved by whole rows a
frame after each glide step (one notch: four steps of 23 px, ending 8 px short).

Known limits: a new surface is copied into a Slint image on the UI thread when it is
presented (about 7.6 MB for a 900 × 600 window; 12 ms in a debug build), once per half a
margin of travel or at rest rather than on every frame. A view larger than the pixel
budget gets smaller margins, so a fast fling can wait at the margin's end for a surface.

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
share a stop*. Painting reads only the bytes around the painted rows (the surface's rows
and as many again before and after: at most 768 rows of 16 bytes, three times over, so
36 KiB per file) through the I/O gate.

Difference navigation uses the keys and buttons of text comparison: Next and Previous
difference (F7, N, Alt+Down or Alt+Right; Shift+F7, P, Alt+Up or Alt+Left) move to the
next or previous row that starts a region and stop at the first and last one; Alt+Home
and Alt+End go to the first and last region. A region the scan has not reached yet is
shown as soon as it is found (*finding the next difference…*), and Alt+End waits for the
end of the scan. Arrows, Page Up/Down, Ctrl+Home/End, the position bar and the wheel or
touchpad scroll by pixels, as text does ([Scrolling text and bytes](#scrolling-text-and-bytes)). The hex view has no
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

The limits of 32 MiB and 1,000,000 lines per file kept the worst case near one second
in an optimized build (two in a debug build) and the peak memory near two and a half
times the texts, as measured with synthetic logs of numbered 70-byte lines. The byte
limit is 64 MiB since the owner's decision of 26 November 2026 (`MAX_BYTES`). The line
limit, which drives the alignment's cost, is unchanged, so a 64 MiB file is admitted
when its lines average 67 bytes or more, and one with more than 1,000,000 lines is
compared as bytes as before. The alignment's time bound therefore still holds; reading
and peak memory follow the bytes, near 320 MiB for a pair at the limit (2.5 × 2 ×
64 MiB; not re-measured). `a_fifty_mib_file_is_compared_as_text` reads and aligns a
50 MiB pair. Larger pairs, which a text view would not make readable, are compared as
bytes with bounded memory. A pathological pair (1,750,000 lines in all, where Myers gives up and the
anchored pass emits about a million operations) aligns in about a second
(`many_changes_between_anchors_align_in_bounded_time`).

## Presentation and tests

The same native comparator window hosts the File A / File B form and the resulting
text, image or byte comparison. It retains main-window placement/size, native resizing/moving,
Escape and the close button. Any number of comparator windows are open. A closed one is
hidden at once and released on the next turn of the event loop (`OpenWindow::release`),
not inside the callback that closed it: its component, timers and session go, and the
session's mailbox, with any output the worker left in it, is dropped on a blocking
thread ([record](../verification/2026-11-24-ui-perf-fixes.md#closed-comparator-windows)).
A closed session tears down in the engine (its worker with the comparison's state, the
comparator policy it pinned and an external launch's accounting) while further
comparators open: nothing counts sessions, so nothing waits for that teardown.
`close_release_tests` opens six comparators at once, and holds a closed comparator's
teardown with a test policy whose `Drop` waits while two more open beside it.
Each input has Browse; Enter submits both nonempty fields. Short forms center within the window, scroll at 320×200 and keep Compare and
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

A prominent green identical result requires an input-bound, whole-input typed backend
verdict; neither status text nor equal image dimensions or pixels is a substitute.

Viewer and comparator windows follow the application appearance. `apply_appearance`
(`midnight-compare-ui/src/appearance.rs`) applies the current or previewed theme,
density and UI font to open windows and primes windows opened later; the UI font does
not change a window's document font. `Session::restore_palette` hands the worker only
the document colours (text, page, selection, selection text and accent) and returns
the accepted palette revision, which `Session::palette_revision` still reports after
close or disconnect; the worker stamps each completed frame with the palette it used,
so a revision is an acceptance, not a paint acknowledgement. Office and author HTML
styles and original image and PDF pixels keep their own colours.

Tests cover existing automatic Unicode/raw-native pairing, partial physical order,
empty/ambiguous/pending panel inputs, repeated invalid paths and retry, unchanged
source contents, picker thread/owner, cancellation, invalid picker results, raw path
presentation, real Slint field input, busy-state duplicate prevention and compact
rendering. See `midnight-compare-engine/src/file_choice_tests.rs` and
`midnight-compare-ui/tests/file_choice.rs`.

The File A / File B form follows Salamander's Compare Files dialog (two paths, each
with Browse); no implementation, artwork or MAX_PATH restriction is copied.

Before a window opens, the frontend merges the application document colours and seeds its
opening fences from the session's real `palette_revision()` before it publishes or
consumes the session; `restore_palette` serves later changes. Viewer and comparator
revision conventions differ, so no universal initial revision is assumed.
