# Asset provenance

This record describes where assets came from; it is not a legal clearance opinion.

| Asset | Origin | External reference or copied artwork |
| --- | --- | --- |
| Harvest Navigator (`assets/branding/master.png` and its `midnight-explorer.png`/`.ico` exports, the default application icon), Origami Trail and Interleave in `assets/branding/variants/` | Created with the built-in image generation tool for Midnight Explorer. Each directory contains the selected master and its PNG and ICO exports. | Design briefs and refinements are recorded in the [generation prompts](../assets/branding/prompt.md#selected-launcher-candidates). |
| `crates/midnight-ui/ui/icons/checksums.svg` | Hash and check-mark strokes authored directly as SVG for this project, 8 November 2026; viewBox 24×24. | No external artwork or icon set used; conventional checksum/verification symbols. |
| Tokyo Night, One Dark, Rosé Pine/Dawn, Catppuccin Latte, Norton Commander, Amber Terminal and Verdant Light theme colors | Semantic palette adaptations and original retro/green color combinations added 8–9 November 2026. | [Exact modern palette sources, licenses and modifications](theme-provenance.md); retained license files accompany distributions. Original palettes include no copied product code, screenshot pixels, fonts, logos or icons. |
| Slate Blue, Slate Teal and Slate Violet theme colors | Original neutral charcoal palettes with blue, teal and violet accents, authored for this project on 11 November 2026. | The user-supplied screenshot informed the surface hierarchy and blue accent. No screenshot pixels, external palette, theme code, artwork, fonts or icons were copied. [Palette provenance](theme-provenance.md#slate-variants-11-november-2026). |
| `crates/midnight-ui/ui/icons/find.svg` | Circle and handle authored directly as SVG for this project, 7 November 2026; viewBox 24×24. | No external artwork or icon set used; conventional magnifying-glass symbol. |
| `crates/midnight-ui/ui/icons/filter.svg` | Funnel outline authored directly as SVG for this project, 8 November 2026. | No external artwork or icon set used; conventional filter symbol. |
| `crates/midnight-ui/ui/icons/pause.svg` | Two vertical strokes authored directly as SVG for this project, 20 November 2026; viewBox 24×24, tinted by the active theme like every action glyph. | No external artwork or icon set used; conventional pause symbol. |
| `crates/midnight-ui/ui/icons/sound.svg`, `sound-off.svg` | A speaker outline with two sound arcs, and the same speaker with a cross, authored directly as SVG for this project, 26 November 2026; viewBox 24×24, tinted by the active theme. The video viewer's Mute and Unmute button. | No external artwork or icon set used; conventional speaker and muted-speaker symbols. |
| `crates/midnight-ui/ui/icons/browse.svg` | A folder outline and three dots authored directly as SVG for this project, 24 November 2026; viewBox 24×24, tinted by the active theme. A field's square Browse button (`FieldButton`). | No external artwork or icon set used; conventional folder and ellipsis symbols. |
| `crates/midnight-ui/ui/icons/chevron-right.svg` | A right-pointing chevron stroke authored directly as SVG for this project, 24 November 2026, as the existing `chevron-down.svg` turned a quarter; viewBox 8×12, tinted by the active theme. A closed section disclosure (Custom Commands), where the font had no "▸". | No external artwork or icon set used; conventional disclosure symbol. |
| `crates/midnight-ui/ui/icons/view-detailed.svg`, `view-brief.svg`, `view-thumbnails.svg` | Strokes and rounded squares authored directly as SVG for this project, 24 November 2026; viewBox 24×24, tinted by the active theme. The panel's compact View picker shows the shown view's glyph (rows with a column, two columns of rows, a grid). | No external artwork or icon set used; conventional list, columns and grid symbols. |
| `crates/midnight-ui/ui/icons/archive.svg` | SVG paths and shapes authored directly by the assistant for this project, 7 November 2026. A gold archive box with a zipper; viewBox 24×24. | None used. The box/zipper is a conventional archive symbol, not a reproduction of a particular product icon. |
| `crates/midnight-ui/ui/archive-badge.slint` | Small zipper paths authored directly for this project, 7 November 2026; conventional alternating teeth and pull, with a theme-aware contour. | User-supplied zipped-folder screenshot was a motif reference. No screenshot pixels, external vector paths or icon package were copied. |
| `crates/midnight-ui/ui/icons/attributes.svg` | SVG paths authored directly by the assistant for this project, 7 November 2026. A folded document with a clock, recolored by the active theme; viewBox 24×24. | No external artwork or icon set used. Used in the attributes/time/case toolbar, function action and dialog header. |
| Windows file, drive, folder and action icons | Loaded at runtime through Windows Shell APIs, including `SHGetFileInfoW` and `SHGetStockIconInfo`. | Operating-system assets; they are not original project artwork. |
| Linux file, drive, folder and action icons | Loaded at runtime from installed, trusted desktop icon themes. | Theme authors' assets, under their respective terms. |

The archive asset is embedded as vector artwork and used when an archive has
only a generic document icon. Associated application icons retain precedence.
Existing project assets not individually listed above have not been retroactively
certified as original by this record. Third-party library and bundled asset
notices remain in [third-party.md](third-party.md) and distribution packages.

## SVG worker font fallback, 12 November 2026

The independent shared image decoder embeds unmodified Noto Sans Regular, Arabic
and Hebrew font files from the pinned cosmic-text 0.19.0 source archive. They
remain under SIL Open Font License 1.1, with exact per-file SHA256 hashes and
upstream/source information in [the font provenance](../plugins/image-decoder/fonts/README.md).
The license and provenance accompany both image-worker distributions. These fonts
are loaded from embedded memory only; no document-selected or installed font path
is opened. Coverage and family fallback are explicitly limited as documented.
