# Mouse activation implementation

- [x] Audit Salamander click/execute behavior; freeze OpenService/protocol.
- [x] UI regression RED then actual pointer routing/file activation/Ctrl selection.
- [x] Native per-OS classification and literal default-association launch tests.
- [x] Engine bounded opener and helper lifecycle; error, isolation and stale-result tests.
- [x] Update primary design/security/controls documentation.
- [x] Windows/Linux tests, strict Clippy, formatting/size, build and smoke.
- [x] Fresh review, fix material findings and relaunch the updated Windows app.

No Git repository exists; retain evidence under `.superpowers/sdd/mouse-activation`.

Final evidence: Windows 215 passed (two subprocess-only fixtures ignored directly
but covered by passing parents); Linux 207 passed. Both strict Clippy/build/smoke
and 89-file size guards passed. Windows native UI smoke screenshot inspected;
updated FM launched visibly as the desktop user. See
`docs/verification/mouse-activation.md` for test boundaries and results.
