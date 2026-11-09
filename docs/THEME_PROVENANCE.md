# Theme palette provenance

The themes added on 8–9 November 2026 extend Midnight Explorer's semantic
palette catalog without adding a library, editor plugin, font or icon package.
The modern choices are established community editor palettes selected for variety;
this is not a claim about a global popularity ranking.

| Theme | Stable ID / saved name | Palette reference and license |
| --- | --- | --- |
| Tokyo Night | `9` / `tokyo-night` | [folke/tokyonight.nvim, commit cdc07ac](https://github.com/folke/tokyonight.nvim/tree/cdc07ac78467a233fd62c493de29a17e0cf2b2b6), Night and inherited Storm colors; Apache-2.0. [Retained license](licenses/themes/folke/LICENSE). |
| One Dark | `10` / `one-dark` | [joshdick/onedark.vim, commit 47bec7a](https://github.com/joshdick/onedark.vim/tree/47bec7a6196a843dad195d2666c3ac84c6e80c78), `autoload/onedark.vim`; MIT, Copyright (c) 2015 Joshua Dick. [Retained license](licenses/themes/joshdick/LICENSE). |
| Rosé Pine | `11` / `rose-pine` | [rose-pine/neovim, commit ff48305](https://github.com/rose-pine/neovim/tree/ff483051a47e27d84bdef47703538df1ed9f4a47), main variant in `lua/rose-pine/palette.lua`; MIT, Copyright (c) 2023 Rosé Pine. [Retained license](licenses/themes/rose-pine/LICENSE). |
| Norton Commander | `12` / `norton-commander` | Original semantic palette inspired by classic blue, cyan and yellow DOS file-manager screens. No upstream code, screenshot pixels, logo, font or icon is included. |
| Amber Terminal | `13` / `amber-terminal` | Original charcoal/teal terminal interpretation, warm amber focus and selection, soft cyan folders and warm white text. Replaces DOS Commander; no external code or artwork is included. |
| Verdant Light | `14` / `verdant-light` | Original pale green/white surfaces, graphite text, forest-green controls and restrained lime accents. No brand name, logo, code or artwork is included. |
| Catppuccin Latte | `15` / `catppuccin-latte` | [catppuccin/catppuccin, commit d09787d](https://github.com/catppuccin/catppuccin/tree/d09787dd98ca6fba08af5ef2ae94a7e09f17daca), Latte reference palette; MIT, Copyright (c) 2021 Catppuccin. [Retained license](licenses/themes/catppuccin/LICENSE). |
| Rosé Pine Dawn | `16` / `rose-pine-dawn` | Dawn variant from the same pinned Rosé Pine source and MIT license above. Warm paper surfaces, rose focus, pine folders and violet secondary accents. |

The removed `dos-commander` saved name is accepted only as a migration alias for
Amber Terminal. Saving writes `amber-terminal`; DOS Commander is no longer a
picker entry. IDs 0–12 and their persisted names remain unchanged.

The upstream license files and palette references were inspected at the exact
commits above. All four retained licenses permit use and distribution without mandatory fees,
subscriptions or royalties. Their license terms and attribution still apply.
These choices do not change Midnight Explorer's own license.

The application maps selected reference colors to its own UI roles and adds or
adjusts surface, selection, border, error and foreground shades for file-manager
contrast. In particular One Dark body text is brighter than the reference editor
foreground, and Tokyo Night and Rosé Pine use stronger frames. Latte and Dawn use
darker semantic text and borders than some upstream accent colors so file names,
folders, pending sizes and controls remain readable on selected and hovered rows.
The adaptation is
in `crates/midnight-ui/ui/theme-palettes.slint`; no upstream editor implementation
is embedded. Existing native file icons keep their operating-system colors.

Release packaging includes this record and all four retained licenses under
`licenses/themes/`. The Cargo dependency inventory is unchanged by these palette
additions because no dependency or feature selection changed.
