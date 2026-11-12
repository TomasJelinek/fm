# Inline tree prefix search implementation plan

> **For agentic workers:** Use `superpowers:subagent-driven-development` for the
> separate backend and presenter slices; the coordinator owns integration checks.

**Goal:** Keep the accepted type-to-search/filter interaction working after a
panel adopts inline directory branches, for native and archive roots.

**Architecture:** Extend the existing acknowledged tree reprojection transaction
with transient search options. The backend reuses the flat panel Unicode matcher,
builds match positions and publishes rows/selection together. The presenter keeps
one latest desired query plus at most one in-flight options request; typing does
not wait for a provider call or create an unbounded command backlog.

**Tech Stack:** Existing Rust, Tokio and Slint; no new dependencies or plugin ABI.

**Spec:** [Accepted panel search](../specs/2026-11-05-panel-search-and-metadata-design.md),
[inline trees](../../developer/INLINE_TREES.md), and
[current panel integration](../../developer/TREE_PANEL.md).

## Global constraints and decisions

- This implements already requested search and tree behavior; it does not change
  the broad application goal or weaken guarded tree operation consumers.
- Use the shared dirty feature checkout. A new checkout from HEAD would omit the
  tree implementation; do not reset, stage or commit unrelated work.
- Generated artifacts go under `_build/qa/tree-prefix-search`; main Cargo output
  stays under `_build`. Coordinate one main build lease.
- No filesystem work, Unicode scans or full match-index rebuilding in UI callbacks.
- Preserve exact projection/selection identity, raw names, cancellation, bounded
  mailboxes, deferred reclamation and 100,000-byte source limits.
- A search examines loaded visible tree rows; it does not recursively enumerate
  unopened folders. Persistent filters combine with the transient prefix. Filter
  mode retains only matching rows and the necessary unselectable ancestors.
- Empty search restores the persistent-filter projection, keeping a usable caret.
  Jump mode preserves selection; filter mode removes hidden/context-only marks.
- A transient query has a 4,096-byte admission budget, not a filesystem path limit.
  Invalid/oversized requests leave the last accepted view intact.

## Shared interfaces

Add `TreeSearchOptions { query: String, filter_matches: bool }`, deriving Default,
Clone, Debug, PartialEq and Eq, to the core tree protocol. Add `search` to
`TreeProjectionOptions`. Its default is the existing no-search behavior.

`TreeDisplayIndex::search_rows: Arc<[usize]>` contains ordered projection positions
with `TreeDisplayRow::search_match == true`. Its allocation participates in the
projection budget. Existing row `prefix_end` contains the original display UTF-8
grapheme boundary, never a folded-string offset. Context rows never match.

No new filesystem request or native operation authority is introduced. Reuse
`TreeCommand::Reproject` and its exact correlated success/failure receipts.

## Review focus

1. Rapid typing/Escape, full command queues and branch completions must converge to
   the latest query without stale first-match focus or dropped input.
2. Context ancestors, hidden matches and zero results must never authorize a stale
   caret, selection or activation.
3. Unicode folding expansions, combining graphemes, duplicate captions and opaque
   archive names must preserve raw identity and honest highlighting.
4. Sort, persistent filter and quick-filter preference changes must compose with
   the active query, including Options preview and Cancel.
5. Navigation/refresh/shutdown must retire obsolete queries and results; a tree
   publication must not resurrect flat selection or search projections.

## Task 1: Backend projection and protocol

Files: core `tree_protocol.rs`, `tree_view.rs`; engine `search.rs`,
`tree_projection.rs`, `tree_actor.rs`, `tree_controller.rs` and its worker;
focused core/engine tests and `tests/tree_filters.rs` as needed.

- [x] Add failing Unicode/jump/filter/ancestor/selection and malformed-request tests.
- [x] Implement the interface above, reuse the existing Unicode prefix matcher,
  and validate query allocations before bounded mailbox admission.
- [x] Keep matching backend-only and preserve all-or-nothing reprojection.
- [x] Pass scoped backend tests and review exact source/selection/cancel fences.

## Task 2: Presenter input and continuity

Files: UI presenter `search.rs`, new `tree_search.rs` if useful, `tree_options.rs`,
`tree.rs`, `filter.rs`, presenter sort handling and focused presenter tests.

- [x] Add failing tree input, cursor movement, Escape, zero-match, quick-filter
  preference and stale-receipt regressions.
- [x] Replace the current unavailable notice with bounded latest-query delivery.
  Preserve search in sort/filter requests and handle selection/branch races.
- [x] Read the backend match index for count and cursor movement. Keep match
  highlighting current and activation fenced while a query is pending.
- [x] Pass presenter regressions and preserve the original flat panel behavior.

## Task 3: Integration, review and deployment

Coordinator owns new Rust/Slint integration tests under `crates/midnight-ui/tests`,
documentation and the shared build lease. Consult Salamander quick-search input
for expected interaction while retaining the user's later Unicode/filter rules.

- [x] Exercise real engine-to-presenter search with expanded native/archive roots,
  multiple layouts, rapid input, filter/sort, pending selection and navigation.
- [x] Review backend and UI slices independently; fix findings before qualification.
- [x] Run affected suites, strict scoped lint, formatting and source-size checks.
- [x] Rebuild/register the tested debug app and smoke-test the matching runtime.
- [x] Update feature status and tree documentation with exact verification scope;
  keep descendant operations, branch watch/refresh retention and other goal gaps.

## Task 4: Archive extension sorting found during integration review

The current shared comparator and locale-key preparation both obtain extensions
from `Entry.path`. Archive entries intentionally have empty native paths, so this
silently makes extension sorting fall back to names. Correct the existing sort
requirement while preserving the accepted no-fabricated-native-path contract.

Files: core `entry.rs`, `sorting.rs` and focused native/archive sorting tests.

- [x] Observe a failing archive extension-order test before implementation.
- [x] Add one borrowed extension accessor used by preparation and comparison.
  Native entries retain host path semantics; archives use the original UTF-8
  basename without interpreting host separators. Unknown encoding stays unknown.
- [x] Cover locale order, both directions, directories first, dotfiles, trailing
  dots, Unicode, caption mismatches/duplicates and native opaque names.
- [x] Review the fix independently, then repeat the affected core sorting and
  tree/UI integration checks against the final source before debug deployment.

This follow-up changes no plugin ABI, dependencies, operation identities or UI
control. Its production edits wait for the current package test build lease.

## Task 5: Small-window Options footer found during Linux qualification

The existing dialog movement/resize matrix exposed a real four-pixel overflow of
Options' Cancel button at 320 by 200 with the touch font set to 32. The other
dialogs already use an anchored, scrollable action footer; Options' single
horizontal action row lets native button minima push Cancel outside the window.

- [x] Reproduce with dialog/stage labels and keep the strict geometry assertion.
- [x] Reuse `DialogFooter` for Options; keep Cancel anchored and let the action
  region scroll when necessary. Preserve Reset, OK, status and initial focus.
- [x] Preserve Cancel during validation and prevent dismissal while saving;
  the shared dismiss-enabled property defaults to the existing enabled behavior.
- [x] Qualify Windows/Linux compact layout, large fonts, movement, keyboard and
  transaction controls; run strict scoped lint and rebuild the final debug app.
