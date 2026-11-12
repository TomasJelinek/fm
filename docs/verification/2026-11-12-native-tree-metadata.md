# Native inline-tree metadata — 12 November 2026

## Change and authority

Attributes, timestamps and filename case now capture the exact acknowledged tree
selection, or focused real occurrence when unmarked, before entering the existing
metadata preview and versioned confirmation. Provider/native identities are retained;
display paths, parent navigation, context rows, archives and mixed providers cannot
authorize native metadata mutation.

The capture shares the four-slot admission pool and runtime-independent retirement
domain with existing tree consumers. Native preparation reserves the complete batch
from the existing 256-target metadata budget and checks count/scope before and after
preparation. Cleanup remains on workers, including stale or refused UI receipts.

Selected ancestor/descendant roots remain independent for nonrecursive edits, in
deepest-first order. Recursive edits visit covered roots once through their selected
ancestor, using captured occurrence ancestry. Completed native root pins close before
the next root; retained provider capture ownership closes after all pins. This fixes
a reproduced Windows AccessDenied when a selected parent's case rename followed its
selected child while the completed child's handles remained open.

Outer command refusal and transient capture admission refusal retain the original
intent. One-pump deferral avoids a request/receipt loop. Provider/target resource
limits require explicit retry or a smaller selection. Navigation, reprojection,
replacement and shutdown retire pending captures. Terminal mutations request loaded
native-tree reconciliation through the existing maintenance path.

No dependencies, native platform implementations or plugin ABI changed.

## Verification evidence

Unless an explicit path is shown, logs are under
`_build/qa/tree-metadata-20261112/` in the authoritative checkout.

| Evidence | Result |
| --- | --- |
| `red.log` | Expected missing-feature failure: opening metadata on an expanded descendant sent zero backend commands. |
| `ui-retry-red-2.log` | Expected full-event-path failure: transient capture refusal retried in the same receipt pump. |
| `engine-metadata-final.log` | 47 passed; native selected child/parent case test reproduced Windows AccessDenied, then fixed by consuming completed root pins. |
| `engine-resource-red.log` | 48 passed, including corrected native overlap case/time behavior; only the expected provider-resource retry classification regression failed. |
| `windows-engine-final.log` | **49 passed, zero failed** in the focused engine metadata gate, including all 15 new tree metadata tests. |
| `_build/qa/find-actions-20261112/linux-final/engine.log` | **1329 passed, zero failed, 11 ignored** across 44 Linux engine suites. All 15 tree metadata tests and existing metadata operation tests passed; this is native Linux source execution, not Windows desktop evidence. |
| `_build/qa/find-actions-20261112/linux-final/ui-tree.log` | **6 passed, zero failed** in the focused Linux tree metadata presenter gate. |
| `_build/qa/find-metadata-scroll-20261112/windows-affected.log` | **1844 passed, zero failed, 44 ignored** across 30 Windows suites. Includes 934 UI library tests (all six new metadata presenter tests), four native Engine/Slint tree metadata integration tests and all five existing metadata editor tests. |
| `app-harness-2.log` | Earlier successful application harness compilation. Superseded by the rebuilt application harness in the final Windows affected gate above: 46 passed, 17 ignored. |
| `_build/qa/find-metadata-scroll-20261112/windows-clippy.log` | Strict Clippy completed for the root runner's affected app/UI/native/thumbnail/viewer packages and engine with `--all-targets --no-deps -- -D warnings`. The log records two `vendor/femtovg` dependency warnings; this is not a warning-free dependency or whole-workspace claim. |
| Owned-file `git diff --check` | Passed. New source files remain below the 100,000-byte handwritten limit and use UTF-8/LF with rustfmt width 100. |

The final engine gate exercises real Windows native case/time changes with both
recursive and nonrecursive overlapping roots; exact revision confirmation;
cancellation without changes; replacement rejection; selected descendant paths
independent of display/refresh context; hard-link names; context/archive/mixed
refusal; capture bounds; complete-batch pin admission; and native/provider destruction
off the caller thread. Existing metadata conflict, partial-cancellation, recursive
budget and shared-handle-budget tests also pass.

A new confirmation test initially sent stale/current Apply requests back-to-back
before the actor released its duplicate-confirmation reservation. It was corrected
to await a new explicit preview revision, matching the existing workflow. Temporary
Find/core/Slint compilation failures encountered during shared integration were
reported to their owner and resolved before the final engine gate.

## Integration scope and remaining qualification

The final Windows affected gate passed all six presenter tests in
`crates/midnight-ui/src/presenter/tests/tree_metadata.rs` and all four command-bridge
tests in `crates/midnight-ui/tests/tree_metadata.rs`. The bridge tests exercise real
native descendants in both left and right expanded panels for attributes and
filename case through Engine/Slint. The five `metadata_editor` tests passed,
including prepared confirmation, compact attributes and timestamp controls. These
are automated native-fixture and UI integration tests, not an installed desktop
interaction recording.

The Linux engine total and the focused six-test Linux presenter result above were
independently checked against their logs. The final Windows gate rebuilt and ran
the application harness; the earlier harness's viewer-close bridge limitation is
therefore no longer an outstanding compilation item.

Protected deployment and an installed native desktop metadata workflow remain
with the root task. No result for those steps is inferred from the completed test
suites or the strict affected-package Clippy gate.

The normal metadata limits remain: live, nontransactional subtree edits; cancellation
retains completed changes; no link/mount traversal; bounded pins, depth and name
snapshots; explicit unsupported-field errors; no archive/TEMP mutation fallback.
Capture pins native object identity during preparation, not at dialog invocation.