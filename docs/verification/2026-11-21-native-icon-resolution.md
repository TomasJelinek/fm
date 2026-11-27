# Native icon resolution — 21 November 2026

## Scope

The user's report: in the Thumbnails view of `C:\Windows\System32` at a high
display scale, every icon was visibly pixelated. Tiles are about 216 device pixels
there (96 px at 225 %), and each icon was a small bitmap enlarged to that size.
This record covers the fix on Windows and Linux, its tests and a measurement. The
contract is in [native icons](../developer/native-icons.md).

Source: commit `2b901122` plus the icon change's files only, built in an isolated
export (`git archive` of `2b901122` with the changed files copied in; `protocol.rs`,
the engine's `lib.rs` and the UI tests' `support/owned_commands.rs`, which other roles
also change, written as `2b901122` plus only this change's hunks) with its own target
directory, because the shared worktree held other roles' unfinished work. The later
commits up to `06ff3e92` change none of these files.

## Root cause

- **Windows type icons were always 32 px or smaller.** `IconService::load_icon`
  refused sizes above 64 and drew the Shell's small or large icon
  (`SHGetFileInfoW` with `SHGFI_ICON`, 16 or 32 px at 96 DPI) into a bitmap of the
  requested size, and `IconService::icon_edge` kept its default answer of 32 for
  every edge. The UI then drew that 32 px bitmap into 45 px rows and 216 px tiles.
- **Action art was always 32 px.** `IconEdges::of` gave action icons
  `IconKey::DEFAULT_EDGE` whatever the scale, so dialog title icons were enlarged
  at every scale above 100 %.
- **Program and icon-file artwork** came from `SHDefExtractIconW` at the requested
  size, bounded to 64 px for command buttons and 128 px for the icon picker, so no
  program could be drawn as a tile. Its choice of frame was already the smallest
  frame at least as large (the reverted variant below chose the same frames as the
  new code at 16 to 256 px); its own scaling of the chosen frame made opaque
  pixels slightly translucent (alpha 254 where the frame has 255).
- **Linux** already chose theme directories by pixel size, but read PNG only; themes
  that ship scalable artwork (current Adwaita, Breeze) fell back to small PNGs or
  to nothing.

## What changed

See the contract for the full description. In short:

- Windows draws every type, stock, known-folder, drive and link icon from the
  smallest system image list at least as large as the edge (36, 72, 108 and 256 px
  on this 216 DPI machine), recognizes the jumbo list's corner placement of small
  artwork, and reduces once with a premultiplied Lanczos filter
  (`midnight_platform_common::icon_scale`, shared with Linux). `icon_edge` answers
  the drawn edge itself.
- Program, icon-file and command artwork is drawn from the icon frame that suits
  the edge (smallest at least as large, else largest, deepest color first), PNG
  frames included, read without running code.
- `IconEdges` gained `action`, the largest action box in device pixels (32 px
  logical), which the UI reports with the other edges.
- In the Thumbnails view, a program's tile shows the program's own icon:
  `ThumbnailContent::FileIcon`, `Command::SetFileIconView`,
  `IconService::load_file_icon`, `Event::FileIconLoaded`. Visible tiles only,
  within the preview request budget, cancelled by the next view, remembered when a
  program has no icon, retained in the preview image cache.
- Linux ranks scalable theme directories whose range holds the edge right after
  exact ones and renders their SVG artwork at the exact edge with the existing
  resvg dependency; SVG is also read from fixed-size directories and pixmaps.

## Dependencies

`midnight-platform-windows` enables the windows 0.62.2 feature `Win32_UI_Controls`
(`IImageList`). The pinned manifest defines it as `["Win32_UI"]`, the crate has no
optional dependency, and no other package requests it: a metadata-only Windows capture
(`cargo metadata --locked --offline --format-version 1 --filter-platform
x86_64-pc-windows-msvc`) of the export resolves 44 features of windows 0.62.2, 43
without this request. No package, version, licence or notice changes, `Cargo.lock` is
unchanged (every build used `--locked`), and the Linux SVG path uses the existing resvg
0.48.1 with its features unchanged. The capture and the package's
`resolve.nodes[].features` are kept in `_build/qa/native-icons-20261121/`; see
[third-party inventory](../third-party.md).

## Probe of the system image lists

Measured on this machine (Windows 11, system DPI 216) with a throwaway C# probe
before the change was written: list edges 36, 72, 108 and 256 px. Every associated
type sampled (exe, dll, txt, mp4, zip, pdf, bat, ini, url, png, msc, folder) has
real 256 px artwork in the jumbo list. An icon file with only a 32 px frame
appears in the 36, 72 and 108 px lists enlarged, and in the jumbo list as the
108 px bitmap in the top-left corner of an empty 256 px canvas. The stock
Settings icon is an opaque black square or transparent in every list. The Shell's
shortcut overlay (overlay index 2) is drawn by the 108 and 256 px lists. Of the
665 programs in System32, 175 carry icon groups in the file; the others have none
(services such as `svchost.exe`) and keep the generic program icon.

## Tests

Debug builds of `2b901122` plus the change, in the isolated export with its own target
directory (`CARGO_TARGET_DIR=_build\agent-icons`). The test executables were compiled
inside one build slot and run outside it, each from its package directory; every
build below followed the last edit of the sources it compiles.

| Executable and filter | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-platform-windows` library, `icons` | 31 | 0 | 1 (the measurement) |
| `midnight-platform-common` library, `icon_scale` | 5 | 0 | 0 |
| `midnight-core` library, `desktop::tests` | 6 | 0 | 0 |
| `midnight-engine` library, `desktop::` | 54 | 0 | 0 |
| `midnight-ui` library, `file_icons`, `icon_view`, `presenter::desktop::tests`, `thumbnail`, `user_command_icons` | 51 | 0 | 0 |
| `midnight-core`, `midnight-engine` and `midnight-ui` integration, `icon` | 4, 3 and 14 | 0 | 0 |
| `midnight-explorer` (`me`), `thumbnails::` (the pulse that sends the views), one thread | 26 | 0 | 0 |

Commands, from the export's root:

```powershell
cargo test --no-run -p midnight-platform-windows -p midnight-platform-common -p midnight-core -p midnight-engine -p midnight-ui --lib --locked --offline -j 4
cargo test --no-run -p midnight-core -p midnight-engine -p midnight-ui --test integration --locked --offline -j 4
cargo test --no-run -p midnight-explorer --bin me --locked --offline -j 4
# then each executable with the filter in the table, for example:
midnight_ui-<hash>.exe file_icons icon_view presenter::desktop::tests thumbnail user_command_icons
me-<hash>.exe thumbnails:: --test-threads=1
cargo clippy -p midnight-core -p midnight-platform-common -p midnight-platform-windows -p midnight-engine -p midnight-ui -p midnight-explorer --all-targets --keep-going --locked --offline -j 4 -- -D warnings
cargo clippy -p midnight-core -p midnight-engine --target x86_64-unknown-linux-gnu --all-targets --keep-going --locked --offline -j 4 -- -D warnings
cargo clippy -p midnight-platform-linux -p midnight-platform-common --target x86_64-unknown-linux-gnu --all-targets --locked --offline -- -D warnings
cargo check --workspace --all-targets --locked --offline
rustfmt --edition 2024 --check <the changed .rs files>
```

The Windows-target clippy run and both Linux-target runs pass without a warning after
the last edit of the code they lint. The workspace check passed without warnings after
the last change to a type or command other crates use; the later edits are internal to
the engine and to tests and are covered by the clippy runs. `midnight-ui` was not
type-checked for Linux here (fontique needs fontconfig headers this machine lacks, as
the [test speed record](2026-11-21-test-speed-pass.md#linux) found), so its Linux-only
`file_icons_unix.rs` is first compiled by the Linux run. The Linux tests were not run
here: the Orchestrator runs them under WSL. Logs: `_build/qa/native-icons-20261121/`.

### With the fix reverted

Each variant was compiled from the export with the named mutation and run with the
same filter; the listed tests fail there and pass with the fix.

| Mutation | Failing tests |
| --- | --- |
| Windows provider bounded to 64 px again, and jumbo corner placement not recognized | `icons::sized_tests`: all four (`shell_artwork_is_rendered_at_every_edge_it_is_drawn_at`, `large_icons_carry_detail_of_their_own_not_an_enlarged_small_bitmap`, `unassociated_types_are_generic_at_every_edge_and_associated_ones_are_not` by the bound; `artwork_without_a_jumbo_image_is_drawn_from_the_largest_true_bitmap` by the corner) |
| Every type icon drawn from the list of about 32 px and enlarged; program frames chosen and scaled by the Shell (`SHDefExtractIconW` at the edge) | `large_icons_carry_detail_of_their_own_not_an_enlarged_small_bitmap` (0 sharp steps in either bitmap); `a_program_without_large_frames_is_enlarged_from_its_largest` (alpha 254 after the Shell's scaling). The two frame-choice tests pass: the Shell picks the same frames at 16 to 256 px |
| Action art at the default edge in core and at scale 1 in the UI; tile edges without the scale; a file icon view resent on every pass; straight-alpha resampling | `desktop::tests::icon_edges_list_each_place_a_kind_is_drawn_in_once`; `icon_view::tests::edges_follow_density_view_modes_window_size_and_scale`; `file_icons::tests::tile_edges_follow_the_display_scale`, `visible_program_tiles_without_artwork_are_asked_for_once_at_their_device_edge`, `program_artwork_is_retained_within_the_preview_byte_budget_least_recent_first`; `icon_scale::tests::transparent_pixels_carry_no_color_and_do_not_tint_the_seam` |
| The former executable path: the Shell's extraction at the edge, at most 128 px | `icons::executable::tests::icon_file_frames_are_chosen_by_the_requested_edge`, `program_frames_are_chosen_by_the_requested_edge_and_groups_by_index` and `a_program_without_large_frames_is_enlarged_from_its_largest` (no artwork above 128 px) |
| The preview image cache without its byte budget | `file_icons::tests::program_artwork_is_retained_within_the_preview_byte_budget_least_recent_first` |
| A new file artwork view appends its tiles instead of replacing and cancelling the old ones; programs without artwork read again | `desktop::file_icons::tests::a_scrolled_view_cancels_the_load_in_progress_and_drops_the_queued_tiles` (a tile of the scrolled-away view was loaded) and `a_program_without_artwork_is_read_once_per_listed_revision` |

The cancellation test first passed with its fix reverted: the old view's queued tiles
were loaded only after its assertion. It now waits for two type icons, which take
turns with file artwork, so a queued old tile is loaded before the second of them;
the reverted variant then fails, and the fixed code passed 30 repeated runs.

## Measurement

`icons::icons_measure` (ignored by default), release build, on this machine: a cold
and a warm pass of the provider over all 4,902 files of `C:\Windows\System32` in
name order (665 programs, 61 types), at the edges of a 225 % display: rows 45 px,
tiles 216 px. A details pass loads each type once, as the engine does; a thumbnail
pass reads each program's own icon and each other type once. "First screen" is the
first 40 rows or 15 tiles. The figures cover the provider calls the engine's icon
actor makes, one at a time; not the capture and revalidation around program
artwork, not the upload to the renderer, and not the engine's and UI's caches, which
serve a warm pass in the application without any provider call.

| Pass | Loads | Time to all sharp icons | First screen | CPU | Program median / p95 | Peak working set |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Details, cold | 61 | 1,391 ms | 643 ms | 688 ms | — | 40.2 MiB |
| Details, warm | 61 | 45 ms | 2 ms | 47 ms | — | 40.2 MiB |
| Thumbnails, cold | 723 (233 with artwork) | 3,916 ms | 119 ms | 2,703 ms | 3.8 / 10.4 ms | 47.4 MiB |
| Thumbnails, warm | 723 | 1,503 ms | 18 ms | 1,469 ms | 0.9 / 5.7 ms | 47.7 MiB |

The test process started at 9.8 MiB; private bytes ended at 12.8 MiB. The cold
details pass includes the process's first use of the Shell: in fresh processes, a C#
probe of the four types of the first 40 rows took 86 to 184 ms with the former
lookup (`SHGFI_ICON | SHGFI_SMALLICON`) and 89 to 292 ms through the large list.

Memory in the application: a type drawn as a 45 px row and a 216 px tile costs
190 KiB (61 types: 11.3 MiB); every program tile of System32 at 216 px would be
118.4 MiB, of which the preview cache keeps at most 54 MiB, least recently used
first. A full request budget of 256 tiles is 45.6 MiB, so the visible tiles always
fit.

## Not verified

- The running application: no tile, row, tab or menu was looked at, and no
  acceptance scenario covers icon sharpness yet.
- The Linux tests, which run under WSL: `icons::theme::tests` and `icons::tests` of
  `midnight-platform-linux` (including the new
  `directories_rank_exact_pixels_then_scalable_ranges_then_nearest_larger_then_smaller`,
  `scalable_ranges_follow_their_scale_and_default_to_their_size`,
  `scalable_artwork_is_rendered_at_the_exact_size_before_a_larger_bitmap_is_reduced` and
  `svg_artwork_in_fixed_directories_and_pixmaps_is_rendered_at_the_requested_size`),
  `icons::svg::tests`, `icons::executable::tests` and the platform-neutral groups above
  on Linux. The Linux code was linted for the Linux target only, `midnight-ui` not even
  that, and no Linux test was run with its fix reverted.
- A per-monitor DPI change while running: the Shell's list edges follow the system
  DPI at process start; artwork for a monitor of another scale is reduced or, above
  the largest list, enlarged from it.
- `.lnk` shortcuts keep the generic document icon; their target's artwork is not
  read. macOS has no platform adapter.
