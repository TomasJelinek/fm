# Markdown tables and installed fonts

**Goal:** Make ordinary Markdown documents readable with native tables and a
discoverable, persistent font chooser.

**Architecture:** The independent Markdown plugin parses bounded source windows
and emits validated semantic data through the public API. The host lays out and
paints that data on viewer workers. Slint presents rendered viewports and short
input callbacks. Font enumeration uses the existing renderer font database on a
worker. Existing viewer preferences store the selected Markdown family and size.

**Scope:** GitHub-style pipe tables (header, column alignment, inline formatting,
empty cells), task-list markers and strikethrough; maintain existing CommonMark
headings, emphasis, lists, code, links and source mapping. HTML stays inert and
remote images/resources are not fetched. This is not an HTML/browser engine.

## Work

- [x] Font chooser: replace manual family entry/generic buttons with installed
  families, preserve selection and unavailable-family fallback, keep the compact
  toolbar and Escape behavior. Test worker discovery, persistence and UI input.
- [x] Public semantic table representation: bounded, validated, version-aware,
  preserving native source coordinates and existing non-table document data.
- [x] Host layout: table columns, headers, alignment, cell wrapping and borders;
  preserve scrolling, source mapping and selection without loading whole files.
- [x] Plugin: enable relevant pulldown-cmark options and emit the public table
  representation. Test source anchors, Unicode, malformed/huge tables, paging,
  cancellation and safe handling of HTML/URLs. Plugin tests stay in the plugin.
- [x] Integrate exported SDK, run focused API/plugin/renderer/engine/UI tests,
  inspect native captures with tables and selected fonts, build debug app/plugin,
  register a matching trusted runtime and verify startup.
- [x] Update design, API and supported-Markdown documentation with actual limits.

## Constraints and review

No new paid dependency; reuse pinned pulldown-cmark and Parley. No file/network,
font discovery, parsing or layout on the UI thread. All temporary output stays
under `_build`; toolchains remain in `.tools`. Preserve the existing dirty
worktree and unrelated feature work. Handwritten source stays below 100,000 bytes
and Rust formatting follows the repository's 100-column rustfmt settings.

Review small windows, proportional/CJK fonts, multi-line cells, page boundaries,
copy/search source positions, abusive cell counts and old plugin compatibility.

Completed evidence: [Markdown tables and fonts](../../verification/2026-11-08-markdown-tables-and-fonts.md).
