# Viewer previous-file navigation — 12 November 2026

## Implemented behavior

`viewer.previous-file` defaults to Backspace and Alt+Left;
`viewer.next-file` defaults to Space and Alt+Right. Both use the viewer-only content
shortcut scope and may be remapped or disabled. Search fields retain ordinary
Backspace editing. Comparison navigation keeps its separate context and aliases.

`ViewerInput::PreviousFile` uses the same captured order, pending capture/close
handling and automatic per-file mode routing as NextFile. Both directions wrap.
Loading/ready images and PDFs pass the action to this common lifecycle. UI admission
uses the existing queue and only cancels stale raster/rebase/image camera work after
the command is accepted. Display paths do not gain filesystem authority.

The native viewer and performance probe examples now resolve the default catalogue
through the same Viewer/ViewerSearch contexts instead of a Space-only lambda.
HTML edit-focus arbitration and protocol compatibility are owned by the coordinated
HTML task; their evidence must be recorded separately from these engine/UI checks.

## Prepared regression checks

| Target | Coverage |
| --- | --- |
| `midnight-core --test viewer_shortcuts` | Default Backspace/Alt-arrow aliases, remapping/disabling, native panel Backspace preserved, all file-cycle aliases excluded from viewer search. |
| `midnight-viewer-engine --lib worker::tests::file_navigation` | Three tests: both wrap directions preserve captured order and reset mode; pending previous identity removes old content; loading/ready image and PDF inputs reach file navigation. |
| `midnight-viewer-ui --lib previous_file` | Two tests: previous-file commands clear retained raster motion and reject stale image zoom/frame state. Existing forward cases reuse the same helpers. |
| `midnight-viewer-ui --test keyboard` | Actual search widget deletes text with Backspace instead of cycling a file. |
| Ignored engine `real_plugins_render_search_copy_cycle_modes_and_reject_changed_files` | Existing real-plugin fixture extended with reverse wrap and forward return through Text/Source/Markdown; requires the protected three-plugin installation. |

Tests were added before production changes, but no Cargo run or RED/GREEN result
was claimed during this source-only implementation: root held the shared build
lease. Owned Rust files were formatted with width 100 and child traversal disabled;
UTF-8/LF, 100,000-byte file limits and whitespace were checked. Root's coordinated
integration gate must supply execution results. No new installed native workflow
or HTML runtime result is claimed by this note yet.

## Coordinated Windows results

Cargo-built targeted tests passed: viewer shortcut integration 6, captured file-order
engine tests 3, viewer UI library 121, and actual viewer search/keyboard integration 2.
Comparison keyboard and difference-navigation integrations also pass with the separate
Comparison context, preserving their existing Alt-arrow behavior.

These form part of the 19-suite parallel headless gate (229 test invocations; one routing
test intentionally appears in two focused filters), recorded under
`_build/qa/viewer-physical-keys-20261112/parallel-test-results.json`. Native installed
Chromium and foreground cycling qualification remain separately reported; these counts
do not represent execution of the ignored real-plugin engine fixture.
