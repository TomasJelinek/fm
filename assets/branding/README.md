# Midnight Explorer application icon

The default application icon, Harvest Navigator, pairs an amethyst-and-ivory compass needle
with a gold crescent on a blue folder. It shares Origami Trail's copper and teal tab stripes. The transparent
background works on both light and dark desktops. This is application branding; file
and drive icons continue to come from the operating system.

- `master.png`: the selected generated Harvest Navigator artwork, retained for future
  exports.
- `midnight-explorer.png`: 256-pixel window icon, embedded by Slint. On Wayland,
  the compositor uses the installed desktop entry and this PNG instead; see the
  [Linux launcher instructions](../../docs/USER_GUIDE.md#linux-desktop-launcher).
  Linux release archives and `scripts/install-linux-desktop.py` install this file.
- `midnight-explorer.ico`: Windows executable icon with 16, 20, 24, 32, 40, 48, 64,
  128 and 256-pixel frames. Embedded at build time using the Windows SDK resource
  compiler; the application does not need to load an external icon file at runtime.

The artwork was created with the built-in image generation tool; the design brief and
its final color refinement are retained in [PROMPT.md](PROMPT.md#harvest-navigator).
Harvest Navigator replaced the earlier Moon icon as the default in 0.6.0.

To regenerate the size exports on Windows, run `powershell -NoProfile -File
scripts/build-app-icon.ps1` from the repository root. It scales `master.png` to each
size and packages the frames; it does not call an image service. Normal project builds
on Windows and Linux use the committed exports and do not run this maintenance step.
No intermediate artwork is written outside `_build`; the three files here are
intentional source assets. Future macOS application packaging can derive ICNS
sizes from the same master once that platform is implemented.

## Icon shortlist

Choose a design in **Options → Appearance → Application icon**. Harvest Navigator is
the default. Selecting a design previews it on the toolbar and in app windows.
Choose **OK** to keep it across launches or **Cancel** to restore the saved icon.
Windows and X11 support per-window icons. On Wayland the shell uses the icon from
the installed desktop entry. Windows shortcuts can also have their own icon.

| Preview | Icon | Design | Files |
| --- | --- | --- | --- |
| <img src="midnight-explorer.png" width="80" alt=""> | Harvest Navigator (default) | An amethyst-and-ivory compass needle and gold crescent sit on a blue folder with two closely spaced tab stripes. | [PNG](midnight-explorer.png) · [Windows ICO](midnight-explorer.ico) |
| <img src="variants/origami-trail/midnight-explorer.png" width="80" alt=""> | Origami Trail | The split window pairs a thumbnail grid with a file list. An angular gold fold crosses the violet cover. | [PNG](variants/origami-trail/midnight-explorer.png) · [Windows ICO](variants/origami-trail/midnight-explorer.ico) |
| <img src="variants/interleave/midnight-explorer.png" width="80" alt=""> | Interleave | Smooth turquoise and deep teal sheets interlock around a gold fold engraved with a tiny crescent. | [PNG](variants/interleave/midnight-explorer.png) · [Windows ICO](variants/interleave/midnight-explorer.ico) |

To use a design on a Windows desktop shortcut, open **Properties → Shortcut →
Change Icon** and select its ICO file. To rebuild an alternate design's PNG and ICO
from its `master.png`, run the export script from the repository root with its folder
name. For example, to rebuild Origami Trail, run
`powershell -NoProfile -File scripts/build-app-icon.ps1 -Variant origami-trail`.
