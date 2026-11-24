# Find usability implementation plan

> **For agentic workers:** Use the scoped ownership assignments in this plan; the orchestrator integrates and holds the shared Cargo build lease.

**Goal:** Remember Find placement and recent filename masks, scan without an entry-count ceiling, retain at most 100,000 matches, and make fed results support normal file actions, including Make File List and Checksums.

**Architecture:** Extend the existing preference codec and accepted-search path. Keep native geometry sampling in UI adapters, filesystem work in backend services, and fed-result operations bound to immutable result identities.

**Tech stack:** Existing Rust services and Slint components; no new dependencies.

**Spec:** User requirements in this chat, including the 100,000-result cap, working existing jump behavior, Ctrl+M, Ctrl+F8, and other applicable file actions for fed results.

## Constraints and review focus

- Preserve old settings, strict validation, Options preview isolation and rejected-save retries.
- Keep raw native paths and existing file/revision checks; presentation paths never authorize operations.
- Retain bounded result memory, depth, content work and cancellation while removing the default scan-count limit.
- Restore reachable placement after monitor changes; save closing windows even during application shutdown.
- Recent masks are exact accepted strings, newest first, unique and bounded to 30 entries.
- Resizing, focused metadata and Ctrl+M must act on fed results, including frozen selection after panel navigation.

## Tasks and ownership

- [x] Settings owner: add validated Find preferences, schema compatibility, MRU updates, placement saves and independent Options behavior; add codec and presenter regressions.
- [x] Search owner: default to unlimited examined entries in native and archive scans, retain explicit caller budgets, raise result cap to 100,000; verify a streamed million-entry fixture with a late match.
- [x] Feed owner: resize columns, show focused metadata and aggregate status, expose Go to file, preserve successful jumps and refused navigation state, route Ctrl+M; add input and presenter regressions.
- [x] Orchestrator: prefill/edit/select mask history, expose found counts at compact sizes, capture and restore Find geometry, connect frozen feed captures to Make File List, update current documentation and stale acceptance expectations.
- [x] Feed owner: prepare initial result entries and indexes in a cancellable backend worker; preserve the old panel until source-window close is admitted and reject stale pane/search results.
- [x] Orchestrator: run targeted red/green tests and integration checks with one Cargo owner, review every handoff and record verified evidence.

The user authorized committing and pushing the integrated changes directly to main.

## Verification

Compile affected core, settings, engine and UI test targets once per coherent implementation stage; execute their focused suites with bounded concurrency and desktop/input cases serially. Reuse the existing Docker build image and offline cache. Keep generated output under `_build/` and caches under `.tools/`. The earlier whole-workspace attempt exhausted practical disk headroom; prioritize the affected targets and report the scope actually verified.

Verified results: [638 targeted tests passed, six ignored](../../verification/2026-11-24-find-usability.md). Strict Clippy, formatting, source-size and whitespace checks passed.
