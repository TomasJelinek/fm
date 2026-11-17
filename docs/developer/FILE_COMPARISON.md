# File comparison and manual input

`Ctrl+Shift+C`, F10's Compare files and the shared toolbar invoke the same comparator.
The action remains available without a file target: it can ask for both inputs.

## Planned comparator settings

Configurable comparator associations are **planned**, not implemented or qualified
by the current runtime. The [design](../superpowers/specs/2026-11-13-comparator-associations-design.md),
[UI wireframes](../superpowers/specs/2026-11-13-comparator-associations-ui.md) and
[implementation plan](../superpowers/plans/2026-11-13-comparator-associations.md)
cover ordered BOTH-basename routing to existing built-ins or a trusted external
program. The current pairing and ownership behavior below remains implemented.

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
are shown as the control pictures ␋, ␌ and ␛. NUL and other control characters
mark the file as binary.

The installed fixed-pitch families are scanned on a `comparator-fonts` thread, started
when a text comparison begins reading; the scan reads every installed family and can
take seconds on a loaded machine. The preferred family is resolved against the catalog
(case-insensitive match, else Consolas, else generic monospace). A completed catalog is
kept for the service, so every later text comparison resolves the preferred family
before its first frame. Only a comparison that starts while no scan has completed
paints its first frame in the generic monospace family and is repainted when the
catalog arrives; its font chooser stays disabled (*Loading fonts…*) until then. Closing
the comparison cancels its scan, which then keeps no partial catalog.

Arrow keys move exactly one row (`Input::Lines`), Page Up/Down move by the rows that
were fully visible (`Input::Pages`), and wheel distances are converted with the painted
row height, so paging neither skips nor repeats lines at any font size. Images pan 24
logical pixels per line and one viewport per page. The UI keeps at most 64 inputs
while the worker is busy. Repeated line, page and scroll steps accumulate. State
inputs (geometry, image mode, font, Fit or Actual pixels, and a file-choice request)
are never dropped: a newer one replaces the queued one of its kind and is queued after
the navigation that preceded it. A full queue therefore always holds navigation (scroll,
line and page steps, seek, difference and zoom), and sheds the oldest such input first.
An idle worker blocks on its input queue instead of polling; only a pending font
catalog bounds that wait.

## Presentation and tests

The same native comparator window hosts the File A / File B form and the resulting
text/image comparison. It retains main-window placement/size, native resizing/moving,
Escape and the close button. Each input has Browse; Enter submits both nonempty
fields. Short forms center within the window, scroll at 320×200 and keep Compare and
Cancel outside the scrolling area. File fields own their editing/navigation keys.

Tests cover existing automatic Unicode/raw-native pairing, partial physical order,
empty/ambiguous/pending panel inputs, repeated invalid paths and retry, unchanged
source contents, picker thread/owner, cancellation, invalid picker results, raw path
presentation, real Slint field input, busy-state duplicate prevention and compact
rendering. See `midnight-compare-engine/src/file_choice_tests.rs` and
`midnight-compare-ui/tests/file_choice.rs`.

Salamander reference: `src/plugins/filecomp/dialogs.cpp` (`CCompareFilesDialog`) and
`src/plugins/filecomp/lang/lang.rc` (`IDD_COMPAREFILES`). The two-path/Browse behavior
is adopted; no C++ implementation, artwork or MAX_PATH restriction is copied.
