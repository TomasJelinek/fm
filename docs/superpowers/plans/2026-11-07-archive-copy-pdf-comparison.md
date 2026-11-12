# Archive, PDF and comparison implementation plan

> **For agentic workers:** Use superpowers:executing-plans or
> superpowers:subagent-driven-development. Track completed checks below.

**Goal:** Faster browsing, verified archive extraction, PDF viewing and text/image
comparison, plus larger location icons and useful default filename colors.

**Architecture:** Keep plugin parsers isolated, reuse captured source identities
and the existing operation queue, and keep Slint presentation bounded. Independent
archive, PDF and comparator work can run concurrently with coordinated contracts.

**Tech stack:** Rust, Tokio, Slint, existing SHA-256 transfer service; new parser
and diff libraries only after pinned license review.

**Spec:** `../specs/2026-11-07-archive-copy-pdf-comparison-design.md`.

## Global constraints

- Non-UI operations run on backend workers; bounded resources and cancellation.
- Plugin projects share public APIs only and own their tests.
- Separate platform files; rustfmt 100 columns; handwritten files <100000 bytes.
- Free dependencies, preferably permissive OSS; inventory and notices updated.
- Build/QA output stays under `_build`; preserve native and provider identities.

## Review focus

- Archive member names cannot escape the destination or become device paths.
- Stale selection/navigation cannot retarget a queued copy or comparison.
- PDF pages and image size differences cannot cause unbounded allocation.
- Removing default color rules survives preference round-trip.
- Larger native icons remain centered and do not overlap drive captions.

## Tasks and evidence

- [x] Location icons: update `theme.slint` and `location-controls.slint`, preserve
  row/control bounds; verify `midnight-ui --test panel_refinements`.
- [x] Default rules: `midnight-core/src/filename_colors.rs`, preferences and
  common settings serialization; tests cover category matching, custom rules,
  legacy upgrade and explicit empty round-trip.
- [x] Profile panel listing/indexing and fix demonstrated avoidable work; retain
  ordering, cancellation and snapshot tests, record before/after fixture timings.
- [x] Archive plugin: metadata index, lazy ZIP reads and indexed children; test
  malformed headers/CRC/limits; extend public formats after license review.
- [x] Extraction: `ArchiveCopyRequest` captures `ArchiveLocation`, entries Arc,
  selected IDs, fallback and native destination. Backend resolves into verified
  copy queue; F5 draft keeps captured virtual identities. Test traversal, source
  changes, checksum mismatch, conflicts and cancel cleanup.
- [x] PDF: public `DocumentMetadata` and `PageViewport` messages plus document
  SDK; independent PDF plugin and tests; host pagination and viewport integration.
- [x] Comparison: new backend/UI crates, `files.compare` shortcut and captured
  pair routing; bounded diff and sandbox image frames; test all three image modes.
- [x] User Commands design: inspect Salamander User Menu fields, expansion and
  invocation; specify toolbar, settings deep link and safe argument handling.
- [x] Follow-up: reduce the window minimum to the user's final **320×200**
  logical pixels in all densities and font sizes, and verify
  responsive toolbars, panel columns and dialog action reachability.
- [x] Persist exact normal physical window placement and monitor; recover onto
  primary display if missing. Viewer/comparator inherit parent bounds; Escape
  always closes independently of shortcut overrides.
- [x] Guard parent navigation against file operations; fix Ctrl F1–F12 captions
  and retain the action and pane captured at the actual mouse press.
- [ ] Reproduce and fix first-character keyboard-settings search crash. Native
  and software typing regressions pass, but the reported crash is not reproduced
  and is not claimed fixed; see `docs/verification/2026-11-07-ui-followups.md`.
- [x] Movable dialogs; remove metadata file-preview pane and add an original
  compact attributes/time icon. Preserve backend confirmation and validation.
- [x] Resolve shared-checkout plugin startup using a verified private runtime
  and a desktop shortcut; retain trust checks on all plugin discovery paths.
- [x] Docs/licenses, focused Windows/Linux checks, release build and native UI
  validation; fresh code review and fixes for the actionable findings.
- [x] Commit/push the verified changes to the existing branch.

## Execution record

Baseline: `fa8d6f03d93cacd5eb3511b7fdd2e52d613d4676`, initially clean.
Existing feature branch is retained; no additional worktree/build duplication.
Delegation: archive_next owns archive plugin/protocol/backend, pdf_next owns PDF
plugin/public raster API, compare_next owns comparator; root owns integration,
core archive request, presets, location icons and master docs. Shared manifest
and lockfile edits are coordinated before dependency resolution.

Fresh review found two important defects, both fixed and re-reviewed:

- Archive F5 initially included selected rows hidden by a filter. The captured
  display index now constrains selection/fallback resolution on the backend;
  regression tests cover mixed visible/hidden and wholly hidden selections.
- New shortcut defaults initially conflicted with older explicit assignments.
  Compatibility handling now preserves user assignments while still rejecting
  conflicting explicit overrides.

Default color matching now waits for saved preferences and nonempty committed
listings, avoiding redundant startup work. The existing cancellation test now
correctly cancels only the populated panel. CI independently builds/tests archive
plugins on Windows/Linux and checks the public filesystem API. PowerShell viewer
wrappers preserve absent environment variables across sequential plugin builds.

The Windows release UI was visually inspected using
`_build/qa/new-features-release-ui.png`; enlarged drive/location icons remain
centered inside unchanged controls, and the entire bottom toolbar renders.
The actual release PDF worker also passed both native AppContainer host tests.
