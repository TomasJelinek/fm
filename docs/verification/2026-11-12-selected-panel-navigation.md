# Selected-item panel navigation verification

Date: 2026-11-12. Implementation was verified in coordinated Windows/Linux gates; results are recorded below.

Changes implement Salamander's previous/next selected-item behavior in displayed order, including folders and expanded tree occurrences, preserving selection and stopping at panel boundaries. Defaults are configurable and excluded from filename search and other editors. Comparator defaults remain unchanged; existing actual-window `difference_navigation` integration already covers Alt+Up/Down, both sides, boundaries, repeats and remapping.

Targeted checks:
- `midnight-core --test panel_selected_navigation`: default context separation, remap/disable and prior explicit assignments.
- `midnight-ui --lib selected_navigation`: native marks/parent/boundaries, sparse 100003-row selection, pending marks/loading/search, reversed filtered basis, tree occurrences, archive duplicate names, bitset masks and key dispatch.
- `midnight-ui --test selected_navigation`: installed presenter plus actual key events across both panes and all three layouts; marks preserved; command/search focus; live remapping.
- Existing comparator `--test difference_navigation` remains the actual-window contract for its Alt+Up/Down behavior.

Native desktop foreground qualification is not claimed by these headless integration checks.

## Coordinated Windows results

The final targeted Windows run passed the core integration (2), presenter/key-routing
filter (8), installed-presenter UI integration (1), and unchanged comparison difference
navigation integration (1). The existing 24-test shortcut target also passes after its
old assumption that Alt+Up/Down were unbound in Files was updated to the new actions;
Command history and Search/Other exclusions remain asserted.

These ran in a bounded four-process/five-thread headless batch. Raw per-target results
and timings are in `_build/qa/viewer-physical-keys-20261112/parallel-test-results.json`.
This is deterministic headless UI/key-event evidence, not foreground desktop qualification.

The final Linux gate also passed core 2, presenter/key-routing 8 and installed-presenter
UI integration 1, plus corrected shortcut integration 24. Logs are
`delta-panel-{keys,presenter,window}-linux.log` and `delta-core-shortcuts-linux.log`
in the same QA directory.
