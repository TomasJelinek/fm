# Panel interaction and diagnostics implementation plan

**Goal:** Resolve visible-tree admission races, make folder errors actionable,
improve document/selection visibility, complete native inline branch actions,
and make Ctrl+D clear the complete panel selection.

**Architecture:** Preserve backend-owned typed occurrence captures and existing
native services. Explicit row actions must survive unrelated background probe
publications while stale/replaced identities remain rejected. Diagnostics separate
user-facing reason/path from complete technical evidence; permission elevation
reuses the explicit single-use native request. No OS checks or bounds are bypassed.

**Scope:** User screenshots and instructions dated 12 November. Existing Rust/Slint
workspace and per-platform implementations. Targeted tests during work; coherent
batch validation after all changes; headless targets may run concurrently, desktop
input checks stay serial. Source <=100000 bytes, rustfmt100, outputs under _build.

- [x] Tree scheduling: reproduce current-visible-publication race; preserve bounded
  intent scheduling/row identity under probe, refresh, navigation and shutdown.
  Owner tree_queue_repair: engine tree controller/mailbox/worker and presenter/tree.
- [x] Folder diagnostics: concise themed reason/path, optional technical detail,
  no repeated report bookkeeping or escaped newline artifacts. Enter/double-click
  a denied folder opens the same dialog with explicit elevation when supported.
  Owner folder_error_ui: panel_warnings, notice state/windows and UI bindings.
- [x] Panel visuals: brighter document/plaintext defaults with exact-default-only
  migration, marked selection distinct from caret, search SVG instead of glyph.
  Owner panel_visuals: filename colors/settings migration, theme/row/search Slint.
- [x] Deselect default: Ctrl+D -> selection.clear, Ctrl+Shift+D unassigned.
  Preserve deliberate saved mappings; files-only clear remains an unbound
  compatibility action but leaves the ordinary F10 selection menu. Verify default,
  custom mapping, input focus, live menu and native/archive/tree selection scopes.
  Owner root: core catalog/tests, menu and keyboard integration tests, docs.
- [x] Remaining native inline actions: open, properties, native context menu and
  comparison use captured row identities and common backend services. Unsupported
  provider capabilities stay explicit. Allocate files after tree scheduling fix.
- [x] Integrate and review all changes, targeted Windows/Linux validation, visual
  snapshots, fresh debug build/private deployment; commit and push relevant files.

Review: stale row removal/replacement; queue saturation vs unrelated publications;
elevation refusal/cancel; raw paths containing backslash+n or control characters;
custom colors/bindings; selected+focused/inactive contrast; no-target parent row;
no implied archive mutation from native branch services.

Integration also preserves the main-branch audit fixes for bounded worker/result
ownership, cancelled requests, lazy history models and bounded settings reads.
The additional PDF preset is coral red, independent of other document rules;
only exact historical default palettes migrate automatically.
