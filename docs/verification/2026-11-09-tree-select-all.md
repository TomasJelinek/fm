# Ctrl+A in expanded panels

Select all now targets only the panel's visible top-level entries, including
files, folders and directory links. Nested rows are excluded and existing nested
marks are cleared. Explicit click/range selection remains available. Context-only
filter ancestors and `..` remain unselectable.

One shared core row policy is used by the worker and the UI's immediate preview.
Regressions first failed in both paths: the backend selected three folders instead
of the two roots, and the UI preview kept the nested folder selected. Tests also
cover a filtered view containing only a context ancestor and its matching child.

Evidence and build logs are under `_build/qa/tree-select-all/`.

The combined library run passed **47 core, 547 engine and 654 UI tests**, with one
existing ignored engine test. Workspace formatting and the scoped whitespace
check passed; all 1,533 handwritten source files remain within 100,000 bytes.
The initial build could not replace a running debug executable; after it exited,
the retry completed successfully. No force termination was performed.

The 88,318,464-byte debug executable has SHA-256
`9fec4ac1d05175bb9d13498f13476af8468303a99f55b21b327026978be7156f`.

Strict all-targets core Clippy passed. Native startup checks passed for both the
new private installation and `_build/debug/me.exe` (exit 0, empty stderr, rendered
snapshots). Exact-build plugin registration, unchanged prior installation files
and unchanged user preferences were verified. This is debug validation; the
release executable was not rebuilt in this slice.
