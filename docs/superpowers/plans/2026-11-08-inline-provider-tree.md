# Inline provider tree implementation plan

> **For agentic workers:** Use `superpowers:executing-plans` for the backend and
> coordinated parallel qualification. Preserve the existing dirty worktree.

**Goal:** Expand several native/archive/provider folder branches directly in
the existing panels, keeping ordinary files at the panel root and navigating
with Enter/double-click independently of disclosure clicks.

**Architecture:** Backend-owned branch instances retain mounted provider leases.
Rows have distinct, nonreused occurrence IDs and provider node references; a
caption never supplies an operation path. A bounded asynchronous enumeration
request carries branch and request identities, cancellation and its mount.
Results publish only into the same live branch/request. Immutable projections
carry display state, indentation and context, without plugin-owned resources.

**Tech stack:** Existing Rust/Tokio engine, internal provider registry, native
filesystem and archive adapters, Slint virtualized panel rows. No new dependency.

**Spec:** Stage 7 of `2026-11-05-follow-up-stages.md`, TREE-01 and FILTER-01 in
`../../IMPLEMENTATION_BACKLOG.md`; VFS identity and selection security contracts.

## Global constraints

- All enumeration, sorting/filtering and resource destruction runs on backend
  workers; UI uses immutable rows and opaque action tokens.
- Native raw paths and provider bytes remain lossless; no display-path authority
  or MAX_PATH limit. Independent plugins share only public APIs.
- No automatic traversal of links/junctions, archive mounting or network access
  from hover. Probes are bounded visible-branch work, not recursive scanning.
- `+` requires known children, `-` means expanded; empty, unknown, loading and
  failed remain distinct. Failed/cancelled enumeration cannot claim emptiness.
- Keep Rustfmt100 and source files below100,000 bytes; outputs under `_build`.

## Review focus

1. Collapse/re-expand, navigation and provider replacement invalidate old results.
2. Ancestor cycles and identical tokens from distinct mounts cannot retarget rows.
3. Filtering retains ancestor context without selecting hidden mutation targets.
4. Repeated probes, wide/deep folders and errors cannot grow retained state unbounded.
5. Selection, watches and operations use stable identity, never transient row index.

## Tasks and gates

### 1. Bounded provider-backed branch model

- [x] Add `midnight-engine/src/providers/tree.rs` and focused model tests: multiple
  independently expanded branches, root files retained/descendant files excluded,
  empty/unknown/error states, links, cancellation, cycles, resource limits and
  provider revocation. Observe regression failures before implementation.
- [x] Add a folder-only bounded listing route in `providers/lease.rs` that does
  not retain irrelevant regular files, while preserving registry validation and
  exact mount-scoped identities. Test existing general listings unchanged.
- [x] Implement monotonic branch/request IDs, captured leases and cancellation,
  explicit probe/expand/refresh/collapse transitions and stale-result rejection.
  Cross-provider container expansion requires an explicit mounted target; an
  archive-looking suffix alone never confers Browse permission.
- [x] Run engine provider tests, focused strict Clippy and raw-name/native/archive
  adapter tests. Record actual limits and any remaining integration work.

### 2. Production actor and panel integration

- [ ] Route root native/archive listings into retained provider contexts using the
  mandatory native plugin. Map current operations explicitly; no native fallback.
- [ ] Add commands/events for branch request, result and collapse using exact
  pane/navigation/branch/revision tokens; shared bounded worker permits remain
  held until provider calls really return. Cancel on tab/navigation/provider loss.
- [ ] Extend backend projection with depth/disclosure and preserve ancestor context
  through filtering, prefix search, sibling locale sorting and watched refresh.
- [ ] Update `Pane`, selection/index scopes and row actions without broadening
  mutation authority. Hidden descendants never become implicit operation targets.
- [ ] Add disclosure control to Details/Brief/Thumbnails and preserve native file
  drag, rectangle selection, Enter/double-click navigation and virtualized scrolling.
- [ ] Native Windows/Linux interaction tests and real directory/archive fixtures;
  test retained selection, unavailable provider, cycles and 320x200 layout.

### 3. Review and delivery

- [ ] Update design/API/security/hotkeys as applicable and actual feature status.
- [ ] Independent review, causal fixes, combined debug build/runtime qualification.
- [ ] Review the dependency closure before scoped commits and authorized push;
  never stage generated/build artifacts or claim TREE-01 complete from model tests.

## Execution ledger

- 2026-11-08: Existing provider registry has stable mount/node identities but flat
  production `Entry`/selection routes. TREE-01 is missing. Implementing task1 is
  a dependency step, not completion of the requested visible panel behavior.
- Linux full-format viewer qualification proceeds separately with its own Cargo
  lane. Root owns the Windows lane; no concurrent edits to shared contracts.
- Review found and regression tests reproduced stale loading after abandoned
  work, contradictory expanded probes, hidden root errors, ignored-file token
  validation gaps, retained String-capacity accounting, and revoked container
  replacement. Fixes preserve provider checks and cancellation ownership.
- Ruling: tree admission failure must leave the existing complete flat root
  listing available; these branch budgets are not new navigation/path limits.
  Typed descendant operation scopes and archive ancestry remain task2 blockers;
  no immediate-parent or captured-snapshot mutation check may be weakened.
- Typed adapter targets now retain native paths or complete archive ancestry.
  Tree entry resolution distinguishes the source container file from its entered
  root and rejects hidden/root/foreign occurrences. Explicit native capture and
  locale injection preserve production seams; cloned archive target handles
  retain the source/enclosing registrations and enforce revocation. This closes
  a backend prerequisite, not task2's production actor/selection/UI integration.
