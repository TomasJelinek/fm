# Native inline-tree clipboard implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development or
> superpowers:executing-plans to implement and verify each task.

**Goal:** Ctrl+C/Ctrl+X capture native items selected in expanded branches, or the
focused item when unselected; Ctrl+V uses the existing verified Copy/Move queue.

**Architecture:** Resolve exact acknowledged occurrences on the tree backend,
then pass an opaque retained source to the existing native clipboard worker.
The UI never converts tree display paths into filesystem authority. Reuse the
selection-admission policy shared with F5/F6, and retain the original clipboard
intent until the OS publishes it. Cut itself does not mutate files.

**Tech stack:** Existing Rust crates, Slint presenter, Tokio backend and Windows/
Linux clipboard services. No new dependency or plugin ABI.

**Spec:** [Tree panel contract](../../developer/TREE_PANEL.md), main design's
clipboard/inline-tree requirements and the user's Ctrl+C/Ctrl+V request.

## Constraints and boundaries

- Preserve selected-or-focused semantics and forbid parent `..`/context rows.
- Exact projection Arc, selection allocation/revision, generation and provider
  context grant authority; text and row indices do not.
- Four shared capture permits, existing 16-request clipboard worker queue,
  4,096 native clipboard paths/4 MiB encoded names; no new pathname-length cap.
- All resolution, native clipboard work and final provider release occur on
  backend workers, including stale results, queue refusal and shutdown.
- Clipboard snapshots are names, not frozen file identities. Paste performs the
  ordinary fresh preparation, overlap checks, checksum verification and source
  removal rules. Navigation before publication cancels pending clipboard work;
  navigation after successful publication does not erase the OS clipboard.
- Native tree roots are valid paste destinations. Virtual/archive exports need
  a distinct virtual clipboard protocol and remain explicitly unsupported, as
  in flat archive panels. Never export extraction paths or weaken archive writes.
- Keep platform implementation files separate, Unicode/raw paths, rustfmt100,
  source <=100,000 bytes and all generated/test output under `_build`.

## Review focus

- A fast selection gesture followed by Copy must capture that revision, not older marks.
- Sorting, navigation, cancellation and clipboard-open blocking must not publish a different source.
- Mixed or unavailable providers must fail before modifying the native clipboard.
- Queue refusal or stale UI replies must not release a provider on the UI thread.
- Cut/Paste must retain ordinary verified Move behavior, including later external clipboard changes.

## Tasks

### 1. Backend authority and clipboard publication

Files: core `tree_clipboard.rs`, `tree_protocol.rs`, `protocol.rs`, exports;
engine `tree_clipboard.rs`, tree controller routes, `clipboard.rs` and dispatch.

- [x] Prove capture and worker failures before implementation with targeted regressions.
- [x] Add `TreeClipboardRequest`, `CapturedClipboardFiles::intent/resolve`,
  `TreeCommand::Clipboard`, `TreeEvent::Clipboard`, `Command::CopyCapturedFiles`.
- [x] Capture exact selected/focused native occurrences, retain leases through
  native publication and reject virtual/mixed scopes atomically.
- [x] Cover cancellation before/after native open, bounded queue refusal,
  stale contexts and off-thread final release; verify Copy and Cut native intent.

### 2. Presenter and actual keyboard workflows

Files: UI `presenter/tree_clipboard.rs`, shared tree capture helper,
`tree_copy.rs`, `file_actions.rs`, `tree.rs`, presenter tests and UI bridge tests.

- [x] Prove existing Ctrl+C/Ctrl+X refusal in an adopted native tree.
- [x] Route clipboard capture through exact selection admission; share common
  request-state logic with F5/F6 rather than duplicating provider/path logic.
- [x] Cancel pending work on navigation/shutdown, retry bounded send pressure,
  correlate replies and retire stale opaque sources on backend workers.
- [x] Drive Ctrl+C/Ctrl+X/Ctrl+V on Windows through installed Slint callbacks and real Engine
  services; verify nested native source paths and destination bytes, source
  removal only for Move, and continued tree reconciliation.

### 3. Qualification and documentation

- [x] Windows core/engine/UI library and five installed-key clipboard bridge tests.
- [x] Linux core and engine library gates.
- [x] Affected Windows F5/F6 integration coverage: tree Copy two tests and tree
  Move six tests passed in the final operation rerun.
- [x] Review final source, scoped strict Clippy, workspace format/size checks,
  Windows debug build/deployment and backend/native Windows startup smoke.
- [ ] Linux UI clipboard bridge and native OS clipboard interoperability.
- [x] Update TREE_PANEL, feature/backlog status and dated evidence with measured
  scope and remaining virtual-clipboard/platform qualification limits.

Source routing, Windows clipboard bridges and Windows/Linux backend qualification
are complete for this slice. The Windows debug artifact was deployed and startup
tested; this is not a published release or Linux UI/native clipboard qualification.
See the [dated verification record](../../verification/2026-11-10-tree-clipboard.md)
and [Windows artifact evidence](../../verification/2026-11-10-settings-polish.md).
The separate yellow clickable size-warning report still requires scanner/core/UI
integration under its [own plan](2026-11-10-size-warning-reports.md).
