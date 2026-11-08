# Isolated source-position API qualification

The optional Locate request / TextPosition event, SDK support, source plugin,
and four consumer lockfiles were qualified as a 19-file change independently
of the other application work. The original snapshot used `c6e4c10`; the root
compatibility snapshot used `a3a8259` (the unrelated archive-test fixture fix).
No optional host Locate callers, viewer UI changes, or native-provider cutover
were included. Existing host Open/Viewport behavior remains compatible.

Fresh checks on 8 November 2026:

| Scope | Result |
| --- | --- |
| Windows and Linux isolated API/SDK/tools | 60 tests on each platform; strict all-target Clippy passed |
| Linux generated C header and C/Rust ABI layout | Exact match |
| Linux standalone Basic / Source / Markdown | 27 / 27 / 34 tests; formatting, strict checks and native builds passed |
| Linux exported C-loader conformance | 7 scenarios; no retained jobs, services or buffers |
| Committed host and viewer engine with the new API | 30 and 31 tests; 5 and 2 existing ignored tests |
| Committed engine against the new API | All-target check passed |
| Committed host / viewer engine / engine | Strict all-target Clippy passed |

Standalone consumers resolve only the exported registry API/SDK; their source
trees contain no private API or host crates. Their source and lockfile hashes
were checked after every gate. Third-party dependency versions and licenses
are unchanged.

The canonical draft exports match all four consumer locks:

- API: `7f1820f3ae94df7f377cc287fa7a7d5fee82b3061cbf4e78df861e70e03fc9e0`
- SDK: `8df0f3156505a88a84d67dd7e3a38807a284f0f61ee2922e739249a40aed915f`

An initial WSL export from a Windows-created snapshot exposed different file
modes (0755 instead of Git's 0644). The exact native Linux snapshot with Git
modes reproduced both hashes; source bytes and package checksums were not
modified to hide the discrepancy. This is a local draft export, not replacement
of a frozen or remotely published SDK release.

Evidence is under `_build/goal/viewer-api-publish/`: `selected-files.json`,
`verification-summary.json`, `archive-hashes.json`, dependency-boundary reports,
`logs/windows-api-*.log`, `logs/root-*.log`, and the standalone consumer logs.
Windows native plugin rebuild, full application UI acceptance, and release
performance are outside these isolated gates.
