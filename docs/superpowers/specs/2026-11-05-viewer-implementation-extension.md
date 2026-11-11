# Viewer plugins and external editing

This accepted user-request extension follows the completed browsing/mutation and
Midnight Explorer branding work. It refines the [plugin contract](2026-11-05-dynamic-viewer-filesystem-plugins-design.md),
without replacing its ABI, ownership, asynchronous work or safety requirements.
This extension takes precedence over the older combined text/source plugin and
Markdown-after-archives ordering: the user now requests three viewer packages first.

Implementation details and verification are maintained in
[viewer-implementation.md](../../verification/viewer-implementation.md), including
the four-session limit, page-bounded selection/copy, native installation trust checks,
restart-based plugin preferences and the actual syntax catalogue. These limits apply
to this initial delivery; archive/native filesystem plugin migration remains separate.

## Delivery and independence

Deliver three independently built viewer packages: basic text/binary, source
highlighting, and rendered Markdown. Each has its own Cargo workspace, lockfile,
tests, fixtures, licensing and CI, and only depends on the public API/SDK from this
project. Plugin development/build/test/install never edits the host source tree.
The user selected one Git repository on 2026-11-06: projects live under `plugins/`
but are not members of the host Cargo workspace. Each exported plugin directory
must still build and test with the released public SDK without application sources.
The host owns generic discovery, content leases, routing and the common Slint
window/renderer. No parser/highlighter implementation is linked into the host.
The API/SDK is packaged separately so plugin builds need no application checkout.

The existing native filesystem adapter can supply content leases during migration;
this stage does not claim to finish the separate native/archive filesystem plugin
migration. Viewer discovery uses `viewers/<package>/plugin.toml` beside the actual
executable, never the panel directory. Removing a viewer package removes its
capability at the next startup. Archive implementations remain a later stage.

## Interaction

F3 selects an installed viewer for the file; Markdown takes precedence for `.md`,
source highlighting for its published language catalogue, basic text for unknown
formats. Alt+F3 explicitly requests the basic plugin. No silent external launch
or hidden statically linked fallback is allowed when a plugin is absent.

Each viewer is a separate host-owned window, initially matching the main window's
logical content size and theme/font settings. Escape closes it, cancels outstanding
work and releases its leases on backend workers. A plain Space in the content area
advances to the next regular file in the captured source panel order and wraps to
the first. Directories and special/device entries are skipped. Text inputs consume
Space normally. The capture retains backend-prepared native identity and order when the panel later
navigates, sorts or closes; a missing/changed file reports an error rather than
silently selecting an unrelated item. Rapid next-file requests are latest-wins.

The viewer exposes vertical/horizontal scrolling, Ctrl+F search, next/previous
match, and bounded cancellable text selection/copy. Search controls use compact
labels/tooltips. Escape closes the viewer even when its search field has focus.
Searching and indexing never run on the UI thread. Source byte offsets remain
separate from UTF-8 ranges and pixel positions.

The basic plugin supports UTF-8 and BOM-marked UTF-16 LE/BE, CR/LF/CRLF, malformed
bytes and long lines. Binary mode presents byte offsets, hexadecimal bytes and a
printable text column; explicit text mode renders control/invalid bytes visibly
without terminal escape interpretation. Large files are paged, not read into one
String. Scroll extent can remain provisional while a sparse index progresses.

The source plugin publishes the actual supported syntax catalogue, recognizes
extensions/shebangs, and falls back to plain rendering for unknown or resource-
limited syntax. “All known text types” means all languages in that catalogue,
not a promise of a grammar for every proprietary language. Markdown uses semantic
document blocks and the same theme/selection infrastructure. It never executes
HTML/scripts or fetches remote images, styles, fonts or links automatically.

## Renderer and protocol implementation gates

Slint 1.18.1 StyledText supports raw interpolation and styled paragraph shaping,
but its stock item wraps words and exposes no general text-selection API. Before
freezing the SDK, prove no-wrap/horizontal scroll, RTL/combining hit testing and
source-correct selection in a shared host renderer adapter. If stock integration
cannot satisfy those checks, use one shared Parley layout/render adapter owning
both drawing and hit testing; do not overlay a differently shaped plain input.

ABI 1 explicitly negotiates Text, Source, Binary and Document modes and the
`text-page.v1`, `binary-page.v1` and `document-blocks.v1` models. Alt+F3 requests
Text even for binary content; ordinary F3 may choose Binary. Markdown search/copy
uses displayed text with source anchors; source mode is a separate action.
Wire messages, sparse indexes, source maps, style runs, queued events and parser
state need explicit byte/item/depth budgets beyond the text cache cap. A u64
source anchor plus a normalized scrollbar/local viewport avoids representing an
entire huge file as Slint i32 row indices or f32 pixel coordinates.

The native broker offers LiveValidated revisions, not an immutable snapshot:
validate the prepared native identity on open and detect content changes during
reads. Listing EntryId/path/size alone is not sufficient identity. Separate
exported plugin builds use the released SDK archive/local registry with exact
commands; merely producing `.crate` files is not a dependency-source workflow.

## F4 and editor settings

The panel toolbar also offers New empty file (Shift+F4). Its compact dialog takes
a filename and uses the backend mutation queue to create a zero-byte file with
exclusive no-overwrite semantics in the captured active directory. A collision
keeps the existing entry intact and lets the user correct the name. Successful
creation refreshes/focuses the new entry; opening an editor remains an explicit F4
action. Test cancellation, Unicode, a replaced parent and concurrent name creation.

F4 opens the focused regular local file using a backend platform service. Settings
contain ordered, enabled wildcard/regex associations. The first matching enabled
rule chooses the program, literal argument list and optional working directory;
the previous custom/system editor remains the final fallback. `{file}` is a whole
argument, and the raw native path is passed directly, never interpolated into a
shell command. Patterns compile on the backend; Options preserves its OK/Cancel
transaction. See [editor associations](../../verification/2026-11-11-editor-rules.md)
for precedence, raw-path handling, migration and argument editing.
Configuration cannot come from the directory being viewed. Invalid/missing editor
configuration reports an error without substituting an unrelated program.

Defaults are discovered on the backend: Windows Notepad from the system directory;
Linux an available graphical editor (the user's desktop association where it can
be resolved safely, then common installed editors), with a terminal editor only
through the existing explicit terminal service. macOS support, when that platform
is enabled, uses the system TextEdit application. Each platform implements its
own source file/service. Options displays the resolved default and permits an
explicit executable/arguments override. A viewer remains read-only; launching an
external editor is a separate explicit user action. Editing archive members or
remote resources requires a future staged writeback/conflict contract.

## Evidence and acceptance

Salamander references inspected without copying its implementation:
`src/viewer3.cpp:3360` maps Space to next-file; the demo viewer at
`src/plugins/demoview/viewer.cpp:707` handles Escape and at line 723 uses the
source panel enumeration. The requested wrap behavior is explicit here; it is
not inferred from Salamander's end-of-list behavior.

Required checks include real DLL/SO installation/removal, wrong ABI and malformed
manifest refusal, delayed/cancelled reads, Unicode/binary and >4 GiB offsets,
bounded memory for huge lines, cross-page search, stale results, Markdown active
content, selection/source mapping, window size/Escape/Space/focus behavior, and
literal editor argv containing spaces, quotes and shell metacharacters. Plugin
tests live with each plugin; host tests exercise only generic public boundaries.
