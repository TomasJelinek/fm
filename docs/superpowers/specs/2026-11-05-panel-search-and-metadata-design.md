# Panel prefix search, tooltips and filesystem compression

## Confirmed scope

Typing directly in a focused file panel starts a literal filename-prefix search.
The user's later clarification makes search and filtering **always Unicode
case-insensitive**, irrespective of filesystem sensitivity. Plain typing starts
the mode without Shift; Windows Shift+drive-letter shortcuts remain available.
An Options preference switches from jumping among matches to showing matches only.
The existing full Unicode/raw path, asynchronous backend, security, OS separation,
Rust style/100-column target and 100000-byte source limits remain in force.

Additional requested work: document native build/run instructions and convenience
BAT/SH helpers; shift tooltips away from the cursor, flip near edges and slightly
reduce their font; visibly mark filesystem-compressed entries.

## Interaction and rendering

- Match the start of files and folders in their current displayed sort order.
  The synthetic `..` row stays available for navigation and never matches text.
  Search is literal: punctuation is not wildcard, regular expression or markup.
- First/changed query chooses the first match. Up/Down visits previous/next
  matches without wrapping; Home/End and Page Up/Down operate on the same match
  order. Moving the cursor does not change multiselection.
- Escape clears the query/mode and restores the full view, keeping the last usable
  focused raw entry. Backspace edits by extended grapheme; empty query exits.
  Space selects outside search and becomes query text inside it. Insert ends
  search and performs its existing selection action.
- Native TextInput receives committed Unicode/IME text. Preedit stays local and
  never triggers backend search. IME owns composition keys; Ctrl+Space remains
  available to IME and Ctrl+P inserts the current path. Shift-digit Hot Paths are
  suspended during search/composition so shifted punctuation can be entered;
  other established Ctrl/Alt shortcuts and separate editors keep their meaning.
- Reuse the panel path slot for contextual query/preedit feedback, with no
  permanent extra band. Independent pane queries survive focus changes. Folder
  and folder-tab navigation clear them; same-directory sorting/refresh retains
  and recomputes them against the accepted new immutable listing.
- While searching or with zero matches, keyboard activation/Properties must not
  act on a leftover nonmatching cursor. Explicit pointer activation resolves the
  captured visible raw entry; it never treats a filtered ordinal as a native ID.
- Color the matched original-name prefix through a full shaped paragraph. A
  disabled, read-only TextInput with persistent transparent-background selection
  can retain bidi/ligature shaping and color a UTF-8 range without parsing names
  as markup. Keep long-name clipping/ellipsis and pass-through row input.

## Backend contract and Unicode

`SearchRequest` contains pane, navigation, monotonic per-pane `SearchRevision`,
native location, immutable `Arc<[Entry]>` source and Unicode query. `Command::Search`
and `CancelSearch` feed latest-wins, bounded per-pane workers. One actual CPU job
per pane is awaited; idle actors wait asynchronously. Full scans, folding, index
construction and large-data destruction belong to backend workers.

`SearchResult` retains the exact source Arc, sorted `SearchMatch` values containing
canonical source-row positions/original display-prefix byte ends, and a contiguous
matched `DirectoryIndex`. `SearchReady`/`SearchFailed` carry pane/navigation/revision.
Validate source identity and location against committed/visible engine snapshots;
guard publication after output backpressure and reject stale results in the UI.
Typing during initial streaming waits for the final immutable source.

Use pinned, already-cached `unicase 2.10.0` full Unicode case folding and
`unicode-segmentation 1.13.3`. Fold comparison text only: raw names/paths and stored
identities are never normalized. No accent stripping or NFC/NFD equivalence is
introduced. Full folds include expansion (`ß`/`ss`) and sigma variants; highlighting
ends at an original extended-grapheme boundary. Non-Unicode native names remain
addressable in the ordinary view; a nonempty Unicode query does not match their
lossy replacement text.

The query uses text delivered by the native input method/toolkit. Pinned Slint's
Parley input path normalizes each incoming key/composition event to NFC before
TextInput receives it; insertion does not normalize existing query contents.
Separate commits can therefore retain a decomposed query. Midnight Explorer adds no second
normalization and never changes original filesystem names. Input tests distinguish
single-event composition from separate combining commits rather than asserting
an unsupported raw-IME-packet guarantee.

## Filtering, selection and continuity

Default jump mode retains all rows. Filter mode maps through matched source
positions in O(1), retaining the canonical listing/index for filesystem watches,
folder-size scans and raw identity. Filtering is not navigation and never commits
a subset to the backend as the visible filesystem directory.

Use the actual displayed view's backend-built index for Ctrl+A, deselect/invert,
selection rebases and selected-item Properties. Rebase selections on a changed
filtered view so hidden entries do not remain actionable. Pending query changes
may retain the prior visible projection while indicating work; keyboard activation
is guarded until the current result is ready. Capture operation/row identities,
and invalidate held gestures when their projection changes. Large retired sources,
matches and indexes use the existing background reclamation path.

Persist `quick_search_filter: bool` (default false) in strict preferences version4.
Migrate versions1–3 without changing their existing values. Reject malformed/null,
duplicate and foreign-version fields; preserve local edits when initial loading
arrives late. Save through the existing asynchronous atomic settings writer.

## Inline directory branches

The same input/search interaction applies after a native or archive panel adopts
inline branches. Search examines the currently loaded, expanded presentation;
it does not enumerate unopened folders. Match the original raw filename with the
shared Unicode matcher. Archive captions are not name authority: invalid UTF-8
raw member names do not match a nonempty Unicode query, and a differing caption
does not receive an invented prefix highlight.

Transient query options travel with the existing acknowledged tree reprojection
request. Persistent filters intersect the prefix predicate. In quick-filter mode,
matching descendants retain necessary unselectable, unmatched ancestor context.
Jump mode preserves marks; filter mode removes hidden/context-only marks. Sorting
preserves the current query and applies within sibling groups. Clearing the query
restores the persistent-filter projection and the last usable focused occurrence,
without reopening collapsed branches or reviving hidden selection.

The backend creates ordered matching-row positions and original-name UTF-8
highlight boundaries together with the immutable projection and selection. The
UI only reads this index for counts and cursor movement. Keep one latest desired
query and at most one in-flight options transaction. Rapid input and Escape
supersede older results; pending or failed queries cannot activate a leftover
cursor. Options preview and Cancel also supersede the desired filtering mode.

Tree search queries have a 4,096-byte command admission budget and reject NUL;
this is independent of path and filename lengths. Transient queue/stale-view
failures carry typed retryability, not a classification inferred from diagnostic
text. Retry on the ordinary bounded UI pump and do not flash failure notices for
automatically retried searches. Semantic/resource failures preserve the last
accepted projection and are reported. Resolved selection requests are tracked
separately from successfully applied selection revisions, so a refused selection
cannot leave search waiting indefinitely or grant unaccepted selection authority.

Insert captures the exact matching occurrence before clearing an active query.
Its single bounded intent waits for the acknowledged empty-query view and settled
selection, then toggles that occurrence and advances the caret. A new query,
navigation, disappearing row or terminal clear failure cancels the intent. A
rejected stale publication cannot change the saved caret or cancel valid input.

## Tooltips

Use a slightly smaller `Theme.small-font`. Prefer an approximately14px right and
18px downward pointer gap; flip each axis when its preferred side will overflow,
and keep the visible box inside its hosting window/surface with an8px margin.
Adapt text wrapping/viewport sizing for constrained surfaces, retaining complete
plain text in accessibility data. Tooltips stay input-transparent and never steal
focus, including inside menus, dropdowns and Options. File-row tooltips stay removed.

Pinned Slint1.18.1 does not expose placement on the public Tooltip pseudo-element.
An isolated structural compiler-IR adapter may be used only if its spike proves
normal resource/dependency tracking and builtin ToolTip lifecycle can be retained.
Do not patch dependency registries or rewrite generated Rust by text substitution.
The exact-version adapter must be isolated and exercised on upgrades; a root-only
overlay that paints below popup contents is insufficient.

## Filesystem compression

Add `CompressionStatus::{Unknown, Uncompressed, Compressed}` to `EntryMetadata`.
Show a small noninteractive mark at the native icon for Compressed, with an
accessible description and F1 explanation. Preserve native icon colors, filename
search highlighting and logical file-size values. Archive suffixes alone are not
evidence of filesystem compression. Unavailable attributes remain Unknown.

Windows reads the native compression attribute through a safe MetadataExt adapter
in its own platform file. Linux uses safe rustix `statx` with no-follow/no-automount
flags and respects the compression support mask; unsupported kernels/filesystems
fall back to ordinary metadata with Unknown. Query metadata without opening file
contents, following the final link, changing compression or performing size-ratio
guesses. Keep each native implementation/tests in its respective OS module.
See [Windows compression attributes](https://learn.microsoft.com/en-us/windows/win32/fileio/compression-attribute)
and the [Linux statx contract](https://man7.org/linux/man-pages/man2/statx.2.html).

## Directory links, junctions and icon overlays

Directory links and Windows junctions need a recognizable link icon. Preserve
their distinction from ordinary directories: `EntryKind::DirectoryLink` maps to
`IconKind::FolderLink`, and recursive size calculation must still accept only
real `Directory` entries. Explicit navigation follows the selected raw location;
directory-only navigation fails safely if its target has become a regular file.

The Windows adapter classifies directory links from the cached enumeration
`FileTypeExt` information in its own listing module, without opening the target
or querying extra metadata per entry. A platform-neutral enumeration helper
accepts the classifier; it contains no Windows implementation or conditional OS
body. A missing Linux target-type hint remains a generic symlink, rather than
triggering an implicit target probe, network access or mount merely for an icon.

Windows icons use the Shell's system link overlay on the appropriate folder/file
base, using cached generic types without submitting untrusted link paths to Shell
extensions. [SHGetStockIconInfo](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shgetstockiconinfo)
documents `SHGSI_LINKOVERLAY`. Other platforms prefer suitable host-theme artwork
and use recognizable application link artwork when unavailable. Icon acquisition,
composition and caching stay on bounded backend workers. The link overlay and
compression indication can coexist; no file-row tooltip is introduced.

Tests cover a real Windows junction, no-follow classification, raw Unicode names,
native overlay request flags, visible fallback artwork, UI folder-link mapping
and continued exclusion from recursive scans. Inspect Salamander's link/icon
behavior as prior art without copying its source or fixed-path assumptions.

## Prior art and implementation choices

Open Salamander references: `src/fileswn0.cpp:34`, `:888`, `:1359`, `:1440`
and `help/src/hh/salamand/basicwork_qsearch.htm:17`. Its bounded prefix traversal,
Escape/Insert/Space and same-directory retention inform this design. Its ANSI
tables, fixed MAX_PATH buffers, wildcard semantics and byte-based backspace are
not reused; no GPL implementation is copied. Its persistent mask filter is a
different feature from this optional live query filter.

Chosen approach: immutable backend match projection plus native text input.
UI-only scans conflict with the existing backend boundary and large listings;
re-enumerating the filesystem per keystroke adds latency and races. Simple split
Text widgets can break contextual shaping, so the renderer spike validates a
single-paragraph color range instead. The filesystem-dependent case proposal was
removed after the user explicitly selected universal case-insensitive matching.

## Verification

### Reuse and generated Rust

Repeated controls and menus use shared components with explicit data/callbacks.
`hinted-inputs.slint` owns the tooltip inside reusable ComboBox/LineEdit/action-button wrappers;
putting child content on each instance can force Slint to inline the component.
`command-menu.slint` shares the F9/F10 menu body while each popup retains its
geometry and action dispatch. A fixed delegate count keeps native icon arrival
from replacing a held menu item.

Panel key policy lives in the pure Rust `file_keys.rs` router and a small callback
bridge. Native editor/IME events retain their editing path; passing them through
must not refresh models. Slint receives the small routing result for focus and
text actions rather than generating the same large shortcut tree in the UI.

The isolated tooltip compiler adapter moves each original stock show body into
one private impure function and calls it for the initial show and conditional
geometry retry. It does not clone the stock body into both branches. Format the
generated Rust AST with `syn`/`prettyplease` before writing it; generated output
must not become a multi-megabyte single line. Resource/import tracking remains
the normal Slint build step. The adapter is exact-version pinned and fails when
its expected compiler contract is absent.

Measure the same frozen UI input with stock/adapted generation and equivalent
formatting. Report normalized token bytes separately from formatted disk bytes;
whitespace changes do not prove less code. Compiler output belongs under Cargo's
`OUT_DIR` in `target/`, stays ignored by Git, and is excluded from the 100KB
handwritten-source limit. Generated source size is not executable size or runtime
memory usage; test behavior and native rendering after reuse changes.

The [deeper code-generation audit](../../verification/slint-code-generation-size.md)
separates initialization/layout/item-tree plumbing from application functions and
measures stripped two-pane and control fixtures. FilePane and TipBody already
share implementations. Further large savings need fewer distinct compositions,
model-driven dialog bodies or qualified lightweight controls; moving functions
alone cannot remove the dominant generated work. Preserve keyboard, IME,
accessibility, icon fidelity and native popup ownership when evaluating alternatives.

### Behavioral checks

Meaningful RED/GREEN tests cover Unicode folds/graphemes/invalid-native identities,
no-match/Escape, bounded arrow traversal, initial streaming, delayed/out-of-order
results, cancellation/backpressure, same-folder refresh/sort, selection/Properties,
held clicks, multiple panes/tabs, native preedit/commit and command/path editor
isolation. Render tests cover colored prefixes, RTL/ligatures, long-name elision,
compression marks and tooltip placement/click-through at every edge in both
densities/themes/font sizes. Test strict v4 migration and late-load preferences.
Exercise build helpers' argument/exit behavior, then run final native builds and
full Windows/Linux tests, Clippy, format/source guards and smoke checks.
