# Midnight Explorer application icon

A moon with a silver illuminated half and a midnight-blue shaded half. The
transparent background works on both light and dark desktops. This is application
branding; file and drive icons continue to come from the operating system.

- `moon-master.png`: original generated artwork, retained for future exports.
- `midnight-explorer.png`: 256-pixel window icon, embedded by Slint. On Wayland,
  the compositor uses the installed desktop entry and this PNG instead; see the
  [Linux launcher instructions](../../README.md#linux-desktop-launcher).
- `midnight-explorer.ico`: Windows executable icon with 16, 20, 24, 32, 40, 48, 64,
  128 and 256-pixel frames. Embedded at build time using the Windows SDK resource
  compiler; the application does not need to load an external icon file at runtime.

The assets were created with the built-in image generation tool on 2026-11-06;
the exact prompts are retained in [PROMPT.md](PROMPT.md).
The generation brief was a centered full moon with roughly equal light silver and
dark slate-blue sides separated by a gently curved vertical boundary, restrained
crater shapes, a transparent background, clear small-size readability, and no text
or additional objects. A second pass requested a cleaner outer silhouette while
preserving the moon and shading.

To regenerate the size exports on Windows, run `powershell -NoProfile -File
scripts/build-app-icon.ps1` from the repository root. This only scales and packages
the committed master; it does not call an image service. Normal project builds on
Windows and Linux use the committed exports and do not run this maintenance step.
No intermediate artwork is written outside `_build`; the three files here are
intentional source assets. Future macOS application packaging can derive ICNS
sizes from the same master once that platform is implemented.
