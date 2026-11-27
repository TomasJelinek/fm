# Slint UI compile time — 18 November 2026

## Scope

Why `midnight-ui-bindings` (the Rust generated from `crates/midnight-ui/ui/main.slint`)
took about seven minutes to rebuild after any `.slint` change, and what changed.
Baseline is main `544b48db`. Linux only; nothing here was built or measured on
Windows.

## Result

Debug profile, Docker build image (Ubuntu 24.04, Rust 1.99.1), `jobs = 4` from
`.cargo/config.toml`, 16-core/32-thread host. Unit times of
`cargo build -p midnight-ui-bindings --locked --timings`; in the baseline run the
last dependencies were still compiling beside the bindings, in the final run only
the bindings were rebuilt.

| | main `544b48db` | this change |
| --- | ---: | ---: |
| Build script (Slint to Rust) | 41.4 s | 10.8 s |
| `rustc` on the bindings | 370.1 s | 172.7 s |
| of that, until code generation starts | 229.8 s | 56.5 s |
| Generated source | 62.9 MB | 30.6 MB |
| Generated elements / bindings | 4,821 / 35,872 | 2,652 / 20,894 |
| Generated components without globals (conditional or repeated) | 677 (461) | 554 (364) |
| Tooltip popup components | 57 | 27 |

A build of the whole application with only causes 1 and 2 and the profile change
fixed took 12.9 s and 226.8 s (81.2 s until code generation). With everything below,
three builds that compiled other crates and test targets beside the bindings took
176.6 s, 177.1 s and 192.3 s.

## Causes

### 1. A silenced lint copied the source for every short block

`rustc`'s `unused_braces` lint builds its suggestion before it looks at the lint
level. For a `{ expression }` block that starts and ends on one line it copies the
source text before and after the block (`span_to_prev_source` /
`span_to_next_source`), even where the lint is allowed. The adapter formatted the
generated module with prettyplease, which keeps short blocks such as
`.set({ (true) as bool })` on one line: about 9,300 of them in a 63 MB file. Each
copy is a 20 to 64 MB allocation that jemalloc returns to the system at once, so the
compiler spent most of its time in page faults.

Evidence, on the baseline output compiled alone with `--emit=metadata` (no code
generation), pinned to four cores:

| Run | Wall | User | System |
| --- | ---: | ---: | ---: |
| prettyplease output | 201.5 s | 73.3 s | 128.2 s |
| same, `MALLOC_CONF=dirty_decay_ms:-1,muzzy_decay_ms:-1` (diagnostic only) | 77.2 s | 73.4 s | 3.8 s |
| line-structured output | 65.3 s | 53.9 s | 11.4 s |

`strace -c` counted 36,184 `madvise` calls in the first run, each `rustc` process had
about 50 million minor page faults, and of 4,861 `madvise(MADV_DONTNEED)` calls of
16 MB or more caught in `gdb`, 4,849 came from
`rustc_lint::unused::UnusedBraces::emit_unused_delims_expr`.

Fix: `crates/midnight-ui-bindings/build/source.rs` writes the token stream itself and
ends the line at every brace, as slint-build's own writer does. A block that spans
lines is not examined by the lint. The output is no longer pretty-printed; run
rustfmt on a copy to read it. `syn` and `prettyplease` are no longer build
dependencies of the crate ([dependency note](../third-party.md)).

The allocator setting is not adopted: it only hides the copying, costs memory, and
applies only where `rustc` uses jemalloc.

### 2. The SDK ScrollView was inlined at every use

The Fluent `ScrollView` takes children, so it is inlined at each use, and its scroll
bar sets `visible` and its arrow buttons `opacity` on their own roots, which the
pinned compiler can only lower by inlining them too. With 71 scroll views that was
about 1,900 of 4,821 generated elements and 1,200 of 1,298 animated bindings.
`crates/midnight-ui/ui/scroll-view.slint` provides `ScrollView` and `ListView` with
the same interface, geometry, colors and input behavior, with one shared scroll bar
component. List virtualization is unchanged (15 `ListView` repeaters before and
after). The arrow glyphs are drawn as paths instead of the SDK's embedded SVGs.

Standalone compile of the generated module, four cores, incremental as Cargo's dev
profile: 381.9 s before, 309.9 s with the shared scroll view alone, 257.6 s with the
line-structured output as well.

### 3. Other components copied by the compiler's inlining rules

A component is generated once only where it is not inlined. The pinned compiler
inlines a component at a use that gives it children or sets `clip`, everywhere if
its root sets `visible`, `opacity` or a shadow, and into every component that
inherits it; each `if` or `for` element then becomes a generated component per copy.

- `FilePane` was clipped at both uses in `main.slint`, so the whole pane with every
  row and conditional below it was generated twice. It now clips itself.
- `ErrorNotice` set `visible` and `clip` on its root and was copied into 21 hosts.
- `DialogFrame` takes the dialog as children, so its geometry state, resize logic and
  eight resize edges were copied into every dialog. They are now one child-free
  `DialogFrameChrome`.
- A `Tooltip` written at a use site is its own popup component. Controls that can
  host a child use the shared `Tip` in `controls.slint`: 57 tooltip components became
  27.
- Smaller cases: the user command toolbar, action button content, file artwork, the
  stock `LineEdit` and `CheckBox` used both directly and through their hinted
  wrappers, and bar-less scroll strips.

### 4. Incremental compilation did not pay for this crate

Adding or removing one element renumbers the generated identifiers after it. A
one-element change altered 117,440 of 348,446 generated lines.

Standalone compile of the same module, four cores, opt-level 1:

| | Wall | Peak memory |
| --- | ---: | ---: |
| Incremental, full build | 257.6 s | 7.0 GB |
| Incremental, after changing one value | 207.1 s | 9.4 GB |
| Incremental, after adding one element | 233.1 s | 9.1 GB |
| Not incremental (16 codegen units) | 225.7 s | 7.1 GB |

Each incremental session also wrote about 1.8 GB. The dev profile now sets
`incremental = false` for this package only.

## Measured and not changed

- **opt-level 0 for the bindings**: 113.1 s instead of 257.6 s in the standalone
  compile. Not adopted: the generic property and item-tree code of Slint is
  instantiated in this crate, and `Cargo.toml` keeps opt-level 1 for interactive
  performance of debug builds. Its effect on UI test and resize time was not measured.
- **Thin local LTO off** (`-C lto=off`): 191.4 s instead of 257.6 s. Cargo cannot set
  `lto` for one package, and the dev-wide setting would also apply to the optimized
  renderer and text dependencies.
- **Codegen units** without incremental: 1 unit 283.8 s, 4 units 235.5 s, 8 units
  238.3 s, 16 units (default) 225.7 s, 32 units 247.2 s. Left at the default.
- **Optimizing `i-slint-compiler` for the build script**: the script would run in
  about 4 s instead of 11 s, but the compiler crate takes 28 s instead of 10 s to
  build, on the critical path of every clean build.

## Remaining cost and opportunities

Most of the remaining time is LLVM at opt-level 1, including thin local LTO. The
generated module still has about 21,000 bindings.

Measured but not applied, because behavior would change or tests are needed first
(on 19 November the tooltip sites except two, `ChecksumOutputPicker` and
`ArchiveExtensionInput` were done:
[record](2026-11-19-slint-build-speed.md)):

- About 30 remaining single-line `Tooltip { TipBody { … } }` sites can use `Tip`
  (measured about 1.4 MB of token text); each needs a check that its parent is not a
  layout.
- `ChecksumOutputPicker`, `ArchiveExtensionInput`, `FindActionButton`,
  `FinishedJobButton` inherit hinted controls and therefore copy them (about 0.1 to
  0.3 MB each).
- In-layout `if … : Text` and `if … : HintedActionButton` conditionals cost about
  10 KB each beyond their content, but a hidden element keeps its layout cell.

## Tests

Docker build image, Linux, debug profile, headless software renderer.

`cargo test --locked -p midnight-ui -p midnight-ui-input -p midnight-viewer-ui
--no-fail-fast` and `cargo test --locked -p midnight-compare-ui --test integration`:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-ui` library | 1,210 | 10 | 2 |
| `midnight-ui` integration | 760 | 4 | 1 |
| `midnight-ui-input` library and `placement` | 32 | 0 | 0 |
| `midnight-viewer-ui` library | 167 | 3 | 0 |
| `midnight-viewer-ui` integration | 25 | 2 | 0 |
| `midnight-compare-ui` integration | 15 | 0 | 0 |

The same 19 tests fail on main `544b48db` built and run the same way in a separate
worktree, so no failure is new. On main they are, in `midnight-ui`: the library tests
`presenter::preferences::tests::local_preview_wins_over_late_load_and_stale_save_results`,
`presenter::settings_picker::tests::picker_pages_are_the_editors_and_custom_commands_options_topics`,
`presenter::terminal::tests::windows_choice_persists_and_wins_over_late_preferences_loading`,
`presenter::tests::bookmarks::startup_duplicate_merge_preserves_local_callback_and_other_preferences`
and six `presenter::tests::preferences_load` tests; the integration tests
`comparator_options::long_values_wrap_and_error_fields_remain_reachable_across_sizes_fonts_and_themes`,
`comparator_transaction::leaving_picker_page_releases_busy_controls_even_before_a_picker_reply`,
`options_footer::footer_keeps_actual_save_failure_visible_above_its_actions` and
`panel_selection::selected_rows_keep_their_fill_when_focused_in_every_theme_and_view`.
In `midnight-viewer-ui`: two `image_presentation_tests::fixture_runtime_closes_a_retained_session_*`
tests, `web_popup_tests::web_consumed_release_does_not_leave_auxiliary_scrollbars_pressed`,
`ui_polish::web_policy_widgets_require_backend_permission_for_pointer_actions` and
`wrap_control::source_and_markdown_widgets_fit_narrow_window_and_emit_real_input`.
The `midnight-compare-ui` library test target does not compile on main
(`tests/support/mod.rs` names its own crate), so only its integration target ran.
Those failures were not investigated here.

An earlier state of this change failed two more tests,
`panel_scroll::pixel_scroll_settles_on_rows_after_native_motion_stops` and
`archive_options::archives_topic_has_individual_archive_and_document_checkboxes`. The
shared scroll bar was two-way bound to the Flickable's `content-y` at run time; the
SDK's list layout treats that as a binding and stops correcting the position. The bar
now receives the position one way and reports moves through a callback, and both pass.

`cargo build --locked -p midnight-explorer -p midnight-elevation` succeeds.
`cargo clippy --locked -p midnight-ui-bindings -- -D clippy::all`, `rustfmt --check`
on the changed Rust files and `scripts/check-source-size.sh` pass. The full workspace
test, Clippy and format gates were not run.

## Not verified

- Windows builds and timings. The lint cost is in `rustc` itself, so a similar gain is
  expected there, but it was not measured.
- Release profile timing. The release build compiles the same generated module.
- Visual comparison of the replaced scroll bars against the SDK ones beyond the
  existing geometry and input tests.
