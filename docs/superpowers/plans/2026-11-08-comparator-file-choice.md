# Comparator file choice implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement and verify this change.

**Goal:** Offer File A and File B when panel snapshots cannot identify a unique native pair.

**Architecture:** Keep automatic pairing in the comparator backend. On incomplete or invalid
inputs, keep the session alive and show a form in the existing resizable native comparator
window. Reuse the platform PathPickerService, with all validation and file access on workers.

**Tech Stack:** Existing Rust, Tokio, Slint; no new third-party dependencies.

**Spec:** User request; `docs/superpowers/specs/2026-11-07-archive-copy-pdf-comparison-design.md`.
Salamander reference:
`src/plugins/filecomp/dialogs.cpp`, CCompareFilesDialog, and `lang/lang.rc`, IDD_COMPAREFILES.
Use behavior as inspiration, no copied implementation or MAX_PATH limit.

## Constraints and review focus

- Preserve physical A/B order and known native paths, including non-Unicode native names.
- Ambiguous, loading, parent, directory and virtual rows must never become an arbitrary file.
- Empty/missing/invalid paths keep the form editable; no shell parsing or file modifications.
- Browse cancellation preserves both fields; closing cancels the native picker/session.
- Disable edits/submission while a request is pending to prevent stale picker results.
- Keep Escape, native close, keyboard field editing, and 320x200 layout usable.

## Steps

- [x] Reproduce the missing-pair session termination with a failing service test.
- [x] Add an unspecified snapshot, partial-pair resolution and backend choice/browse/retry loop
  in midnight-core/file_compare and midnight-compare-engine/{lib,worker,file_choice}.
- [x] Add reusable A/B input form and raw-path presentation in midnight-compare-ui; inject the
  existing platform picker and reuse native window ownership from midnight-ui.
- [x] Test automatic/partial/ambiguous inputs, retry, Unicode, cancellation, native owner,
  keyboard control focus and compact layout. Review and run Windows/Linux checks.
- [x] Update design/developer docs, build debug, and inspect the rendered chooser.

Completion evidence: `docs/verification/2026-11-08-comparator-file-choice.md`.
Independent review's partial-pair finding was fixed and covered for both active sides.
