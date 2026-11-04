# Selection and Continuous Sizes Implementation Plan

> **For agentic workers:** Use subagent-driven-development for the independent
> selection, UI and scan-lifetime tasks; verification-before-completion applies.

**Goal:** Implement safe selection hotkeys, marker-free size estimates, inline
drive shortcuts and continuous background sizing, then commit relevant files.

**Architecture:** Backend-owned immutable selection snapshots and ordered
per-pane actions; scan identity survives display/sort generation changes. UI
only routes input and presents cached state.

**Tech Stack:** Rust1.99.1, Slint1.18.1, Tokio; existing Windows/Linux adapters.

**Spec:** [Selection and continuous sizing](../specs/2026-11-04-selection-and-continuous-sizes-design.md).

## Constraints and review focus

Preserve lossless native paths, no application MAX_PATH cap, separate platform
implementations, public Rust style, rustfmt100 and100000-byte source limit.
No write operations are enabled. Watch stale entry IDs, rapid commands/manual
toggles, partial listings, failed queue sends, retired tabs and large cleanup.
The supplied workspace is now a user-created Git repository without a first
commit; preserve all existing work and commit only after final validation.

## Tasks

- [x] Add core selection snapshot/protocol, backend controller and ordering,
  rebase/property-expansion tests; migrate presenter selection state.
- [x] Route exact Ctrl+A/D/I in file panes, wire the installed bridge and test
  both panes plus text-editor/popup isolation.
- [x] Remove folder-size exclamation prefixes; retain warning states/tooltips
  and verify actual rendered warning colors.
- [x] Preserve size scans across same-directory sorting/refresh with raw-path
  guards and blocked-worker regression tests.
- [x] Place the drive/location strip beside its dropdown; verify compact
  Desktop/Touch hit targets, overflow and native icons/tooltips.
- [x] Update current keyboard/design/security/build-layout documentation.
- [x] Independent code review; resolve confirmed important findings.
- [x] Windows/Linux tests, strict Clippy, formatting, size guard and app builds;
  inspect native Windows UI and launch the verified executable.
- [x] Inspect staged paths/diff, exclude build/cache/temp files, create the
  initial Git commit and report its hash and build directory layout.
