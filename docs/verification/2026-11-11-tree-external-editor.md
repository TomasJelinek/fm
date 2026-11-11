# External editor from adopted inline trees

Status: source implemented; 38 focused Windows checks pass, including the real F4
input route on both panes. Combined application/Linux gates are tracked by the
parent qualification run.
This closes only the F4 external-editor portion of TREE-01. Rename, deletion,
metadata, checksums, comparison and context-menu consumers remain separate work.

F4 keeps its existing focused-file contract, independent of selection. The local
Salamander reference (`D:/experiments/salamander-main/src/fileswn5.cpp:1228`,
`CFilesWindow::EditFile`) likewise obtains the caret file and rejects non-disk
panel editing. Its ANSI/path-length and shell-command construction are not copied.

The production path captures the exact acknowledged tree projection and
focused occurrence on its backend model worker. It accepts only ordinary native
root files while branches are expanded, and retains provider authority and bounded
capture ownership. Display captions/paths and occurrence IDs never become native
paths. Archive entries, links, parent navigation, context-only rows and stale
publications remain rejected. UI receipt correlation preserves the original pane
and request; navigation/close cancels outstanding capture/launch intent. The
external-editor service revalidates the live regular file immediately before
spawn. External editors receive paths, so later path replacement/editor behavior
remains outside the file manager's transactional guarantees.

## Verification

Logs are under `_build/qa/tree-editor-20261111/` unless stated otherwise.

- `ui-red.log`: old F4 tree guard reproduced five failures; one blocked-input
  regression passed. The typed route was enabled afterward.
- `abort-red.log`: deterministic current-thread shutdown reproduced provider
  destruction on the async owner before the delivery task's first poll.
- `_build/qa/transfer-stage-retry/editor-capture-green.log`: all eight backend
  cases pass with unchanged real session/provider boundaries, including archive
  File refusal, cancellation/revocation, shared capacity, exact publication,
  parent-path policy and abort-safe worker destruction.
- `green.log`: 37 passing tests: four core tests, eight engine capture/lifetime
  tests, three engine editor integrations, 20 UI library tests, and two real F4
  integrations (native left/right panes). The original guard now routes through
  typed capture; existing flat F4 and editor configuration tests remain green.
- `guarded-actions-green.log`: the strengthened existing tree-action test passes;
  directory F4 and unsupported mutation/checksum consumers remain blocked.
- Presenter coverage includes unrelated selection, captured-pane switching,
  frozen settings, both command queues, same-session reprojection, cancellation
  and duplicate/stale receipts. Engine integration verifies source resolution
  and final destruction on worker threads, cancellation before/during resolution,
  unavailable service and preservation of the exact native path.
- No desktop editor application was spawned. The actual Slint F4 fixture uses the
  production Engine/tree lane with no EditorService and checks the expected
  terminal unavailable-service result after successful typed capture. The engine
  service fixture records its invocation and returns an injected launch error.
  Existing native editor implementation and literal argv/regular-file checks are
  unchanged; this run does not claim a new OS editor interoperability test.

The focused GREEN command reused the application's feature graph:

```text
cargo test -p midnight-core -p midnight-platform-common -p midnight-ui \
  -p midnight-engine -p midnight-explorer --lib --test editor --test tree_editor \
  editor --no-fail-fast --locked --offline --target-dir _build -- --test-threads=1
```

The Windows environment helper was sourced first; TEMP/TMP pointed to a fresh
per-run QA directory to retain private report-directory ownership. No ACL was
relaxed. The built UI test binary then ran the exact guarded-action regression.
Both behavior RED logs are retained, and independent read-review reported no
remaining actionable capture/lifetime issue.

## Scope of descendant coverage

Production expanded branches continue to enumerate folders only. F4 does not add
file rows to those branches or enable directory editing. The capture interface is
occurrence-based and depth-independent; a synthetic presenter File descendant
checks dispatch identity without claiming that ordinary folder expansion exposes
such rows. Actual backend coverage opens a focused root file while folders are
expanded and other rows are selected.

## Integrated qualification

The [combined batch record](2026-11-11-queue-batch.md) records final Windows
and Linux gates, strict checks, the matching debug build/runtime, and native/software
startup verification. It supersedes pending integration notes above.
