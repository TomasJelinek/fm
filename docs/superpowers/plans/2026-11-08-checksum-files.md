# Checksum file compatibility implementation plan

**Goal:** Save and verify the five formats offered by Salamander: SFV/CRC32,
MD5, SHA-1, SHA-256 and SHA-512, using the existing modeless checksum window.

**Architecture:** Extend the existing bounded checksum actor and native transfer
readers. Preserve immutable captured generations, cancellation, raw paths and
verified no-replace publication. Parsing, hashing and file dialogs stay on backend
workers; the UI displays results and requests explicit actions.

**Reference:** `D:/experiments/salamander-main/src/plugins/checksum/checksum.cpp`
(HashInfo) and `dialogs.cpp` (`SaveHashes`, `AnalyzeSourceFile`, `LoadSourceFile`).
SFV stores filename then CRC; MD5/SHA store hash then filename. Verification also
accepts BSD `ALGORITHM (name) = digest`, comments and hash-only sidecar files.
Existing GNU escaped-name output remains supported. No source code is copied.

## Decisions and boundaries

- Keep one checksum UI/service; a separate verifier window/actor would duplicate
  lifetime, row streaming and cancellation code.
- Append CRC32 without changing existing algorithm bit positions or preferences.
- Verification reads a bounded manifest and validates all entries before opening
  their data. Reject absolute paths, parent traversal, ambiguous/duplicate names,
  unsupported encodings and malformed checksums. Never execute a listed name.
- Resolve entries under the manifest directory. Native handles reject links and
  special files; revalidate identities before and after reads. Never report a
  changed, unreadable or missing source as verified.
- Keep the existing explicit partial-export acknowledgement and no-replace
  writes; native Save selection alone never overwrites a file.
- UTF-8 names round-trip without truncation. Resource budgets are not MAX_PATH.
- No new dependency unless its pinned free/open-source license is reviewed.
- Current dirty workspace is shared. Edit only assigned files, no broad reset,
  staging, commit or release rebuild. Artifacts belong to `_build/`.

## Tasks

- [x] Format/CRC worker: tests first for all five encodings, Salamander examples,
  GNU/BSD syntax, Unicode, hash-only files, invalid paths, malformed/empty input,
  duplicates and bounds; implement codec/export and CRC calculation.
- [x] Verification actor: add request contract and asynchronous worker integration;
  reuse checksum events with Verified/Mismatch/Missing result states. Test matching,
  corrupt/missing/changing sources, cancellation and stale generations.
- [x] Native pickers: backend Open/Save choices with checksum filters and default
  extension, using separate Windows/Linux files. Test request validation and worker
  routing; maintain native raw paths.
- [x] UI: CRC option/result, format-aware destination, Browse, Verify file page,
  status counts, retry/cancel and safe late-picker handling. Add presenter and
  actual-window interaction tests; keep responsive layout and Escape close.
- [x] Integrate: focused suites, independent review, Windows/Linux checks, debug
  build and startup check. Update design, shortcuts and security documentation.

## Review focus

Wrong file format/algorithm, names containing spaces/Unicode/separators, stale
picker/job results after close/recalculate, malicious manifest traversal and
resource exhaustion, false success on partial or changing sources.
