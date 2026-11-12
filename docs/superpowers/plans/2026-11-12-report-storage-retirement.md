# Report storage retirement: reproduction and next implementation

The accepted scope for this batch is a deterministic native-resource reproduction and a reviewed
ownership design. Do not introduce a partially implemented arena or leave failing tests in the
production module graph. Root owns the engine startup ledger; this task owns report storage seams.

1. Preserve a temporary test patch under `_build/qa/report-ownership-20261112`.
2. Use real private `AppendSpool` files, full Unicode records and actual output wrappers to prove
   caller-side final destruction through both `FolderSize.warnings` and `TreeDiagnostic.details`.
3. Run the two tests once with the native Linux fixture environment. Retain the original failures.
4. Restore production source byte-for-byte and remove only exact temporary fixture copies.
5. Document the minimal guarded arena contract, unresolved context plumbing, and next-batch gates.

Production implementation requires separate review of the complete ownership and startup contract.
It must preserve independent files, error isolation, immutable prefixes, complete report contents,
security validation and existing native rejected-send cleanup. No shared append-file rewrite,
arbitrary report-count cap, silent truncation, per-drop thread or runtime-dependent fallback.

Reproduction completed: two intended native ownership failures on Linux, exact source restoration
verified. See `docs/verification/2026-11-12-report-storage-retirement.md`. The implementation remains
an explicit next ownership batch; current production source and startup capacity are unchanged.
