# Checksum files — 8 November 2026

The checksum window now saves and verifies the five formats from Salamander's
checksum plugin: SFV/CRC32, MD5, SHA-1, SHA-256 and SHA-512. Windows and Linux
native pickers offer checksum filters; Save defaults to the selected extension.
Only algorithms calculated for the captured run are export choices. CRC32 is an
optional calculation setting and persists with the other algorithms.

Verify file accepts a typed or picked manifest, then requires explicit Verify.
A focused checksum sidecar pre-fills that page; an empty native panel can open it
without treating `..` as a file. Parsing, filesystem reads and hashes stay on the
existing bounded backend worker. Progress, Stop, Escape, stale generations and
late picker replies retain the existing ownership rules.

Salamander reference: `src/plugins/checksum/checksum.cpp` HashInfo and
`dialogs.cpp` SaveHashes/AnalyzeSourceFile/LoadSourceFile; `wrappers.cpp` parsers.
No Salamander source was copied. GNU/BSD/hash-only sidecars, comments, Unicode,
CRLF and upper/lowercase hex have compatibility tests. UTF-8 and BOM-marked UTF-16
are supported; legacy ANSI requires conversion with its correct source encoding.

Untrusted filenames cannot escape the manifest directory. Each native ancestor
and leaf is pinned/revalidated; links and cross-device descendants are rejected.
Missing, mismatched, changed and cancelled files cannot become Verified. Save
preflights exact paths/digests against the reader before staging, independently
SHA-256-verifies written bytes and never replaces an existing destination.
Ambiguous overlapping records are rejected instead of choosing another filename.

`crc32fast` 1.5.4 is reused as a direct dependency with std-only features and its
MIT license. It was already pinned transitively. Windows/Linux dependency captures
and generated inventories were regenerated and checked: 490 packages, 458 names,
61 direct dependencies; no new package versions.

## Regression evidence

- Initial window test failed because Verify file did not exist.
- Initial codec/CRC tests failed on unsupported parsing, missing CRC and incorrect
  SFV field order. Subsequent tests caught ambiguous BSD/GNU-looking SFV names and
  exporter/reader budget mismatches before fixes.
- Review found generated filename/format ordering and subsequent recalculation
  mismatches. One shared algorithm order and immediate suggestion tracking fix them;
  the real window interaction test switches MD5 then recalculates SHA-256.
- Existing full-row-copy regression test caught an overstrict selected-algorithm
  guard; fixed without weakening single-hash validation.
- Empty-panel toolbar test failed with the old selection gate; checksum UI entry
  now opens verification while destructive/targeted actions retain their gates.
- Independent static review rechecked the format, native traversal, generation,
  picker and UI fixes and reported no remaining actionable findings.

Artifacts/logs/render captures are under `_build/qa/checksum-formats/`. Native
Windows picker tests inspect actual COM dialog configuration without showing it;
Linux tests validate portal requests and raw path handling. They do not claim
interactive desktop testing of the system file dialogs. macOS remains untested.

## Final checks

- Windows UI: 526 unit tests and 24 checksum-window, layout, toolbar and menu
  integration tests passed, including all themes/densities at 320 × 200.
- Windows backend: 45 checksum-related unit tests passed. The format/export suite
  passed 43 tests; native verification passed 42 and protocol/actor integration
  passed 4. The checksum preferences suite passed all 3 tests.
- Linux: 91 backend/actor/native-verification tests, 17 checksum presenter tests
  and 24 checksum-window/layout/toolbar/menu tests passed.
- Scoped rustfmt passed for all 41 checksum Rust files. Source size validation
  passed for 1,272 handwritten files, all at most 100,000 bytes.
- Full-workspace `cargo fmt --all -- --check` reports existing unrelated
  formatting differences in `midnight-core/src/entry.rs` and `sorting.rs`.
  Those files are outside this task and were not changed to clear the check.
- Strict Clippy found two fixed-size-chunk suggestions in the new parser; both
  were fixed and the backend suites rerun. Its final run remains blocked by 34
  existing unused-code/import diagnostics in providers/tree code and a redundant
  async block in `tree_actor_tests.rs`. No checksum diagnostics remain in that run.

## Runnable build

The final Windows debug build is `_build/debug/me.exe` (85,580,288 bytes), SHA-256
`998cbccafa9bdb765c8ec1e86714e93627f1efc65d3cb5cd923fd59b7b860daa`.
The existing compatible plugins were deployed with this executable to
`C:\Users\tjelinek\MidnightExplorer-20261108-144102-50441492` and its matching
runtime was registered without changing checkout or ancestor permissions.

A native startup smoke test launched `_build/debug/me.exe` through that runtime,
captured `startup.png`, exited with code 0 and wrote no stderr. User preferences
had the same SHA-256 before and after:
`bcf7388cc3c3e2742fd86febf6f91a3edac29fb312fdfc6bc054492bcbbab053`.
This is a debug delivery; the release executable was not rebuilt for this task.
