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

`ViewerPreferenceChange` describes one changed field. The viewer service accepts
and validates the small configuration update independently of rendering and file
I/O. The window invokes the application's preference listener immediately after
acceptance, so an immediate Escape does not depend on another rendered frame to
save that change. Programmatic restoration does not invoke this listener.

The presenter merges each patch into the latest application settings, preserving
unrelated changes from other viewer windows. During the initial settings read,
at most one patch per field is retained; those patches merge into the loaded
configuration rather than replacing unedited values with defaults. Persistence
uses the existing backend settings command, coalescing and atomic file publication.
Queue rejection retains the latest unsaved values for the existing save retry and
shutdown flush. A failed initial read never authorizes overwriting the old file.

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
