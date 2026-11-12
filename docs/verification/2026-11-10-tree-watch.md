# Native inline-tree refresh and watches

Native manual refresh, legacy root notifications and the new batched branch hints
now reconcile the adopted tree instead of navigating back to a flat listing.
The [contract](../developer/TREE_PANEL.md#native-watches-and-reconciliation)
describes exact Commit admission, provider ownership, native slot bounds, fair
polling, debounce and dirty-epoch acknowledgment.

## Reproduction and review

The initial Windows presenter run failed the new root-hint/session-retention and
full-queue retry regressions; 11 other refresh tests passed. Architecture review
also required runnable explicit expansion when maintenance cannot acquire the
mutation gate, plus a nonretrying admission diagnostic.

Implementation review found repeated failed-scope reconstruction on unchanged
selection commits and autonomous watches surviving terminal session failure.
Both were corrected with regression coverage. A subsequent Windows test reproduced
an old-session failure canceling a newer flat navigation. Navigation and captured
cancellation fences now cover admission, callbacks, refresh receipts, retirement
and probe return. The final source review has no unresolved findings.

## Current verification

- Windows engine library: 642 passed; one intentionally ignored CPU benchmark.
- Linux engine library: 633 passed; one intentionally ignored CPU benchmark.
- Linux controller/filter integrations: 17 and 11 passed before the final narrow
  stale-navigation fence; the final full library suite covers that fence.
- Windows UI library: 735 passed, including retained marks/caret, manual/root hints,
  outer queue refusal, stale navigation/session and watch-diagnostic handling.
- Windows native watcher suite: eight passed. Linux native watcher suite: eight
  passed. They exercise immediate-child notifications, Unicode, batch validation,
  cancellation/teardown and the 32-target boundary.
- Windows UI bridges: 35 passed across row alignment, size errors, disclosure,
  tree Copy/Move, folder sizes, filters, prefix search, viewer authority, panel
  admission and actual native watches.

The native UI fixtures use real private files and native subscriptions with the
production Engine and installed Slint callbacks, rendered in software. Both panes
preserve their session, marks and caret across external root/expanded-branch
create/remove events with Unicode names. F5/F6 verify destination bytes, keep the
other pane's nested mark, update current root-file F3 authority and continue to
observe later external changes.

An initial transfer-test assertion incorrectly required a redundant debounced
hint after terminal reconciliation had already acknowledged the own-write event.
The corrected test observes actual native callbacks, waits for registration, and
then requires a later external change to appear. Both original failures were test
assumptions; no production suppression policy was weakened to satisfy them.

Logs, scoped before-images and reviews are under `_build/qa/tree-watch/`.
After the final mechanical constructor/initializer cleanup, Linux's four worker
and 38 watch tests passed. Windows core/UI all-target Clippy passed with no-deps
and warnings denied; this does not assert that dependency warnings are absent.
Strict engine all-target Clippy has no remaining new finding but still fails on
the two pre-existing unused-field groups noted below. Formatting and source-size
checks passed. The subsequent expanded-size qualification passed Linux's 730 UI
tests, six native tree Copy/Move bridges and both native watch bridges, as well as
the corresponding Windows gates. An initial Linux watcher fixture failed before
product execution because two concurrent temporary-directory names collided;
an atomic sequence now makes those test-owned names unique. The rerun passed
without changing production watch behavior. Logs are under
`_build/qa/expanded-sizes/`; the rebuilt Windows artifact, exact runtime
registration and successful native startup are recorded in the
[expanded-folder-size verification](2026-11-10-expanded-folder-sizes.md).
These checks do not qualify archive watches, generic provider migration, all tree
operation consumers or installed GPU/gesture behavior. The existing normal engine
library still reports its earlier unused capture-ownership fields; no strict
workspace-lint success is claimed here.
