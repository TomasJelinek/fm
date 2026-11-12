# Inline provider tree backend verification

This is the backend dependency step of TREE-01. The production panel remains
flat: actor routing, projection/selection scopes, disclosure widgets and native
interaction qualification are still required. See the
[implementation plan](../superpowers/plans/2026-11-08-inline-provider-tree.md) and
[API/lifetime contract](../developer/INLINE_TREES.md).

## Implemented scope

- Distinct occurrence, mount/node and request identities; root files retained,
  inserted branches limited to folders/directory links.
- Multiple independent branches, probe/expand/refresh/collapse, exact surviving
  child IDs on refresh, cancellation and late-result rejection.
- Known empty, unknown, loading, child-bearing, expanded and failed states,
  including observable errors on the invisible root.
- Explicit captured-container attachment and replacement, separate source/target
  identities, and navigation without synthesizing native archive-member paths.
- Bounded nodes, payload, depth, requests and mounted containers. Cancelled
  in-flight work retains its permit until it really returns/drops.
- Folder-only enumeration validates all observed identities before filtering,
  without retaining irrelevant regular-file rows or issuing their references.

## Causal regressions and review

The first folder-only regression failed at the general listing's 50,000-entry
limit when there were only two folder rows among 50,003 entries. The new filtered
route passes while the ordinary complete-list limit remains intact.

Additional failing regressions established and then verified fixes for abandoned
work remaining Loading, an expanded Probe contradicting displayed children,
unobservable root errors, ignored-file duplicate/changed-kind tokens, retained
String capacity omitted from payload accounting, and replacing a revoked child
mount being rejected by validation of that same obsolete mount.

An independent read-only review inspected the model, lease validation, native/
archive integration boundaries, cancellation and accounting. Its findings were
fixed without weakening provider checks. The capacity fixture preserves reserved
name capacity at the emit boundary, and a private diagnostic test checks retained
capacity directly rather than a compacted clone. Diagnostic truncation now copies
a bounded UTF-8 prefix into a fresh allocation.

Evidence notes and the integration audit are under `_build/goal/inline-tree/`.
Salamander's `src/mainwnd3.cpp` explicitly skips its tree template; it did not
provide evidence of an equivalent shipped inline tree implementation to reuse.

## Gates

Windows, current source:

- `cargo test --locked --offline -p midnight-engine --lib`: **332 passed,
  zero failed, one existing ignored test**.
- Final focused `providers::` suite after expanded identity/initial-root tests:
  **54 passed, zero failed**. This includes native/archive adapter tests, raw-name
  identity, the existing provider revocation tests and new tree tests.
- `cargo clippy --locked --offline -p midnight-engine --all-targets -- -D warnings`:
  passed.
- Engine formatting and focused Git whitespace check: passed. Repository source
  size check: **1,222 handwritten files**, all at most 100,000 bytes.

Linux current source: focused provider suite **54 passed, zero failed**;
engine all-target strict Clippy passed.
The combined Linux application/strict qualification is recorded separately in
the Linux viewer cutover report when complete.

## Limits of the evidence

Native and archive tree tests exercise the real internal adapters using owned
fixture services/captured mounts, not a production Slint tree or a real filesystem
plugin tree UI. No operation gains authority merely by appearing in a branch.
Existing direct-child mutation validation remains unchanged. Production locale
enrichment, native-plugin content capture, nested archive/container discovery,
filter ancestor context, sorting, watches and selection integration remain open.

Per-tree budgets are admission bounds, not whole-process RSS or filesystem path
limits. Token validation normally reaches its 32 MiB budget before the separate
million-callback ceiling. Known links are not recursively expanded; a concurrent
native namespace replacement is still a live read-only browse limitation.
