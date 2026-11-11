# Viewer and settings usability follow-up

> Use the existing parallel implementation/review workflow. Run targeted checks
> while editing, then the combined affected suite after integration.

**Goal:** Resolve the 11 November screenshot and viewer regressions, extend editor
rules and filter history, and preserve responsive, safe backend ownership.

**Architecture:** Retain Rust/Slint separation and plugin APIs. Reuse compiled
backend filters for ordered editor rules. Keep modal ownership unchanged while
showing a consistent internal icon/title/divider. Histories and settings remain
transactional; changing defaults must preserve saved explicit choices.

**Constraints:** Rust public style, 100 columns, source <=100000 bytes; platform
implementations separate; Unicode/raw paths; no new mandatory-fee dependencies.
Build outputs in _build; root coordinates Cargo. No full-file text loading.

## Parallel work

- [x] Text/Markdown scrolling: reproduce AGENTS.md and build.bat from
  D:/dev/ov-physics-trunk/ovclash; fix wheel/scrollbar ownership or extent defects
  from measured evidence. Preserve bounded on-demand reads. Add BAT highlighting
  alongside SH, with plugin tests independent of the main application.
- [ ] Image loading: measure actual discovery/start/decode/delivery times; improve
  the demonstrated bottleneck and compare before/after, retaining software mode.
- [x] External editors: priority list of enabled rules with Masks or Regex,
  executable, arguments and optional working directory. First matching enabled
  rule wins. Preserve existing editor configuration as fallback. Validate regex
  and raw-path behavior in backend; keep exact argv without implicit shell parsing.
  Rework argument editing so + starts an understandable edit, not an accidental
  empty argument. Use compact list-header controls and native pickers.

## Main UI / settings

- [x] Ctrl+M: remove DOS 8.3 name/path fields and their UI choices; handle old
  templates deliberately and document migration rather than silently failing.
- [x] Options: slightly taller responsive initial size; keep Rescan all plugins
  reachable; remove redundant plugin identifier line where full location is shown.
- [x] Previews: default longest side 96 px, retain existing sizes; display real
  backend cache location above Clear thumbnail cache.
- [x] Shared list actions: centered + / minus / arrows in Editors and Colors;
  standardize clear/remove symbols and sizes across history/cache/extensions.
- [x] Appearance: alphabetic presentation of themes without changing persisted IDs.
- [x] Dialogs: shared icon/title/divider even with native titlebar; compact Rename
  and Filter default geometry; preserve growth when resized and modal policy.
- [x] Find: Whole words beside Containing text, existing persistence/semantics kept.
- [x] Linux locations: alphabetic named drives/locations first, raw slash paths
  afterward, using platform-separated categorization and locale sorting.
- [x] Filename colors: archives yellow with per-theme contrast safeguards.
- [x] Ctrl+Shift+P: append focused file full path to command line, without quotes
  or extra spaces; use existing configurable action where possible.
- [x] Panel metadata columns: measure font-aware size/date/time widths, stable
  while scrolling; return surplus width to filename; Ext immediately after Name.
- [x] Filter dialog: history dropdown, visible clear affordance for any draft
  text, correct footer geometry. Store up to 100 unique accepted mode/text/case
  combinations; cancelled/invalid drafts do not enter history.

## Integration / validation

- [x] Share settings schema 30 between editor rules and filter history.
- [x] Targeted backend, settings migration/transaction, shortcut and layout tests.
- [x] Render at desktop/touch and narrow/large-font sizes; inspect screenshots.
- [ ] Reproduce supplied files and measure changed performance paths.
- [x] Review changes; build fresh debug and plugins that changed; startup smoke
  with native and software rendering. The final runtime/hash and evidence are
  recorded in the settings/UI follow-up verification report.
- Commit/push contains source, tests and documentation only; CI status is tracked
  on the resulting branch head rather than asserted before the remote run.
- [x] Diagnose GitHub run 43082883880: checksum quota and stale Linux UI fixtures; fixes are included. Follow the replacement run after push.

Review focus: old preferences; malformed regex; non-Unicode native names; duplicate
history; settings Cancel; small viewport and high DPI; async stale receipts;
renderer fallback; plugin/runtime versions. Tests must exercise these boundaries
without broad repeated rebuilds or replacing behavior checks with implementation
mirrors.

## Follow-up requests during qualification

- [x] Denied folder entry: retain exact viewport/cursor through loading and failure;
  report Access denied, use a font-independent retry/disclosure icon. Compare
  Salamander's saved topIndex/caretIndex restoration after failed ChangePath.
- [x] Panel error icons open owned application error dialogs. Read retained large
  diagnostic captures asynchronously on backend workers; bound UI previews and
  retain existing explicit full-copy limits, request IDs and cancellation.
- [x] Permission failures offer an explicit elevated retry for directory listing:
  authenticated one-shot helper, Windows UAC and Linux polkit, lossless native
  paths, no elevated plugin loading, no password storage or ACL modification.
  Preserve ordinary operation capture/authority; do not claim mutations elevated.
- [x] Plugins catalog shows all six currently installed viewers at normal Options
  sizes; only constrained/long catalogs scroll. Rescan remains pinned.
- [x] Profile HTML viewer startup and scrolling, then improve measured bottlenecks
  without weakening browser isolation or requiring a GPU.
- [x] Separate Jobs ledger from per-operation progress windows. Each Copy/Move/
  Delete has an owned modeless progress window with an explicit progress bar,
  speed and whole-operation chart. Multiple windows may coexist, bound to exact
  operation IDs. Preserve Retry/Skip/Cancel decisions and cancellation ownership.
  Closing progress with X/Escape requests cancellation; queue refusal keeps it open.
  Closing the Jobs ledger remains presentation-only.
  Successful automatically opened progress closes; its record stays in Jobs and
  manually opened Jobs remains open. Jobs can reopen progress for a running task.

Ownership: editor agent owns Windows/helper elevation architecture; scroll agent
owns diagnostic backend then Linux elevation transport; image agent owns denied
navigation/error presentation then HTML measurements; root owns plugin layout,
operation progress separation and final integration. Coordinate shared core and
engine dispatch files before editing. Batch UI changes before Cargo regeneration.

- [ ] Explorer-style combined progress/speed chart with discovery counts and bytes,
  per-file byte meters ready for future parallel lanes, cooperative Pause/Resume
  and backend ETA. Validate source/destination mappings and complete traversal;
  discovery does not replace identity/revision checks at actual mutation time.
- [x] Panel loading indicator delayed 500 ms; fast results never flash a spinner.

Qualification note: Windows backend (1,857 passed), Linux backend (1,675 passed),
affected frontend targets (928 passed) and UI units (889 passed) have completed
with no unresolved failures after focused corrections. Existing ignored cases are
reported separately in the verification notes. Strict all-target workspace Clippy,
formatting and source-size checks pass. The Jobs-history cache passed all four
new invalidation tests and the final rendered integration targets.
Image startup remains under measurement; HTML wheel-to-draw improved by about
21-24%, but its remaining latency is not described as resolved. Byte-based ETA
currently covers full-SHA256 Copy; native Move and Delete use item-rate estimates.
Cross-volume Move and sampled-verification Copy retain Calculating until an honest
work-budget estimate is available. The shared progress/chart, discovery, pause and
per-file meter implementations are present and tested at their direct boundaries.
