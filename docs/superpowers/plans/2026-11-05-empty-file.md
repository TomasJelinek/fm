# Empty file implementation plan

> **For agentic workers:** Use superpowers:executing-plans; native provider work
> may run in parallel in separate platform files with coordinated build slots.

**Goal:** Create a zero-byte file using a toolbar action and Shift+F4.

**Architecture:** Extend `PreparedDirectory` and the existing bounded mutation
queue. Reuse the host's naming dialog and refresh/focus behavior.

**Tech stack:** Rust, Slint, Tokio, platform-native exclusive file creation.

**Spec:** [Empty file design](../specs/2026-11-05-empty-file-design.md).

## Constraints and review focus

All filesystem work and native resource cleanup stay on backend workers. Windows
and Linux implementations/tests remain separate. No overwrite/truncation, lossy
paths, app path-length limit, shell execution or UI filesystem preflight. Test
collisions, cancellation, path replacement, duplicate confirmation and focus
inside text editors before calling the feature complete.

## Steps

- [x] Add failing native tests against `PreparedDirectory::create_file`; implement
  validated exclusive native creates in each provider, reusing base revalidation.
- [x] Add `MutationKind::CreateFile` and engine tests for confirmation, frozen
  destination, duplicate submission, cancellation and provider errors. Implement
  routing through the existing prepare/queue/worker lifecycle.
- [x] Add UI tests for Shift+F4, toolbar, Unicode input and cancellation. Reuse
  the existing dialog and operation model; preserve ordinary F4 and text editing.
- [x] Update design links, hotkeys and verification evidence. Run focused tests,
  formatting, relevant full suites and Clippy; commit and push the verified work.
