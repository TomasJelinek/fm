# Inline tree viewer implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for implementation and review.

**Goal:** F3 and Alt+F3 open the focused regular file while inline folders are expanded; Space follows the captured visible file order.

**Architecture:** Inline descendants contain folders only. Existing regular files are exact aliases of the retained root listing. Pass the immutable tree projection and focused occurrence to the viewer backend; validate aliases and normalize only those visible root files on the backend before invoking existing native/archive capture. Do not grant generic tree mutation authority or interpret display paths as provider identities.

**Tech Stack:** Existing Rust, Tokio, Slint and viewer host crates; no dependencies or plugin ABI changes.

**Spec:** `docs/developer/TREE_PANEL.md`, `docs/developer/INLINE_TREES.md`, and the main dual-pane design's F3/Alt+F3 and next-file requirements.

## Global constraints

- Preserve the dirty working tree and platform separation; no unsolicited commits.
- All non-UI work and large allocations belong to backend workers.
- All test/build artifacts live under `_build/`; one Windows Cargo lease.
- Never view parent rows, directories, links, unreadable members or synthetic descendants as files.
- Native content still uses the installed provider's secure capture; archives retain their captured mount and source version.
- Current tree branches contain only directories. Future nested-container files require typed target support and remain rejected here.

## Review focus

- Tree row IDs can collide with original root EntryIds: map using `original_root_entry` and validate exact root identity.
- Filtering and descending sorting must determine Space order without adding hidden files.
- Pane navigation after F3 must not retarget an already captured viewer.
- Parent cursor, pending projection, unreadable archive members and stale/foreign root captures must fail closed.
- HTML relative resources in an archive must continue to use member parent IDs and mount authority, never extraction labels/native temp paths.

## Task 1: Backend normalization and presenter routing

- [x] Add an optional tree capture to ViewerLaunch containing an immutable projection and focused occurrence; initialize flat launches to None.
- [x] Write/run failing tests for native/archive root-file mapping, filter/sort order, stale/foreign input, unsupported descendants and pending UI state.
- [x] Normalize visible regular root files on the viewer backend, reusing existing capture paths; avoid duplicating native/archive readers.
- [x] Enable presenter F3/Alt+F3 for settled root-file occurrences, preserving other action guards and startup viewer choices.
- [x] Run focused Windows tests and lint; report exact files and evidence.

## Task 2: Integration, review and delivery

- [x] Exercise Engine/Slint tree publication -> actual presenter ViewerLaunch -> backend capture, including archives and panel navigation.
- [x] Independently review all changes and fix findings.
- [x] Run relevant broader Windows and Linux regression tests, source-size/format checks.
- [x] Build/deploy the debug application with protected runtime registration and run startup smoke.
- [x] Update design, API/integration docs and verification report with precise limitations.
