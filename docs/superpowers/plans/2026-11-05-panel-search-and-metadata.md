# Panel search and metadata implementation plan

> **For agentic workers:** Use subagent-driven development for independent backend, UI and tooltip tasks; root integrates preferences, compression and verification. Steps track observable deliverables.

**Goal:** Add Unicode panel-prefix search/filter, unobtrusive tooltips, compression marks and documented native builds.

**Architecture:** Backend workers prepare immutable matching projections; frontend owns native text editing, focus, rendering and O(1)/binary-search navigation. Existing canonical listing, scan and selection lifetimes remain separate from the view.

**Tech stack:** Rust1.99.1, Slint1.18.1, Tokio1.54.1, cached unicase2.10.0/unicode-segmentation1.13.3 and native platform adapters.

**Spec:** [Panel search and metadata design](../specs/2026-11-05-panel-search-and-metadata-design.md).

## Global constraints

- Always case-insensitive per user clarification; plain unshifted typing starts search.
- Preserve raw Unicode/native identities, no application MAX_PATH or normalization.
- All filesystem/CPU-heavy/indexing/large cleanup on backend workers; bounded latest-wins work.
- Separate Windows/Linux implementations, source<=100000bytes, rustfmt target100.
- No file-row tooltips, permanent help/status bands or file-writing feature expansion.

## Review focus

1. Filtered indexes and recycled EntryIds must not target hidden/other files.
2. IME preedit and shifted Hot Paths must not corrupt the committed query.
3. Escape/query/tab changes must defeat delayed results and held-click retargeting.
4. Unicode fold expansion, bidi shaping and emoji deletion must preserve text integrity.
5. Tooltip edges and unsupported compression metadata must remain truthful and safe.

## Task1: Backend query protocol and projection

Files: core search.rs/lib.rs/protocol.rs; engine search.rs/lib.rs/tests; engine manifest/lock.
Interfaces: SearchRevision, SearchRequest, SearchMatch, SearchResult; Search/CancelSearch commands and SearchReady/SearchFailed events exactly as specified.

- [x] Observe RED for missing prefix behavior, then implement Unicode/grapheme matching over native names.
- [x] Observe query-dispatch RED, implement bounded per-pane workers/source guards, and verify stale/coalesced/backpressured work.
- [x] Verify folds, raw identities, cancellation/shutdown and full-source preservation with core/engine tests.

## Task2: Preferences and native compression (root)

Files: core preferences.rs/entry.rs; common settings codec/tests; UI presenter/preferences.rs; OS-separated metadata modules/tests.
Interfaces: quick_search_filter=false, strict persisted version4; change_search_filter(enabled,send) calls search_filter_changed(); CompressionStatus on EntryMetadata.

- [x] Write/observe failing v4 migration/roundtrip/strictness and late-load edit tests; implement schema and preference merge/save.
- [x] Write/observe native metadata attribute failures; implement safe Windows flags and Linux rustix statx metadata without contents access.
- [x] Verify logical sizes, symlink/FIFO/missing paths, support-mask Unknown and unchanged filesystem contents.

## Task3: Panel interaction and rendering

Files: UI presenter/search.rs, model/presenter/input/Slint files and actual interaction/render tests.
Consumes: task1 protocol and task2 preferences/metadata; produces search_filter_changed() and projected view accessors.

- [x] Prove native TextInput color-range rendering, pass-through and IME behavior in ignored prototype.
- [x] Observe actual panel-input RED; implement query receiver, feedback, navigation/Escape, grapheme edits and focus/shortcut isolation.
- [x] Implement indexed filtered rendering/selection scopes and stale result/gesture guards without UI listing scans.
- [x] Add compression badge/accessibility and verify colored names, all themes/densities and long paths.

## Task4: Tooltip placement

Files: isolated pinned compiler-IR spike; approved build adapter/shared tooltip content plus real hover/render/input tests.

- [x] Prove maintainable placement hook without dependency-registry or generated-Rust text patching.
- [x] Observe edge/overlap RED; implement pointer gap, independent flipping/bounds and smaller font.
- [x] Verify tooltips in dropdowns/Options, all corners, large fonts and underlying click/focus pass-through.

## Task5: Build documentation and integration

Files: README.md, root build.bat/build.sh, main design/shortcuts/security/verification records.

- [x] Add platform prerequisites, build/run/debug/release instructions and fixed-argument native helpers.
- [x] Verify helper syntax, spaced paths, invalid arguments, exit propagation and inert shell-looking inputs.
- [x] Independently review integrated source, run full Windows/Linux tests and strict checks on frozen hashes.
- [x] Exercise actual helpers/builds, native Windows visual/input smoke, update current documentation and finish authorized Git integration.

## Task6: Directory-link and junction artwork (added by user)

Files: core entry/desktop contracts, neutral common enumeration classifier helper,
Windows listing/icon adapter, Linux icon adapter, engine fallback/scan/sort tests,
UI mapping/navigation/model/render tests. Backend worker and UI owner coordinate
new enum variants; no target lookup or platform bodies in the frontend.

- [x] Observe RED for folder-link classification/grouping and explicit UI navigation.
- [x] Add DirectoryLink/FolderLink contracts, Windows cached FileType classifier and system overlay.
- [x] Prefer host link artwork elsewhere, retaining recognizable backend fallback art.
- [x] Verify real junction, Unicode/no-follow behavior, blank-art fallback, directory-only navigation and skipped recursive scans.
- [x] Include the extension in final platform tests, native visual QA and design documentation.

## Task7: Reuse and generated-code audit (added by user)

Files: shared hinted-inputs/command-menu components, Rust file-key router/bridge,
tooltip compiler adapter, code style/design and verification record.

- [x] Compare stock and adapted output from the same frozen UI input; distinguish normalized token bytes from formatting.
- [x] Restore normal AST formatting instead of emitting a multi-megabyte single line.
- [x] Share hinted native controls and menu bodies while preserving popup ownership, action order and held gestures.
- [x] Extract panel key policy into a shared pure Rust function and callback bridge.
- [x] Prove and independently review private stock-tooltip factory reuse without generated-text rewriting.
- [x] Verify long RTL prefix visibility, final combined code-size savings, native input/render behavior and full platform checks.
- [x] Publish measured files/sizes and commit only handwritten source, tests and documentation.

## Task8: Explain remaining expansion and share hinted action buttons

Added after the user challenged the remaining megabytes, including a hypothetical
50% reduction. The objective is actual component/function reuse with the same
features, not shorter identifiers or output minification.

- [x] Break down actual generated methods and compare minimal two-pane, full-pane and native/custom-control fixtures.
- [x] Verify that FilePane and TipBody already have shared implementations; identify parent-specific popup/style and binding/layout scaffolding.
- [x] Measure and independently review a copied-input HintedActionButton prototype preserving eleven existing hints and plain buttons.
- [x] Adopt only the reviewed child-free wrapper and unchanged hint/callback bindings.
- [x] Verify fresh UI behavior and code generation on Windows/Linux, then reopen the native Windows app.
- [x] Record measured limits and further architectural options without promising unmeasured savings; complete authorized Git integration.
