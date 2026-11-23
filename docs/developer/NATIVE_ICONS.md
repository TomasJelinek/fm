# Native icons

How file, folder, drive, location and action icons reach the screen at the size
they are drawn at, and where a file's own artwork is read. Internal contract; the
traits are `midnight_core::IconService` and `IconEdges`, not plugin ABI.

## Rule

Every icon is drawn from a source at least as large as its box in device pixels
(logical size times the display's scale factor), up to 256 px (`MAX_ICON_EDGE`).
A provider reduces its source once, in premultiplied alpha, to exactly that edge,
so the renderer draws it one to one. Artwork is enlarged only when nothing larger
exists, and then from the largest source there is.

## Edges

`midnight_ui::icon_view` reads the boxes from the window in device pixels,
rounded to whole pixels and capped at 256, and the thumbnail pulse sends them as
`Command::SetIconEdges` whenever density, fonts, view modes, the window's size or
the scale factor change them:

| Place | Edge (`IconEdges`) | At 100 % | At 225 % |
| --- | --- | ---: | ---: |
| Details and Brief rows, tree rows | `entry`: `Theme.file-icon-size` | 20 | 45 |
| Thumbnails tiles of files without a preview | `tile`: the thumbnail size; 0 without tiles | 96 | 216 |
| Drive bar, location and bookmark choices | `location`: `Theme.location-icon-size` | 20 | 45 |
| Toolbars, menus, tabs and dialog titles | `action`: the largest action box, 32 px | 32 | 72 |

Each place's edge is its own size class. Action art is one bitmap for every
action box: a dialog title draws it at its edge, smaller boxes reduce it, none
enlarges it.

The backend's icon actor treats a request as naming an icon (`IconKey` at
`DEFAULT_EDGE`) and loads it at the edges its kind is drawn at (`IconEdges::of`,
through `IconService::icon_edge`). It remembers the edges of its 256 most recently
requested icons (`DESKTOP_ICON_CAPACITY`), so a listing asks for each type once. After an edge change it
loads only what is missing, most recently requested icon first, behind any new
request. The UI keeps one bitmap per icon and edge and gives each place the bitmap
of its edge, else the next larger, else the largest: a placeholder shows at once
and sharpens when the bitmap of its edge arrives.

## Thumbnails tiles

A tile keeps its size: at least the thumbnail size plus 60 px wide (the panel's grid
width shared by its columns) and the thumbnail size plus 30 px and three lines of the
font tall, with the name in the lower part. An icon (folder, file type or a program's
own artwork) is a square of the thumbnail size 12 px below the tile's top. An image
or video preview (`FileRow.preview`) fills the preview box instead
(`ThumbnailTile` in `file-entry.slint`): the tile's width less the name's 10 px
insets, and the thumbnail size plus 6 px of height, from 6 px below the tile's top to
the icons' baseline 6 px above the name. It keeps its aspect ratio (nothing is
cropped), is centred, and stands on that baseline, the same at every density and font
size. Each pane reports the longest side a preview of at most 2:1 is drawn at
(`FilePane.preview-edge`, `midnight_ui::tile_preview_edges`), at least the thumbnail
size; previews are requested at that edge times the scale factor, in the resolution
buckets of the [worker SDK](WORKER_SDK.md), so they are decoded at least as large as
they are drawn. A wider panorama is drawn slightly enlarged. A program's artwork keeps
the thumbnail size. The cache key carries the bucket, so a larger request is a miss
that is decoded and stored beside the smaller one, and a larger stored image serves a
smaller request; the budgets stay as they are (the UI's 54 MiB and 4,096 images, the
backend's 4 MiB in memory, 512 MiB and 8,192 files on disk). With previews filling
their tiles, both panes' full grids on a dense display can exceed the UI's pixel
budget; an upgrade that does not fit then waits, the smaller image staying, until the
view changes.

## Windows

Type, stock, known-folder and drive icons are entries of the Shell's system image
lists. The entry comes from:

- a synthetic name, `type` or `type.<extension>`, with `SHGFI_USEFILEATTRIBUTES`
  (directory attributes for folders), so the Shell classifies it by name and never
  opens a file, probes media or touches a network path;
- `SHGetStockIconInfo` with `SHGSI_SYSICONINDEX` for drives of removable, optical,
  network, RAM and unknown kinds and for semantic actions;
- a known folder's PIDL (`SHGFI_PIDL`) for locations and the Recycle Bin;
- a discovered, validated local fixed drive root, the one path ever passed.

The four lists (`SHIL_SMALL`, `SHIL_LARGE`, `SHIL_EXTRALARGE`, `SHIL_JUMBO`)
follow the system DPI: 16, 32, 48 and 256 px at 96 DPI; 36, 72, 108 and 256 px at
216 DPI. An icon is drawn from the smallest list at least as large as its edge.
The Shell keeps artwork without a 256 px image in the jumbo list as the next
smaller list's bitmap in the top-left corner of an empty canvas; such an entry is
drawn from that smaller list instead. The entry is drawn at its list's size on
black and on white, which recovers straight alpha also from mask icons, and
`midnight_platform_common::icon_scale::fit` reduces it with a Lanczos filter in
premultiplied alpha (Catmull-Rom when enlarging).

- Links (`IconKind::Link`, `FolderLink`) are the stock document or folder entry
  with the Shell's shortcut overlay (`IDO_SHGIOI_LINK`) drawn by the list at every
  size. No link path or target is passed to the Shell.
- A file type is `generic` when it shares the stock unassociated document's entry.
- Transparent artwork, and opaque black placeholders of semantic roles, are
  reported as unavailable, so the UI keeps its own vector art.

Program and icon-file artwork (command buttons, the icon picker and program tiles)
is read from the icon group itself. A module is mapped as a data file and image
resource (`LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE`); an icon file
is read whole, at most 16 MiB (`MAX_ICON_FILE_BYTES` in `icons_executable.rs`). Frames are ranked for the edge: the smallest frame
at least that large, else the largest; among frames of one size the one with the
most colors. The system decodes the frame at its own size, including PNG frames
of 256 px and more (`CreateIconFromResourceEx`), and it is reduced once. Bounds:
64 frames per group, 4 MiB per frame, 1,024 px per side (`MAX_FRAMES`,
`MAX_FRAME_BYTES`, `MAX_FRAME_EDGE`). Formats read only in
part, such as 16-bit programs and icon libraries, fall back to the Shell's own
extraction at the edge (`SHDefExtractIconW`).

## Linux

The desktop's icon theme (XSETTINGS, then `gtk-3.0/settings.ini`), the themes it
inherits depth first, then Adwaita, breeze and hicolor, and `/usr/share/pixmaps`
last. Within a theme the directories are ranked for the edge in device pixels
(`Size` times `Scale`):

1. directories of exactly that size;
2. scalable directories (`Type=Scalable`) whose `MinSize` to `MaxSize`, times
   `Scale`, holds it;
3. larger directories from the nearest up, a scalable one counted at its
   `MinSize`;
4. smaller ones from the nearest down, a scalable one counted at its `MaxSize`;
5. directories of unknown size.

Each directory is searched for `<name>.png`, then `<name>.svg`. A PNG is reduced
once (`icon_scale::fit`); an SVG is rendered at exactly the edge by the resvg 0.48.1
dependency, with the bounds of explicitly chosen artwork (UTF-8 only, no DTD, 4,096
elements and a bounded expansion of `<use>` references, no external images or fonts),
except that a theme SVG may have 512 KiB instead of 128 KiB (`MAX_THEME_SVG_BYTES` in
`icon_svg.rs`). Files resolve only inside the fixed system icon roots. A lookup tries at
most 16,384 files (`MAX_PROBES` in `icons.rs`). Linux reads no listed
file's content for artwork.

## Per-file artwork

On Windows a program's tile in the Thumbnails view shows the program's own icon
(`IconService::load_file_icon`, `.exe` only); its rows keep the type icon. This is
the one place a listed file's content is read for an icon, under the rules of a
preview:

1. The tile is a visible thumbnail target (`ThumbnailContent::FileIcon`), within
   the shared budget of 256 targets per view, only where previews are enabled and
   the pane's storage allows them (the network preview opt-in applies).
2. The UI sends `Command::SetFileIconView` with the visible program tiles that
   still lack artwork at their device edge. It sends a new view only when the
   visible tiles or their edges change, so artwork arriving for the current view
   never cancels the load in progress, and scrolling does.
3. The backend's icon actor loads one tile at a time and takes every other turn
   with type icons and command artwork. A new view cancels the load in progress
   and drops the queued tiles of the old one. A program read without artwork of
   its own (most of System32's) is remembered by its tile identity, at most 4,096
   of them, and is not read again until it is listed as another file revision.
4. Each tile is read between two revalidations of a native capture of the file.
   The file is opened without following a link, must be an ordinary disk file,
   and cannot be written, renamed or deleted while its resources are read through
   the handle's final path. No code in it runs.
5. `Event::FileIconLoaded` carries the view's generation; the UI admits only
   artwork of the current view for a tile still visible.

The artwork is retained like a preview, in the thumbnail image cache: least
recently used first, at most 54 MiB of pixels and 4,096 images (`MAX_BYTES`, `MAX_IMAGES`
in `thumbnail_image_cache.rs`), and visible
images are never evicted to make room. A 216 px tile costs 182 KiB, so a full
request budget of 256 program tiles (45.6 MiB) fits; artwork at least as large as
a tile also covers it after a change of scale.

## Memory

Type icons are shared per type and edge. Each costs edge² × 4 bytes per drawn
place: a type drawn as a 45 px row and a 216 px tile 190 KiB. The engine
remembers 256 icons and the UI holds them all at their edges (about 48 MiB for
256 types with 216 px tiles, about 2 MiB with 45 px rows only); its 128 MiB
ceiling (`IMAGE_CACHE_BYTES` in `presenter/desktop.rs`) only stops bitmaps of edges
nobody draws from piling up.

## Tests and measurement

Windows (`midnight-platform-windows`): `icons::sized_tests` (seven keys at every edge
from 16 to 256 px; large icons carry more sharp steps than an enlarged small bitmap;
jumbo corner placement drawn from the true bitmap; generic types at row and tile
edges), `icons::executable::tests` (frame ranking; group, file and header parsing;
frames chosen by edge from an icon file and from a program built with known frames;
only the largest frame is enlarged; cancellation; links and directories are not
read), `icons::tests` (link overlays at 32 and 200 px; drive roots), and
`icons::icons_measure`, ignored by default, which measures a cold and a warm pass over
System32 in a release build. Platform-neutral: `icon_scale::tests` in
`midnight-platform-common`. Engine: `desktop::file_icons::tests` (bounded views,
capture and revision checks, order and edges, cancellation by a newer view, programs
without artwork read once, turns with type icons) and the icon actor's edge tests in
`desktop::command_icons::tests`. UI: `thumbnail_presentation::file_icons::tests`
(device edges by scale, a view sent once and again only when the visible tiles
change, stale artwork refused, a cleared cache, retention within the preview byte
budget) and `icon_view::tests` (the action edge follows the scale). Linux
(`midnight-platform-linux`): `icons::theme::tests` (directory ranking with scalable
ranges and scales) and `icons::tests` (SVG artwork rendered at the exact size before a
larger bitmap is reduced, SVG in fixed directories and pixmaps, oversized SVG
refused). Measurements: [native icon record](../verification/2026-11-21-native-icon-resolution.md).
