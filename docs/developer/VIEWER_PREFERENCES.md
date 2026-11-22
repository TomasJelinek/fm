# Viewer preferences

Viewer choices are application-owned settings. They do not change the independent
plugin API, grant plugins settings-file access or store document contents. The
host captures `Arc<ViewerPreferences>` in `ViewerLaunch` before opening a source.

## Plugin catalogs in Options

Options → Plugins has Viewers, Filesystems, Archives and Thumbnailers tabs. The Filesystems and Archives tabs are read-only catalogs (Archives also opens Archive associations); the Thumbnailers tab edits each thumbnailer's extensions.
Viewer enablement, extension overrides and priority keep stable plugin IDs and
their restart and application-settings semantics. Short names and functional
descriptions are presentation metadata; they do not redirect routing or migrate
saved IDs. The source viewer's description identifies its supported lexical
profiles rather than placing resource-limit wording in its title.

The Extensions editor is a single-line field. Long lists scroll horizontally
with the caret, so Home/End and arrow keys reach either end without clipped
wrapped lines. Spaces, commas and semicolons remain supported separators;
multiline clipboard input becomes space-separated text. Validation, Reset,
preview, OK and Cancel keep their transaction semantics. Options starts at 840×660
logical pixels, grows with larger fonts (at least 28 by 23 font sizes) and stays within the
window, 12 pixels from each edge (`main.slint`); users can still resize it. Evidence:
[layout record](../verification/2026-11-11-options-extension-layout.md).

The internal `Command::ListViewerPlugins` requests all three catalogs
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
reload control. Archive decoder settings are on the Archives page.
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
| PDF zoom | Fit on each new source | The document zoom field is read and written but not restored |

Manual scale uses integer basis points: 10,000 means 100%, with 0.01% stored
precision. Font sizes are 8–48 logical pixels; family names are nonempty Unicode
strings, at most 256 UTF-8 bytes and without control characters. These are settings
resource bounds, not native filename/path limits. Invalid values fail atomically.

Each newly opened PDF starts in Fit, including file cycling to another PDF. Saved
document zoom values stay valid in the settings schema but do not set a new
document's view, and a late settings load leaves the current PDF geometry unchanged.
Explicit zoom commands control the current document, and its first zoom step starts
from the displayed scale. Raster images still restore their separate
saved zoom.

PDF Fit enlarges small pages and shrinks large pages to the viewport, with a
maximum scale of 64× and a bounded raster size. Raster-image Fit keeps
its no-upscale policy. The document Fit policy survives page changes and resizing.

Changing PDF pages retains the current manual zoom or actual-pixels mode in both
directions, including pages with different dimensions. The backend preserves the
runtime scale directly, without rounding it through the persisted basis points.
Fit remains a mode and recomputes its scale for the new page. Page navigation
starts at the new page's top; dimensions smaller than the viewport are centered.
It does not change the saved zoom preference. This policy belongs to the viewer
backend; the independent PDF plugin renders the supplied page and viewport.

## Failure notices

A viewer failure that carries diagnostic details shows a notice in the page's place,
laid out like the HTML viewer's sandbox notice: the message and one Copy details button
with the notice dialog's caption, copy icon and hint ("Copy the complete message and
diagnostic report to the clipboard."), and no shortcut. Copy details copies the
message, a blank line and the details, unchanged, through the viewer's copy, which
sends the engine's clipboard command. The status line keeps showing the message, and a
failure without details stays on the status line only. The details never quote file
content or name a path the message does not show. Office refusals are the only
failures with details so far (`ViewerSandboxNotice::details`, see
[refusals](#office-packages-what-opens-and-what-is-refused)); other viewers leave them
empty.

The notice never takes the focus, so the viewer's keys keep working while the content
has it: Tab from the content reaches Copy details, and Return or Space activates it.
Copy details keeps its row at the bottom of the notice; a message too long for the
space above it scrolls, and the insets shrink first, so the button stays whole at the
viewer's minimum size and with the largest UI font. Another document removes the
notice.

## PDF layouts

Use **Paged** to view one PDF page at a time, with the previous/next page buttons
or **Ctrl+PageUp / Ctrl+PageDown**. Use **Continuous** to scroll through a sequence
of pages. **Fit**, **1:1** and the zoom controls remain beside the layout buttons.
The backend renders retained tiles at the exact zoom and the view scrolls over them
([viewer scrolling](VIEWER_SCROLLING.md#documents-pdf)).

The layout chosen last is a viewer preference, application-wide
(`ViewerPreferences::document_layout`, `ViewerPreferenceChange::DocumentLayout`). A
click on Paged or Continuous saves it at once, like every viewer preference, before
the shown document has switched; the next PDF opens in it, in Fit at the top of its
first page, also after a restart. Settings files store it from schema 36 (written only
for Continuous; see [settings files](../SETTINGS_FILES.md)), and settings export and
import carry it. A settings change does not re-lay out the shown document: the
layout buttons own it. The Options dialog has no viewer presentation page (fonts,
Markdown preset, zoom and layout are all chosen in the viewer), so the layout is not
shown there either.

The zoom percentage is also a field for a typed value: see
[keyboard shortcuts](../KEYBOARD_SHORTCUTS.md) (Ctrl+Y). A typed zoom is one zoom
command anchored at the view's centre and saved like the zoom buttons' commands. The
user's decimal separator comes from the application's number format
(`ViewerWindows::set_number_locale`).

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
uses the backend settings command, coalescing and atomic file publication. Queue
rejection keeps the latest unsaved values for the save retry and shutdown flush. A failed initial read never authorizes overwriting the old file.

Startup reveals the main window after 10 seconds without a settings answer
(`startup::DEADLINE` in `midnight-explorer`), so a
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
families and System monospace, using the backend renderer's cached font catalog. Unknown or proportional families cannot be selected. If a stored family
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

The [source/Markdown presentation controls](VIEWER_PRESENTATION.md) expose these
choices. Evidence: [viewer controls record](../verification/2026-11-08-viewer-controls.md).

## Office contents preview

The built-in Office preview opens readable text, tables or sheet contents at once, with
navigation to further bounded content pages, for DOCX, XLSX, PPTX, ODT, ODS and ODP and
the variants below. It shows readable contents, not faithful Office page layout.
Generated preview pages use bounded inert HTML. The preview does not require external
Office, run macros, evaluate formulas or refresh external data. A sheet may show a
stored formula result or inert formula text. Find searches only the currently generated
page, not the complete workbook or document. See [Viewer protocol](VIEWER_PROTOCOL.md).

Root accepted the generic provider small-body navigation contract for private
implementation: captured main-frame GET/no-post targets of 0..1MiB are completely
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

### Office packages: what opens and what is refused

The preview opens packages saved by Microsoft Office, which pads the local headers of
its first parts with a 0xA220 "growth hint" extra field (520 bytes on
`[Content_Types].xml` and `_rels/.rels`, 264 on some others).

**Container.** `midnight-office-preview` checks the ZIP container itself before
`zip` indexes it ([preflight.rs](../../crates/midnight-office-preview/src/preflight.rs)):

- The end record is the last one whose comment reaches the end of the file. An
  archive comment is skipped, unless it contains ZIP record signatures that could
  pass for a second directory.
- Every central record and its local header must agree on name, flags, compression
  method, CRC and sizes (or the data descriptor's values). Members lie before the
  central directory and never overlap. Each member's range is checked before its
  name and extra field are read, so all names and extra fields read together are
  at most the file's size; beyond them a member costs its 30-byte header and at most
  24 bytes of data descriptor.
- An extra field must be an exact sequence of (ID, size, data) records: a cut
  record header, a record longer than its field or a trailing byte is a damaged
  file. Records the preview does not use are skipped: 0xA220 growth hints, 0x5455
  and 0x000A times, 0x7875 Unix IDs, 0xCAFE, 0xD935 and 0xA11E alignment, and every
  unknown ID. 0x0001 ZIP64 supplies each 0xFFFFFFFF size or offset and a 0xFFFF disk
  number, in APPNOTE order; a local ZIP64 record holds both sizes and makes the
  data descriptor's sizes eight bytes. A repeated ZIP64 record or a missing value is
  a damaged file. A current 0x7075 Unicode path record that names a different part
  is refused; a stale one (CRC of another name) is skipped. Entry comments are
  skipped.
- `zip` indexes a canonical copy of the central directory: resolved 32-bit sizes
  and offsets, the validated name, no extra fields and no comments. Its own
  extra-field reading, which rejects NTFS records of another length, fails stale
  Unicode path records and renames a member after a current one, never applies, and
  it retains no extra field or comment for the index memory reservation.
- Limits: a 256 MiB file, 10,000 parts, an 8 MiB central directory
  (extra fields and comments count), 4,096-byte names and 1 MiB of names, a
  16 MiB index reservation and 256 MiB of reads and unpacked data per page. A part
  whose ZIP64 size exceeds 4 GiB is refused. The preflight reads at most the last
  65,557 bytes for the end record, the central directory, and each member's header,
  name, extra field and data descriptor.

**Library decision.** `zip` 8.6.0 keeps member lookup and
Deflate decoding; no other ZIP library is in `Cargo.lock`. It cannot replace the
preflight under the same limits: it allocates its index from the end record's
counts, searches for another end record after a central-directory error, never
compares local and central headers or detects overlapping or duplicate members, and
its extra-field parser is crate-private and differs from the rules above (see the
canonical copy). Local extra fields it only skips.

**Contents.** Only the parts and elements that carry text and stored values are
read; other writers' markup is skipped whole and never shown as document text.

- XLSX reads `workbook/sheets/sheet`, worksheet `sheetData` cells and shared
  strings. Views, names, calculation settings, compatibility (`mc:`) and revision
  markup, extension lists, formatting, filters, tables and print settings are
  skipped. Charts, pictures, comments, form controls and OLE objects are skipped and
  the sheet page says "Some charts, pictures, comments or other contents of this
  sheet are not shown." Chart, dialog, macro and external sheets keep their place
  in the catalog; their page says what they are, for example "This sheet is a
  chart, which the preview does not show."
- DOCX, PPTX, ODT and ODP pages with only pictures or objects say "This slide
  contains only pictures, charts or other objects that the preview does not show."
  (or "This part of the document …") instead of failing. Extension lists (creation
  IDs), slide backgrounds, transitions and timing, and LibreOffice's form, field and
  change-tracking declarations do not claim omitted objects.
- ODS skips scripts, forms, conditional formats and other sheet-level markup;
  shapes, charts, pictures and comments are skipped with the sheet notice. A cell
  value type the preview does not know shows the cell's paragraphs.
- Template, macro-enabled and (for presentations) show main parts open in their
  family; the main part's content type may also come from the Default for its
  extension. VBA projects and ODF `Basic/` and `Scripts/` libraries are never read
  or run, and a document that embeds a macro-enabled workbook is not refused. The
  viewer chooses the family by suffix (`docm`, `dotx`, `dotm`; `pptm`, `ppsx`,
  `ppsm`, `potx`, `potm`; `ott`, `ots`, `otp`), for files a user associates with the
  preview; the default associations still list only `docx`, `xlsx`, `pptx`, `odt`,
  `ods` and `odp`.

**Refusals.** The status line shows `OfficeError::message(format)`, which names the
kind of document and a reason in plain words. `OfficeError::details()`, the internal
reason, becomes the failure's details, so a document that fails to open also shows its
message in the page's place with Copy details ([failure notices](#failure-notices)),
for example "Encrypted Office package" or "Malformed Office package: central record
bounds" after the message. A cancelled preview has no details. A page that fails while
the user navigates within an open preview stays on the status line.

| Case | Status line (for a presentation) |
| --- | --- |
| ZIP or ODF encryption | This presentation is password-protected, so the preview cannot open it. |
| OLE compound file | … is password-protected or saved in the older binary Office format (.doc, .xls or .ppt), which the preview cannot open. |
| Compression other than stored or Deflate | … uses a compression method other than Deflate, which the preview cannot read. |
| Unknown ZIP general-purpose flags | … uses ZIP storage options that the preview does not support. |
| Split archive | … is part of a ZIP archive split into several files, which the preview cannot open. |
| ZIP64 end records | … is stored in the ZIP64 format for very large archives, which the preview cannot open. |
| Comment with ZIP record signatures | … hides a second archive directory in its ZIP comment, so the preview does not open it. |
| Non-UTF-8 or unflagged non-ASCII part names | … names its parts in a legacy character encoding instead of UTF-8, which the preview does not support. |
| Binary workbook main part | This workbook is a binary Excel workbook (.xlsb), which the preview cannot open. |
| XML in UTF-16 or binary form; another declared encoding | … contains XML in UTF-16 or binary form … / … declares an XML text encoding other than UTF-8 … |
| DTD; processing instruction | … contains an XML document type definition (DTD), which the preview refuses for safety. / … contains XML processing instructions … |
| Damaged or wrong-format package (malformed extra field, header mismatch, overlap, another family's main part) | This presentation is damaged or is not a valid PowerPoint (.pptx) file, so the preview cannot show it. |
| A limit | for example: This presentation is larger than 256 MiB, more than the preview opens. |
