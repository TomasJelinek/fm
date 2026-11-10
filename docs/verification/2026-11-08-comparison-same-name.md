# Automatic same-name comparison counterpart

Ctrl+Shift+C keeps two selected files in the active panel as the first choice.
Otherwise it compares that panel's single selected file, or its focused regular
file, with the opposite panel's explicit single selection. If the opposite panel
has no selection, the comparator finds its unique visible regular file with the
same exact native basename. Its cursor may be elsewhere, on the parent row, or
absent. Left/right order follows the panels even when the right panel is active.

The presenter captures immutable entries, the displayed index and selection; it
does not enumerate a selection or perform filesystem work. The comparator worker
resolves the inputs with cancellation checks, validates counterpart folder/index
membership, then uses the existing captured-file read and revalidation pipeline.
An incomplete listing/search/selection cannot launch. Hidden, missing, duplicate,
directory/link/special or virtual counterparts fail with an error; an unrelated
opposite cursor is never substituted.

Names compare as raw OsStr values. Display captions, Unicode normalization and
case folding are not filesystem identity. The current service contracts have no
per-directory native case-equivalence query: the volume CaseSensitiveLookup flag
reports a filesystem capability, not a directory's active policy. A case-only
name difference therefore requires an explicit opposite selection, including on
Windows. No path is truncated or reconstructed from a caption.

Salamander's source (`src/plugins/filecomp/filecomp.cpp`, input resolution near
lines 498–607) and its compare-files help describe the same-name counterpart flow.
Its ANSI/MAX_PATH and case-insensitive string implementation are not reused.

Evidence is retained under `_build/goal/compare-same-name/`. Causal RED tests
confirmed both missing backend resolution and presenter capture. Regression
coverage includes explicit selection precedence, one/two active selections,
right-active side order, unrelated/absent/parent opposite cursor, pending views,
missing/hidden/ambiguous matches, different case and Unicode normalization,
non-files, inconsistent parent/index, cancellation, paths beyond 260 characters,
and display captions differing from native names. Separate platform tests cover
raw Unix bytes and unpaired UTF-16 names that collide after lossy display
conversion. An owned-file service test reads same-named Unicode files and obtains
the expected rendered text difference.

Windows and Linux each passed all 15 comparator tests. All six Windows presenter
tests passed. Strict Clippy passed for Windows core/comparator all targets and UI
library, and for Linux comparator all targets. The Windows presenter test's
generated bindings precede the final tooltip-only wording change; the succeeding
UI library strict check compiled that wording too. The integrated app build
qualifies the final displayed help.

Logs: `windows-engine-green.log`, `windows-ui-green.log`,
`windows-engine-strict.log`, `windows-ui-strict.log`, `linux-engine-green.log`,
`linux-engine-strict.log`. The first Windows strict invocation lost its argument
delimiter in PowerShell and did not check source; its error is retained in
`windows-engine-strict-invocation-error.log`. The corrected explicit argument-array
invocation passed. Owned Rust files pass rustfmt and the tracked diff whitespace
check.

This is a focused comparator qualification; the final integrated app build and
native menu/shortcut smoke are coordinated separately. No dependencies changed.
