# UI review pass — 20 November 2026

Scope: the open findings 2–6 and 8 of the
[acceptance findings](2026-11-19-acceptance-findings.md), a visual review of every
`ui_review` screenshot in all themes, and the follow-ups requested during the pass
(one action-glyph style, tooltip sharpness, the comparator's compact Close, test
gaps). Finding 7 (`docs/developer/checksums.md`) moved to the checksum review, with
the Checksums window and its controls; this pass did not edit them.

Every result below names the command that produced it. Cargo ran in the project's
Docker build image (`midnight-explorer-build:9049146550…`) with the worktree mounted
read-only at `/workspace`, the shared `_build` and `.tools`, `CARGO_INCREMENTAL=0`,
`--locked` and a per-session `--config` that gives every workspace crate its own
artifact hash. Other sessions built into the same target directory meanwhile.

## Baseline

`cargo test --workspace --locked --features midnight-acceptance/acceptance --test
acceptance -- ui_review --all-themes` on the unchanged integration snapshot (the
test binary takes scenario filters as plain arguments; `--only` belongs to
`scripts/acceptance.sh`): WARN, 21 review flags in 208 screenshots, 344 s.

| Kind | Flags | Where |
| --- | --- | --- |
| overlapping controls | 10 | breadcrumbs at 960×600 (Blue/Desktop, Dark/Touch) and 1340×820 (HighContrastDark/Touch) |
| text does not fit its button | 9 | breadcrumbs (4), function keys F2, F7, F9 at 960×600 (4), Find's Copy details at 672×450 Touch (1) |
| label elided to nothing | 2 | the `workspace` breadcrumb under the leading overlay at 960×600 |

## Changes

| Finding | Change |
| --- | --- |
| 2, function keys | `function-button.slint`: a caption that does not fit whole, even at the readable minimum size, gives way to the key alone (`F7`); very narrow cells still show the key alone. The fit test uses Slint's own elision threshold (a line is elided only when it ends a whole pixel beyond its box). A caption that fits beside its icon keeps the full font size. |
| 3, breadcrumbs | `path-navigation.slint`, `path-token.slint`: no overlay and no scrolling strip. A path that does not fit keeps up to two leading components (while they take at most about a third of the bar), one `...` marker ("Edit full folder path", tooltip: the full path) and the longest suffix of whole components; hidden components are not drawn and have no area. Only the last component can be shortened at its end, when it alone is wider than the bar. 32 px after the path still open the editor. Hovering a later component also highlights the marker. |
| 4, Find | `find-window.slint` (footer only), `dialog-footer.slint`: the query footer shows Copy details only when all six query actions fit beside Close, measured from their captions at the current font (about 642 px wide at Desktop density, 748 px at Touch, with DejaVu). The details view keeps its Copy. |
| 5, function-key layer | `main.slint`: opening or closing an embedded dialog returns the footer to the base layer when no native modifier state is available; `shortcut_bridge::window_deactivated` (called on `WindowActiveChanged(false)`) does the same when the main window loses activation. Native (Winit) modifier state keeps owning the footer. |
| 6, right-to-left names | Names drawn in panel rows (Detailed, Brief, Thumbnails, tree, search highlight), breadcrumb components, tab captions and the details bar's Name field, and the single-name first line of the Copy/Move and Attributes dialogs, are wrapped in U+2066…U+2069 (`NameText.ltr` in `path-text.slint`, `isolated_name` in `lib.rs`). Only drawn text: models, accessible labels, paths and edit fields keep the raw text. The search highlight offsets move by the three bytes of the opening mark. |
| 8, docs | `dialog-ergonomics.md`: About and Compare directories are embedded popups; Help is the native modeless help window. The breadcrumb paragraph describes the new collapse. |

Follow-ups requested during the pass:

- One glyph style (`action-icons.slint`): labelled buttons, toolbar buttons, function
  keys, menu rows and tab close buttons draw the bundled glyph as a monochrome mask in
  `Theme.secondary-text` (`muted-text` while unavailable; destructive and removal
  actions in `error-text`). Host artwork no longer replaces a glyph, so the same
  button looks the same on every platform. Dialog header icons, the application
  logo, shell icons, custom-command icons and file icons keep their colors. Labels
  that had no glyph got one through `FallbackIcons.for-label` (Refresh, Clear,
  Assign, Restore…, Discard changes, Apply colors, Insert value/file, Change icon…,
  Details, Resolve, Jobs, Pause/Resume/Play, Decide later, Replace…, Merge…), the
  operation dialogs' confirm button uses the operation's glyph, and Jobs' Clear
  buttons use the close glyph. A new original `icons/pause.svg` is recorded in
  `asset-provenance.md`.
- Comparator: the compact image-mode Close is sized by its glyph and caption instead
  of a fixed 45 px. Comparator tooltips now take the window scale for device-pixel
  snapping (`compare-ui/src/tooltip.rs`).
- Tooltips (`controls.slint`, `tooltip-placement.slint`): the frame size rounds up to
  whole device pixels (`snap-up`), so a 100.9 px hint at 125 % gets 101.6 px instead
  of 100.8 px, and the border is one physical pixel (`1phx`).

Defects found in the screenshots and fixed:

- Make File List said "1 items" (`presenter/file_list.rs`: "1 item").
- Hot Paths spread its controls over the spare height of its 320 px dialog and kept
  an empty list row with no saved paths (`hot-paths-dialog.slint`: controls at the
  top, no empty row).
- Colors page at Touch density cut the hex value (`#3b82` for `#3b82f6`): the field
  now keeps room for a whole `#RRGGBB`, and Background moves below Foreground when the
  page is too narrow for both (`color-options.slint`).
- Keyboard page at Touch density elided the capture prompt ("Click here, then press
  a…"): the prompt is now "Click, then press keys" (`keyboard-editor.slint`).
- Attributes at Touch density scrolled its permissions page inside a 620 px dialog
  even in a 1600×1000 window: 760 px at Touch.
- The Move dialog's header showed the Copy glyph.

## Tests

New or changed tests, with what they prove:

| Test | Proves |
| --- | --- |
| `function_bar::narrow_function_keys_show_the_whole_caption_or_the_key_alone` | At 960×600 (Desktop and Touch), 820×600 Touch and 640×480 every key draws its whole caption or the key alone, never a caption wider than its box. |
| `path_elision::collapsed_paths_show_whole_components_that_never_overlap` | Deep, long-root and right-to-left paths at 200, 260 and 320 px: no overlapping buttons, no cut component except the last, order leading → marker → trailing, the marker exactly when components are hidden, the current folder always shown. |
| `path_elision::right_to_left_names_keep_their_extension_at_the_end` | Rendered pixels: `עברית.txt` starts where `עברית` does and `.txt` follows it; without the marks it does not; the marks draw nothing. |
| `find_control_geometry::query_footer_never_cuts_an_action_and_keeps_copy_details_where_it_fits` | 672×450 Touch hides Copy details and cuts no action; 672×450 Desktop and 938×615 Touch keep it. |
| `shortcut_bridge::…::function_modifiers_return_to_the_base_layer_around_dialogs_and_deactivation` | Ctrl layer reset when a dialog opens and closes and on deactivation; native modifier state untouched. |
| `tooltip::tests::tooltip_frames_follow_the_window_scale_and_round_up_to_device_pixels`, `wake_tests` (comparator, scale 2.0) | Tooltip scale follows the window (main and comparator); `snap-up` lands on the device grid and never shrinks a size. |
| `panel_toolbar::pointer_action_does_not_leave_a_checked_looking_outline_but_keyboard_focus_is_visible` | Restored: a pointer press leaves no outline; keyboard focus draws the active-frame border on every edge. |
| `tabs_bookmarks::tab_close_shows_on_hover_and_always_on_touch` | Tab close hidden at rest, shown while its tab or itself is hovered, always on Touch. |
| `action_icon_colors`, `bottom_bar`, `native_controls`, `native_tool_layout` | Rewritten for the one glyph rule: every palette tints the toolbar glyph (muted when unavailable), host artwork never replaces a glyph. |
| `file_list_tests::the_item_count_is_singular_for_one_item`, `ui_polish` (comparator) | "1 item"; every comparator image-mode caption, compact Close included, fits its button. |

Adjusted for the display-only isolation marks: `metadata_times`, `panel_row_layout`,
`filename_colors`, `detail_font` (they matched drawn name texts exactly).

Commands (each with `--locked --config <per-session config>`), run in the Docker image:

- `cargo test -p midnight-ui --lib -- file_list shortcut_bridge tooltip`: 34 passed.
- `cargo test -p midnight-ui --test integration`: 803 passed, 0 failed, 2 ignored
  (the whole `midnight-ui` integration binary, 11.8 s). The run before the last
  fixes had 7 failures, all from the changed rules: 5 tests asserting the old native
  icon colours or exact name texts (rewritten above), the compact Move button 3 px
  wider than its row with its new glyph (the confirm button now uses compact
  padding in compact forms), and Panel filter's OK pushed out of a 320 px footer by
  a glyph on Show all (Show all stays without one).

- After the last two Slint changes (the function-key fit threshold and the shorter
  capture prompt): `cargo test -p midnight-ui --test integration -- function_bar
  path_elision`: 16 passed.
- `cargo test -p midnight-compare-ui --lib -- wake`: 1 passed (the DPI step now
  also checks the tooltip scale).
- `cargo test -p midnight-compare-ui --test integration -- ui_polish`: 8 passed
  (image-mode buttons, compact Close included, fit their captions at 320×200 and
  1100×740 with 12 and 32 px fonts).
- `cargo clippy -p midnight-ui -p midnight-compare-ui --all-targets -- -D warnings`:
  clean (one `type_complexity` in a new test fixed first).
- `rustfmt --edition 2024 --check` on every changed Rust file: clean.

## Review after the changes

`cargo test --workspace --locked --features midnight-acceptance/acceptance --test
acceptance -- ui_review --all-themes`, all 208 screenshots of each run looked at
(per-dialog contact sheets across the six full combinations, every theme's main
window, and full-size crops where something looked off):

| Run | Result |
| --- | --- |
| `20261120-020506` (first fixes) | PASS, 0 flags. Seen: F10 Menu shown as the key alone at 960×600 Touch (fit test 0.7 px too strict), cut hex value and capture prompt at Touch, Attributes scrolling at Touch, "1 items". |
| `20261120-024150` | PASS, 0 flags. Seen: the capture prompt still elided (the stretch change had no effect; reverted, prompt shortened). |
| `20261120-025923` (final sources) | PASS, 0 flags, 208 screenshots, 341 s. F10 Menu whole again at 960×600 Touch; F2, F7, F9 show the key alone there and F9 at 960×600 Desktop; the capture prompt is whole at every density. |

Flags: 21 before, 0 after. All 21 belonged to findings 2–4; none remains and no new
flag appeared.

## Left as they are

Matters of taste or design, or areas owned by another session:

- At 960×600 Touch several keys show the key alone (F2, F7, F9): their captions need
  85–105 px in 73 px boxes at the 15 px minimum. A shorter label set would be a
  product decision.
- Panel filter's Show all has no glyph: with one, OK no longer fits the 320×200
  footer (`native_filter_hotpath`).
- Labelled buttons still without a glyph: symbol captions (+, −, ‹, ›, 1:1, Aa), the
  viewer's Paged/Continuous/Fit/Find/Previous/Next and the comparator's mode, Fit,
  Find… and difference buttons (no common glyph; fixed compact widths), the Times
  page's Now, and the Checksums window's buttons (checksum review).
- Notices are capped at 440 px and scroll: at Touch density Drive information's last
  line scrolls even in a 1600×1000 window.
- Rename/Create directory at Desktop density drop their "New name" label below 280 px
  of dialog height yet keep about 70 px of blank space above the footer; Trash shows
  its warning plain at Desktop and red at Touch (`operation-controls.slint`, shared
  with the operation review).
- Hot Paths keeps its 320 px minimum height; with no saved path the spare height sits
  above Close.
- High-contrast themes derive coloured file names to 4.5:1 (the AA minimum), well
  below their own 20:1 text.
- Find (owned by the Find review): at 672×450 Touch the results list shows only its
  header and one line. Help lists `Open other panel's folder — .` (the Period key
  reads as punctuation). About (owned by the About review): the version pill and the
  Version row are empty in the test harness.

## Options findings (second batch)

Findings from the Options acceptance scenarios, each checked in the sources, fixed
and covered by a test:

| Finding | Cause | Change |
| --- | --- | --- |
| OPT-ESC: Escape in the custom archive extension field closed Options and dropped every staged change, although its hint says "Escape to cancel" | The Options focus scope took Escape in its capture phase, before the field | While an inline editor is open, Options leaves Escape to it: the extension field cancels its edit and returns focus to its +, and the next Escape closes Options. An open extension editor without focus is cancelled first too. The same applies to a new argument row (Custom Commands, Editors and external comparators share the argument editor), whose hint now says Escape cancels. |
| OPT-HOTPATH: Hot Paths rows with a common parent looked identical | `PathText` always omitted the end | `PathText` takes an optional tail and then omits the middle, keeping the last folder whole (and cutting it at its own end only when even it does not fit, after a leading marker). Hot Paths rows and the Current folder line use it; the row keeps its full-path tooltip and accessible label. |
| OPT-CMDERR: a Custom Command whose program is missing reported "Operation: external editor" and "0 processes started · 0 not started" | The Linux and Windows launchers reused the editor's program check and its wording; the notice always printed the counts | The editor and Custom Commands share the program rule but word it themselves: "The program was not found." for a Custom Command, operation "Custom Command". A command that failed before planning any process is reported as "<name> could not start." without counts. |
| OPT-CMPERR: an invalid comparator regex ran into a sentence about matcher size limits | The size-limit sentence was appended to every compile error | Only a matcher that is too large explains the limits; a parse error is one sentence ("Invalid regular expression: unclosed character class.") and the page wraps it between words. |
| OPT-TIP: tooltips broke words | `TipBody` used `char-wrap` | `word-wrap`; in the pinned Slint a token wider than the line (a path) still breaks where it must. |
| OPT-F9: the F9 User menu did not list Custom Commands | The menu had fixed rows only | The configured Custom Commands follow the fixed rows (separators as rules, disabled ones listed but unavailable) and run on the active panel; without any, "No Custom Commands configured." and Set up, which opens Options → Custom Commands. The command hint no longer ends with "Right-click for Custom Commands settings": the toolbar, where that works, adds the line itself. |

Other Options inline editors were checked for the Escape problem: the colour picker
already closes first, keyboard capture records Escape by design, and the Plugins,
Colours, Editors and Comparators text fields are ordinary fields (Escape closes
Options as Cancel does). Hot Paths is its own dialog without inline editors; its
shortcut list takes Escape in its own popup.

| Test | Proves |
| --- | --- |
| `archive_options::escape_in_the_extension_field_cancels_only_that_edit_and_then_closes_options` | Escape in the field keeps Options open with its staged extension, closes the editor, empties the draft and focuses +; Enter reopens it; the next Escape closes Options. An open editor whose field lost focus (Shift+Tab) is cancelled before Options closes. |
| `argument_lists` (both lists) | Escape in a new argument row leaves the arguments unchanged, returns to the selected row and keeps Options open. |
| `path_elision::path_labels_with_a_tail_omit_the_middle_and_keep_the_last_component` | Two paths with a 12-level common parent show one marker between the start and their whole, distinct last component; a tail wider than the label gets a marker on each side; a path that fits shows none; a tail that is not the end is ignored. |
| `native_filter_hotpath::hotpaths_form::embedded_hotpaths_rows_with_a_common_parent_keep_their_last_folders_in_view` | In the shipped manager both rows and the Current folder line keep their last folder whole after the marker. |
| `hot_path_windows::tests::a_path_tail_is_the_last_component_with_the_separator_before_it`, `…installed_hot_paths_keep_opening_location…` | Tails for native, Windows, trailing-separator, root and network paths; the manager and the bookmark rows receive them. |
| `user_commands::a_missing_or_unusable_program_is_reported_as_the_custom_commands_own` (Linux) | A missing program: operation "Custom Command", "The program was not found.", the program path; a non-executable file: "The program is not an executable file." |
| `presenter::tests::user_commands::a_command_whose_program_is_missing_is_named_and_reported_plainly` | The notice starts "Test command could not start." and names the problem, without counts or "external editor". |
| `file_comparators::matching::disabled_invalid_regex_…`, `…oversized_regex_…` | An invalid regex gives exactly "Invalid regular expression: unclosed character class."; only an oversized matcher explains the limits. |
| `path_elision::tooltips_wrap_between_words_and_break_only_tokens_wider_than_a_line` | Three words on a line narrower than two take three lines (character wrapping would take two); a long path wraps instead of overflowing; at scale 1.25 the snapped frame is on the device grid and one-line hints stay on one line. |
| `command_menus::f9_lists_the_custom_commands_runs_the_chosen_one_and_points_to_options_without_any` | Without commands the menu shows the empty state and Set up opens Options on Custom Commands; with commands they come before Close, a disabled one does not run, the chosen one runs with its revision and the menu closes. |
| `fonts::user_menu_fits_its_rows_at_every_font_size` and the 32 px Close check (existing) | The empty-state row keeps Close visible without scrolling down to the smallest font and at 32 px. |

Commands (each with `--locked --config <per-session config>`), in the Docker image:

- `cargo test -p midnight-platform-common --test integration -- file_comparators
  panel_filter`: 41 passed.
- `cargo test -p midnight-platform-linux --test integration -- user_commands`: 2
  passed; `cargo test -p midnight-platform-linux --lib -- editor`: 6 passed.
- `cargo test -p midnight-ui --lib -- hot_path user_commands tooltip`: 45 passed.
- `cargo test -p midnight-ui --test integration -- archive_options argument_lists
  path_elision command_menus native_filter_hotpath tabs_bookmarks native_tool_layout
  fonts`: 58 passed. (The first build stopped on a Slint binding loop: the wrapped
  empty-state text's height followed the menu width, which follows the menu height.
  The text is one line now.)
- `cargo test -p midnight-ui --test integration`: 808 passed, 0 failed, 2 ignored.
- `cargo test -p midnight-ui --lib`: 1276 passed, 1 failed:
  `metadata_presentation::tests::header_names_a_single_item…` still expected the
  single name without the isolation marks of the first batch; the expectation was
  updated (rerun below).
- `cargo clippy -p midnight-ui -p midnight-platform-common -p midnight-platform-linux
  --all-targets -- -D warnings`: clean.
- `rustfmt --edition 2024 --check` on every changed Rust file: clean.
- Windows (`midnight-platform-windows`, not built): the changed program check and
  Custom Command wording, extracted with a stand-in `FsError`, compiled with `rustc
  --edition 2024` and returned "external editor" / OS message for the editor and
  "Custom Command" / "The program was not found." for a Custom Command.
- After the expectation update: `cargo test -p midnight-ui --lib --
  metadata_presentation`: 5 passed; `cargo clippy -p midnight-ui --all-targets --
  -D warnings`: clean.
- `ui_review --all-themes` (run `20261120-041928`, these sources before the hint and
  empty-state wording changes below): PASS, 0 flags, 208 screenshots, 351.6 s. The
  Hot Paths Current folder line now reads `…/20261120-04.../review` at 960×600
  Touch instead of losing its last folder; no saved Hot Path exists in the fixture.
- After the hint and wording changes: `cargo test -p midnight-ui --lib --
  user_command`: 69 passed; `cargo test -p midnight-ui --test integration --
  command_menus user_command_toolbar shared_toolbar tooltips fonts
  user_menu_forms_ui`: 43 passed; `cargo clippy -p midnight-ui --all-targets -- -D
  warnings`: clean.

## Not verified

- The Windows launcher change (OPT-CMDERR) was not built: its program check and the
  Custom Command wording were compiled and run only as a standard-library extract
  on Linux.
- The About keys in `keyboard-shortcuts.md` follow the About review's description;
  its sources are not part of this change.
- Native Winit, Windows and real fractional scales: the tooltip `snap-up` and `1phx`
  border were checked by unit tests and at scale 1 in the software renderer only.
- Right-to-left names were checked by a rendering test and the code paths above;
  no `ui_review` screenshot shows a right-to-left name.
- The other acceptance scenarios were not run here; a scenario that clicks a
  breadcrumb by label sees only the components a collapsed path still shows.
