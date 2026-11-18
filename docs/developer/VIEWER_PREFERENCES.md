# Viewer preferences

Viewer choices are application-owned settings. They do not change the independent
plugin API, grant plugins settings-file access or store document contents. The
host captures `Arc<ViewerPreferences>` in `ViewerLaunch` before opening a source.

## Plugin catalogs in Options

Options → Plugins has Viewers, Filesystems and Thumbnailers tabs. The latter two are read-only catalogs.
Viewer enablement, extension overrides and priority retain stable plugin IDs and
their existing restart/application-settings semantics. Short names and functional
descriptions are presentation metadata; they do not redirect routing or migrate
saved IDs. The source viewer's description identifies its supported lexical
profiles rather than placing resource-limit wording in its title.

The Extensions editor is a single-line field. Long lists scroll horizontally
with the caret, so Home/End and arrow keys reach either end without clipped
wrapped lines. Spaces, commas and semicolons remain supported separators;
multiline clipboard input becomes space-separated text. Validation, Reset,
preview, OK and Cancel retain their existing transaction semantics. Options
starts at 840×580 logical pixels, grows for larger fonts and remains constrained
to the available window area; users can still resize it. See the
[layout verification](../verification/2026-11-11-options-extension-layout.md).

The existing internal `Command::ListViewerPlugins` requests all three catalogs
through one bounded backend worker. It publishes independent
`Event::ViewerPluginsListed`, `Event::FilesystemPluginsListed` and
`Event::ThumbnailerPluginsListed` results. These are application catalog events,
not new plugin ABI messages or filesystem capabilities. One catalog's error does
not discard another catalog's successful rows; Refresh stays pending until all
three replies, including errors. Repeated Refresh is coalesced while pending.

`FilesystemPluginsListed` contains metadata discovered by the native/network and
archive filesystem hosts. Installed means a trusted compatible manifest and
regular artifact presence were checked. The catalog never loads a library,
creates a provider, launches a worker, activates a share or infers a running state.
Its overview has no filesystem enable/disable, priority, extension-edit or runtime
reload control. Existing archive decoder settings remain on the Archives page.
Filesystem selection displays details without saving viewer preferences or
granting any operation authority from the displayed ID or location.

`ThumbnailerPluginsListed` carries provider ID, readable name, decoder version,
supported extensions, video capability, helper path and installation status. It
reuses thumbnail service discovery and its path-trust rules, without decoding
media, launching helpers or touching the cache. An absent directory is empty;
a malformed or untrusted installation reports an error and clears stale rows.
Selection survives reordering by provider ID. Global preview enablement, network
access, thumbnail size and cache controls remain on Previews. Per-provider
checkboxes, extension overrides and runtime reload are not exposed by this catalog.

## Viewer window choices

| Choice | Default | Storage |
| --- | --- | --- |
| Text wrapping | Off | Boolean |
| Source line numbers | On | Boolean |
| Search case sensitivity | Off | Boolean; query text is not stored |
| Text/source font | Monospace, 14 logical pixels | Separate family and size per mode |
| Text comparator font | Consolas if installed; otherwise system monospace | Independent installed fixed-pitch family |
| Markdown font | Sans-serif, 16 logical pixels | Family and size |
| Markdown presentation | GitHub | GitHub, Compact or Comfortable |
| Raster image zoom | Fit | Fit, actual pixels or manual scale; restored when opening |
| PDF zoom | Fit on each new source | Legacy document zoom field remains readable/writable; not restored |

Manual scale uses integer basis points: 10,000 means 100%, with 0.01% stored
precision. Font sizes are 8–48 logical pixels; family names are nonempty Unicode
strings, at most 256 UTF-8 bytes and without control characters. These are settings
resource bounds, not native filename/path limits. Invalid values fail atomically.

Each newly opened PDF starts in Fit, including file cycling to another PDF.
Previously saved document zoom values remain valid in the existing settings
schema, but no longer set a new document's view. A late settings load also leaves
the current PDF geometry unchanged. No settings rewrite or schema migration is
needed: explicit zoom commands control the current document, and its first zoom
step starts from the displayed scale. Raster images still restore their separate
saved zoom.

PDF Fit enlarges small pages and shrinks large pages to the viewport, retaining
the existing maximum scale of 64× and bounded raster size. Raster-image Fit keeps
its no-upscale policy. The document Fit policy survives page changes and resizing.

Changing PDF pages retains the current manual zoom or actual-pixels mode in both
directions, including pages with different dimensions. The backend preserves the
runtime scale directly, without rounding it through the persisted basis points.
Fit remains a mode and recomputes its scale for the new page. Page navigation
starts at the new page's top; dimensions smaller than the viewport are centered.
It does not change the saved zoom preference. This policy belongs to the viewer
backend; the independent PDF plugin renders the supplied page and viewport.

## PDF layouts (in development)

Use **Paged** to view one PDF page at a time, with the previous/next page buttons
or **Ctrl+PageUp / Ctrl+PageDown**. Use **Continuous** to scroll through a sequence
of pages. **Fit**, **1:1** and the zoom controls remain beside the layout buttons.
The backend prepares bounded page regions as the view changes.

The new layout controls remain in development until their UI/application gates pass.
The prior Fit/Paged footer regression and two keyboard cases now pass within the31
continuation. Selected39 remains38/1 because Options focus still fails. The current
canonical104-source/159-selection packet is source-derived, not159 passes; the separate
The earlier Viewer46 timeout and subsequent45/1 retain their dated source.
On the canonical `fc4fcdf7…` shared-helper checkpoint, the exact former failure
passed first and the other45 then passed: **46 actual passes /0 failures**.
Both raw logs retain their separate1+45 counts; all nine document cases and
19 image-presentation cases passed, including the genuine immediate-open errors.
These overlap earlier scopes and are not additional unique passes or all159.
Options focus, native/full-app and production migration acceptance remain open.

The preceding backend slice and two final native PDF cases passed, including
mixed page sizes and rapid camera replacement with the real C PDF worker. Those
functional fixtures do not establish full-app input, production native-filesystem
capture or scrolling performance. See the [current checkpoint](../ACTIVE_WORK.md#current-checkpoint)
for the exact accepted source and outstanding gates.

## Preference updates

`ViewerPreferenceChange` describes one changed field. The viewer service accepts
and validates the small configuration update independently of rendering and file
I/O. The window invokes the application's preference listener immediately after
acceptance, so an immediate Escape does not depend on another rendered frame to
save that change. Programmatic restoration does not invoke this listener.
Wrapping changes only through `ViewerPreferenceChange::Wrap`, so the displayed
state cannot diverge from the saved choice; `ViewerInput` has no wrap input.
Accepted changes wake the viewer's backend worker, which reflows the displayed
page without rereading the file.

The presenter merges each patch into the latest application settings, preserving
unrelated changes from other viewer windows. During the initial settings read,
at most one patch per field is retained; those patches merge into the loaded
configuration rather than replacing unedited values with defaults. Persistence
uses the existing backend settings command, coalescing and atomic file publication.
Queue rejection retains the latest unsaved values for the existing save retry and
shutdown flush. A failed initial read never authorizes overwriting the old file.

Startup reveals the main window after 10 seconds without a settings answer, so a
viewer can be opened before the read arrives; it starts with the defaults plus its
own edits. When the read arrives, the explorer restores the loaded choices, merged
with those edits, into every open viewer window, so conflicting edits made in
different windows before the read converge to the last one. Raster images adopt the
restored zoom; PDF geometry stays as it is. Restoration saves nothing and reports
no edit. Open comparators likewise adopt the loaded comparison font; one still
waiting for its font catalog applies it when the catalog arrives, and a family the
catalog does not offer leaves its font unchanged.

The strict application codec writes schema 21 when viewer settings differ from
defaults, or schema 22 when the comparator font is customized. A customized panel
split independently requires schema 23, retaining viewer choices and allowing
an omitted default comparator font. Earlier schemas
load the Consolas comparator default. Its toolbar offers installed fixed-pitch
families and System monospace, using the existing backend renderer's cached font
catalog. Unknown or proportional families cannot be selected. If a stored family
is absent, rendering uses Consolas when available, then generic monospace, without
rewriting the user's saved choice. Family changes retain the diff and viewport.
See the [comparator font contract](../superpowers/specs/2026-11-08-comparator-font.md).

Earlier schemas load default viewer choices. A viewer field in an older
schema, a missing schema-21 viewer field, unknown fields, invalid font values and
invalid zoom modes are rejected. Defaults continue using the minimum schema needed
for other selected application features. Disabled or absent plugins do not erase
their viewer preferences.

Settings live in the same [native application preferences file](../../README.md)
as other options. Search text, opened file contents, per-file reading offsets and
viewer window geometry are not serialized here. New viewer windows still use the
main application's placement.

The [source/Markdown presentation controls](VIEWER_PRESENTATION.md) now expose
these choices. Actual Windows input checks cover the controls; manager-recreation
tests cover restoration and immediate Escape. Type/codec and recreation tests
are not an actual application restart. Live restart, concurrent-window field
changes, disabled plugins and failed writes remain part of the integrated
persistence gate; see [current evidence](../verification/2026-11-08-viewer-controls.md).

## Office contents preview (in development)

The built-in Office preview is intended to open readable text, tables or sheet
contents immediately, with navigation to further bounded content pages. It aims
at readable contents rather than faithful Office page layout. An index-only
opening screen is not the accepted final workflow; the initial-content helper
contract is approved, with initial-content adapter cases passing in the scoped
headless gate. Native and application qualification remain open.

**XLSX was the first backend slice.** DOCX, XLSX, PPTX, ODT, ODS and ODP now have
a scoped six-format gate:68 service,18 XLSX integration and27 Viewer adapter/
frame-guard cases passed, with clean compilation. The later scoped repair gate passed60 checks and strict
Clippy/format0; prior113/86 overlap. The earlier native CEF attempt rendered
content but failed a resource-policy assertion; that failed snapshot remains
historical. The coherent CSS/
reader/diagnostic checkpoint later passed86 scoped headless checks. Worker image
review found readable minimum-width columns with horizontal scrolling. Its native
navigation now passed all11 content proofs with expected generated requests only
and no denied/unexpected requests. Parent-change then failed with the browser
exiting before requested evidence. The separately authorized close/cancel case
then passed once on the same old harness. Those two native passes/one failure
retain their source identity. The latest parent diagnostic `office-9fe36f20a6` remains **0 passed /1 failed**:
completed real Find returned0, Copy failed and Session stayed live. Its blank902×602
diagnostic frame is not validated fresh content/resize proof (resize_ok=false,
fresh_contrasted_frame=false). Source refusal remains required; no provider/Find/
loading root cause is established. Host47 +Viewer44 =91/0 retain their frozen source;
the later four assertion-style lint fixes passed strict Clippy0/format13files0 and
two overlapping resource tests, not93 unique tests or a full91 rerun.

Root accepted the generic provider small-body navigation contract for private
implementation: captured main-frame GET/no-post targets of0..1MiB are completely
admitted through genuine Resolve/ResourceRead/Data before replacement navigation;
larger targets retain streaming through the same resolved capability. Small-body
failure cannot fall back to streaming/old bytes. Preserve one body credit through
actual response release, exact one-shot request binding, a30-second admission
deadline and current authority checks. This changes no public API/SDK or dependency
pins. The strong retained-document boundary applies to eligible small targets;
large streaming retains its late-Read-denial limitation. Office V3's exact16 sources are now canonical (freeze `10d0a5e6…`), with
132 nonowned guards unchanged and public API/SDK, Cargo, executable and runtime
preserved. The unchanged pure navigation core `c6596c4c…` passed **34/0**, compile0;
the existing CEF version-metadata manifest warning remains, so this is not a
zero-warning gate. These tests exclude CEF/native execution and do not qualify
new V3 route/adapter integration. Five native cases await a legitimate complete
new C product; no new executable or deployment is claimed.

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

Generated preview pages use bounded inert HTML. The preview does not require
external Office, run macros, evaluate formulas or refresh external data. A sheet
may show a stored formula result or inert formula text. Find searches only the
currently generated page, not the complete workbook or document. See
[Viewer protocol](VIEWER_PROTOCOL.md) and the [current checkpoint](../ACTIVE_WORK.md#current-checkpoint)
for implementation and qualification boundaries.
