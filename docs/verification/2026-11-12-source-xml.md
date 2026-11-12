# XML source viewer — 12 November 2026

## Change and review

The independent Source viewer previously had 18 real lexical profiles and no XML
profile or default XML routing. It now has 19. The new XML lexer handles element
and processing-instruction names, attributes, quoted values, comments, CDATA,
markup declarations and entity/character-reference spelling. XML Unicode name
ranges follow the [W3C XML 1.0 lexical productions](https://www.w3.org/TR/xml/#sec-common-syn).

Default suffixes are xml, xsd, xsl, xslt, xaml, csproj, fsproj, vbproj, vcxproj,
props, targets and resx. HTML/htm and SVG are not claimed. Existing saved routing
overrides continue to apply through unchanged host behavior. The current manifest
advertises the actual 19 profiles; the host's legacy 17-profile description remains
unchanged so an older installed package does not advertise XML capability.

The lexer is private plugin code using the released public SDK. No host code,
plugin ABI, dependency, lockfile, decoding or paging limits changed. Input is capped
at 64 KiB and output at 4096 spans. Declaration nesting is capped at 16. Scanning
advances by Unicode scalar and checks cancellation throughout; state is committed
only after successful completion. It never validates a document, expands an entity,
resolves a URL/DTD, performs I/O or builds a document tree.

Complete-line checkpoints retain multiline XML context. A viewport starting inside
a comment, CDATA or a quoted value receives the cached state or a bounded rescan;
far uncached views remain plain. Delimiters ending at EOF restore text state;
unfinished constructs retain state. XML quotes do not use backslash escaping.
The existing limits remain: 8192 emitted UTF-8 bytes per viewport, 64 KiB reads,
32 KiB maximum context rescan and 256 checkpoints. Partial enormous lines do not
create invalid intermediate checkpoints.

## Verification

Logs are under `_build/qa/source-xml-20261112/` in the authoritative checkout.

| Evidence | Result |
| --- | --- |
| `red.log` | Five tests failed because XML had no detected profile, before production edits. |
| `green.log` | Initial five XML lexer/routing/cancellation tests passed. |
| `final-tests.log` | **42 passed, zero failed, zero ignored** across the complete independent Linux plugin suite. Includes seven XML lexer tests and three public-SDK viewport tests. |
| `linux-format.log` | Formatting check passed. |
| `linux-clippy.log` | Strict Clippy over all plugin targets passed with `-D warnings`. |
| `linux-release.log` | Native `x86_64-unknown-linux-gnu` release build passed. SO is in `linux-target/release/libmidnight_viewer_source.so`. |
| `windows-tests.log` | **42 passed, zero failed, zero ignored**, complete independent Windows plugin suite. |
| `windows-format.log`, `windows-clippy.log` | Formatting and strict all-target Clippy passed. |
| `windows-package.log` | Native `x86_64-pc-windows-msvc` release DLL and package built successfully; the coordinated Windows runner exited zero. |

New tests cover all 12 default suffixes and excluded HTML/SVG defaults; actual
style roles; Unicode names and combining marks; multiline comments, quoted values,
CDATA, processing instructions and inert DTDs; declaration depth, span and input
limits; cancellation rollback; and EOF context. SDK tests verify seven-byte short
reads, starts inside comments/CDATA, UTF-16 project files with three-byte reads and
exact source anchors, huge Unicode lines and distant seeks in a virtual 10 GB XML
source. All existing profile/decoder/search/protocol tests also pass.

The Linux final wrapper initially exited on a trailing CR in its last `tail` log
path, after all three Cargo commands had succeeded. The build log and produced SO
were checked directly; the wrapper was normalized to LF. This was a reporting
script failure, not a compilation or test failure.

## Packaging and integration evidence

The root task ran `_build/qa/source-xml-20261112/windows-final.ps1` using the
existing independent plugin target and released SDK. The built package is
`_build/qa/source-xml-20261112/package/midnight-viewer-source/`, containing the source
DLL, updated manifest, README and notices. SHA-256 values recorded by that runner:

- DLL: `F1A18D926308BD6D06E10553F8B66938A078AF91C9D4CED7C6799AC4E5D1DD70`.
- Manifest: `EE4AC25999F380CAB2D48FE13A34C5E618C730993756CEB86952B8D3BD364CFC`.

Protected installation is complete at
`C:/Users/tjelinek/MidnightExplorer-20261112-user-menu-2be96843`. The installed
`viewers/midnight-viewer-source/` DLL and manifest were independently hashed and
match both package hashes above. Deployment and runtime registration are recorded
in `_build/qa/find-metadata-scroll-20261112/deployment.log`.

The root runner's backend, native-renderer and software-renderer startup smoke
checks passed against the registered build/runtime. Their logs are
`_build/qa/find-metadata-scroll-20261112/final-smoke-{backend,native,software}.log`;
UI snapshots are `final-native.png` and `final-software.png` in that directory.
These startup checks do not establish XML viewport or scrolling behavior. Actual
native desktop XML viewport/scroll qualification remains pending with the
`scroll_pacing` integration task.

The feature is bounded lexical highlighting. It does not promise XML validation,
schema resolution or highlighting at distant uncached positions beyond the existing
context budget. No installed desktop XML workflow is claimed by Linux source tests.

## Subsequent native desktop qualification

The final `d5cf41d7` protected debug runtime was exercised with the exact
`C:/Windows/System32/WwanFeatureTests.xml` file. F3 opened Source mode with visible
XML coloring; wheel down/reverse captures show movement and return to the first
line, and Escape closed the viewer. The application exited 0 with empty stderr
and unchanged preferences. This supersedes the pending Windows desktop item above.
The [foreground report](2026-11-12-foreground-scroll-resume.md) records file hashes,
caption-guarded input, captures and the remaining refill/frame-pacing limits.
Linux native desktop qualification remains separate.
