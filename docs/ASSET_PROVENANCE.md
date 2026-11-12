# Asset provenance

This record describes where assets came from; it is not a legal clearance opinion
or a change to the repository's license.

| Asset | Origin | External reference or copied artwork |
| --- | --- | --- |
| `crates/midnight-ui/ui/icons/checksums.svg` | Hash and check-mark strokes authored directly as SVG for this project, 8 November 2026; viewBox 24×24. | No external artwork or icon set used; conventional checksum/verification symbols. |
| Tokyo Night, One Dark, Rosé Pine/Dawn, Catppuccin Latte, Norton Commander, Amber Terminal and Verdant Light theme colors | Semantic palette adaptations and original retro/green color combinations added 8–9 November 2026. Amber Terminal replaces DOS Commander. | [Exact modern palette sources, licenses and modifications](THEME_PROVENANCE.md); retained license files accompany distributions. Original palettes include no copied product code, screenshot pixels, fonts, logos or icons. |
| Slate Blue, Slate Teal and Slate Violet theme colors | Original neutral charcoal palettes with blue, teal and violet accents, authored for this project on 11 November 2026. | The user-supplied screenshot informed the surface hierarchy and blue accent. No screenshot pixels, external palette, theme code, artwork, fonts or icons were copied. [Palette provenance](THEME_PROVENANCE.md#slate-variants-11-november-2026). |
| `crates/midnight-ui/ui/icons/find.svg` | Circle and handle authored directly as SVG for this project, 7 November 2026; viewBox 24×24. | No external artwork or icon set used; conventional magnifying-glass symbol. |
| `crates/midnight-ui/ui/icons/filter.svg` | Funnel outline authored directly as SVG for this project, 8 November 2026. | No external artwork or icon set used; conventional filter symbol. |
| `crates/midnight-ui/ui/icons/archive.svg` | SVG paths and shapes authored directly by the assistant for this project, 7 November 2026. A gold archive box with a zipper; viewBox 24×24. | None used. The box/zipper is a conventional archive symbol, not a reproduction of a particular product icon. |
| `crates/midnight-ui/ui/archive-badge.slint` | Small zipper paths authored directly for this project, 7 November 2026; conventional alternating teeth and pull, with a theme-aware contour. | User-supplied zipped-folder screenshot was a motif reference. No screenshot pixels, external vector paths or icon package were copied. |
| `crates/midnight-ui/ui/icons/attributes.svg` | SVG paths authored directly by the assistant for this project, 7 November 2026. A folded document with a clock, recolored by the active theme; viewBox 24×24. | No external artwork or icon set used. Used in the attributes/time/case toolbar, function action and dialog header. |
| `assets/branding/moon-master.png` and its PNG/ICO exports | Created with the built-in image generation tool on 6 November 2026, as recorded in the [branding README](../assets/branding/README.md) and [generation prompts](../assets/branding/PROMPT.md). | The recorded brief specifies a light/dark moon; it does not name another product's artwork as a reference. This records the creation process, not a guarantee of uniqueness. |
| Windows file, drive, folder and action icons | Loaded at runtime through Windows Shell APIs, including `SHGetFileInfoW` and `SHGetStockIconInfo`. | Operating-system assets; they are not original project artwork. |
| Linux file, drive, folder and action icons | Loaded at runtime from installed, trusted desktop icon themes. | Theme authors' assets, under their respective terms. |

The new archive asset is embedded as vector artwork and used when an archive has
only a generic document icon. Associated application icons retain precedence.
Existing project assets not individually listed above have not been retroactively
certified as original by this record. Third-party library and bundled asset
notices remain in [THIRD_PARTY.md](THIRD_PARTY.md) and distribution packages.

## SVG worker font fallback, 12 November 2026

The independent shared image decoder embeds unmodified Noto Sans Regular, Arabic
and Hebrew font files from the pinned cosmic-text 0.19.0 source archive. They
remain under SIL Open Font License 1.1, with exact per-file SHA256 hashes and
upstream/source information in [the font provenance](../plugins/image-decoder/fonts/README.md).
The license and provenance accompany both image-worker distributions. These fonts
are loaded from embedded memory only; no document-selected or installed font path
is opened. Coverage and family fallback are explicitly limited as documented.
