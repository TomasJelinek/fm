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
It consumes the 27-file freeze with30 newly authored tests and overlaps earlier
comparison checks. The canonical P3 packet is promoted (104 sources/159 source-derived selections).
The selected39 integration cases are38 passed/1 Options-init-focus failure.
P3 units initially20/1; the corrected draft exact selector now passes once, with
the other20 not rerun. Native comparator4/0 is scoped below; no full159/native
input or ReadAt acceptance follows. See the [current checkpoint](../ACTIVE_WORK.md#current-checkpoint).

The corrected faithful native comparator fixture ran **4 actual Windows cases /
0 failures** (`comparator-d6ce28b8eb`): raw parent/argv/cwd, NotStarted cleanup,
preworker cancellation/two session credits/no child, and non-Unicode Automatic
fallback. Components were unchanged and every case left remainingOwned[]. This
closes the old fixture settings-timeout for those cases; historical failures remain
recorded. Fixture16 blocking/6 resident/10 transient-holder qualification does not
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
   decoder assignments, then text for other regular files. This fallback does
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

## Presentation and tests

The same native comparator window hosts the File A / File B form and the resulting
text/image comparison. It retains main-window placement/size, native resizing/moving,
Escape and the close button. Each input has Browse; Enter submits both nonempty
fields. Short forms center within the window, scroll at 320×200 and keep Compare and
Cancel outside the scrolling area. File fields own their editing/navigation keys.

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
status text nor equal image dimensions/pixels is a substitute. The current selected39 integration cases are38 passed/1 Options-init-focus failure.
The diagnostic handoff records OK focused while the expected pattern is visible.
The corrected draft exact selector passed once after initial20/1; the other20
were not rerun. Prior55 unit passes retain their own frozen source, and no full159
or native input qualification follows. See the
[current checkpoint](../ACTIVE_WORK.md#current-checkpoint) for exact receipts.

The corrected faithful native comparator fixture ran **4 actual Windows cases /
0 failures** (`comparator-d6ce28b8eb`): raw parent/argv/cwd, NotStarted cleanup,
preworker cancellation/two session credits/no child, and non-Unicode Automatic
fallback. Components were unchanged and every case left remainingOwned[]. This
closes the old fixture settings-timeout for those cases; historical failures remain
recorded. Fixture16 blocking/6 resident/10 transient-holder qualification does not
implement the production-pool repair or supply native ReadAt counts.

Root accepted the Viewer/Compare live-appearance contract for private
implementation; it is not implemented or passing. Application UI font is separate
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
