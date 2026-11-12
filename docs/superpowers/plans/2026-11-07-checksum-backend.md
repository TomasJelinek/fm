# Checksum Backend Implementation Plan

> Implement inline with the executing-plans and test-driven-development skills.
> Root coordinates shared core exports, Cargo access and later protocol/UI hooks.
> No subagents, commits, application launches or preference writes in this task.

**Goal:** Provide a bounded native checksum backend, one-pass multi-algorithm
calculation and verified no-replace GNU manifest export.

**Architecture:** New core checksum contracts; separate engine controller,
streaming worker and manifest module. Reuse native TransferService/TransferSource
for pinned reads and the existing verified transfer stage for manifest saves.
Serial I/O uses the process shared budget. UI/protocol integration follows later.

**Tech stack:** Rust, Tokio bounded channels/semaphores, RustCrypto md-5/sha1
0.10.6 and existing sha2 0.10.9 with std only. MIT branch selected, no assembly
features or additional native libraries.

**Spec:** `docs/superpowers/specs/2026-11-06-checksums-design.md`.

## Constraints and review focus

- SHA-256 default; MD5/SHA-1 are compatibility digests, not authentication.
- One reusable 256 KiB source buffer; all selected digests consume one stream.
- No final symlink/special-file reads; source validation before/after chunks/EOF.
- Recursive traversal is explicit, bounded and preserves raw native identities.
- Native roots are pinned by backend work; UI supplies paths/captured metadata,
  never fabricated native identities or filesystem I/O.
- Bounded jobs, input bytes, retained results, batches and progress. Cancellation
  never releases an in-flight native call's ownership early.
- GNU UTF-8/LF binary records with exact escaping; refuse unrepresentable names,
  parent traversal/cross-volume source paths and source-alias save targets.
- Partial export requires explicit exact included/excluded acknowledgement.
- Existing manifest targets fail explicitly; do not silently overwrite them.
- Ctrl+F8 is not complete until root integrates the real modeless UI and shortcut.

## Tasks

- [x] Write failing known-answer/one-pass/change/cancel tests and core contracts.
- [x] Add reviewed pinned dependencies and lock entries in an agreed Cargo slot.
- [x] Implement serial streaming worker, bounded traversal/results and diagnostics.
- [x] Implement shared-budget bounded controller with generation/cancel/shutdown.
- [x] Write failing GNU round-trip/alias/partial/no-replace tests; implement verified
      staging/export without a direct truncate/write path.
- [x] Run focused Windows tests/Clippy, obtain Linux verification when available,
      update dependency inventories/notices/advisory evidence and backend status.
- [x] Send root integration API and independent review focus; approved actor
      integration is tracked in `2026-11-07-checksum-actor.md`. Root owns UI and
      final release verification.

## Reference and dependency provenance

Inspected Salamander `src/plugins/checksum/dialogs.cpp` around 652-780: enabled
algorithms share each 256 KiB read, but short reads, fixed ANSI paths, link behavior
and blocking destruction are not copied. `wrappers.cpp` around 257 recognizes GNU
digest-first and tagged records; export here uses the explicitly specified GNU
binary format. No reference source/assets are imported.

Actual released `.crate` archives and full license texts are inspected under
`_build/goal/checksums/dependency-review/`. Both proposed hash crates use only
existing digest/cfg-if dependencies; sha1 also uses existing cpufeatures runtime
detection. Optional asm/OID/compression/force-soft features remain disabled.
