# Markdown native and archive viewer checks

On 9 November 2026 the actual application F3 route was exercised with an owned
333-byte Markdown fixture, first as a native file and then as `check.md` inside
an owned ZIP. Both opened the installed Markdown renderer; both viewer windows
closed on Escape while the main application remained alive. The user's saved
preferences SHA-256 was identical before and after each run.

Visual inspection of both captured viewer windows confirmed rendered headings,
bold and italic spans, inline code, a two-column table, a list, a fenced code block
and Czech/Japanese text. The content was not displayed as raw Markdown. The archive
panel also showed the fixture member's modification date/time. No source change
was necessary to make these two routing/rendering cases pass.

The check used the owned diagnostic application at
`C:\Users\tjelinek\MidnightExplorer-20261109-panel-probe\panel-baseline.exe`,
SHA-256 `B3B8F9C69E481EA3812575681FD8E4929BB98C02363B37D413A42E4A149EE055`,
with its installed viewers/filesystems and software rendering. It preserves the
normal application composition and callbacks; its diagnostic change holds the
smoke-test window open with in-memory preferences. This is not a claim that the
user's older desktop executable or the final combined build has been qualified.

The fixture driver is `_build/qa/provider-runtime/markdown-probe.ps1` (add
`-Archive` for ZIP). Logs, binary hashes, results and inspected images are under
`_build/qa/markdown-runtime/` and `_build/qa/markdown-archive-runtime/`.
Initial harness attempts failed because they targeted a not-yet-sized native
window or sent End/F3 before the archive listing was committed. The final driver
waits for a usable owned client window and retries selection/F3 while waiting for
the viewer. Those harness failures were not treated as application crashes.

The reported plain-text appearance has not been reproduced in this candidate.
Final deployment still needs to preserve the installed Markdown plugin and its
extension/mode selection, then repeat these checks against the delivered binary.
