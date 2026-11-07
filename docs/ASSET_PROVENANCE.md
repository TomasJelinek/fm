# Asset provenance

This record describes where assets came from; it is not a legal clearance opinion
or a change to the repository's license.

| Asset | Origin | External reference or copied artwork |
| --- | --- | --- |
| `crates/midnight-ui/ui/icons/archive.svg` | SVG paths and shapes authored directly by the assistant for this project, 7 November 2026. A gold archive box with a zipper; viewBox 24×24. | None used. The box/zipper is a conventional archive symbol, not a reproduction of a particular product icon. |
| `assets/branding/moon-master.png` and its PNG/ICO exports | Created with the built-in image generation tool on 6 November 2026, as recorded in the [branding README](../assets/branding/README.md) and [generation prompts](../assets/branding/PROMPT.md). | The recorded brief specifies a light/dark moon; it does not name another product's artwork as a reference. This records the creation process, not a guarantee of uniqueness. |
| Windows file, drive, folder and action icons | Loaded at runtime through Windows Shell APIs, including `SHGetFileInfoW` and `SHGetStockIconInfo`. | Operating-system assets; they are not original project artwork. |
| Linux file, drive, folder and action icons | Loaded at runtime from installed, trusted desktop icon themes. | Theme authors' assets, under their respective terms. |

The new archive asset is embedded as vector artwork and used when an archive has
only a generic document icon. Associated application icons retain precedence.
Existing project assets not individually listed above have not been retroactively
certified as original by this record. Third-party library and bundled asset
notices remain in [THIRD_PARTY.md](THIRD_PARTY.md) and distribution packages.
