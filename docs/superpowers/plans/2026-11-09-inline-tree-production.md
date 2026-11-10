# Inline provider trees: production integration

This continues the accepted TREE-01 goal. The user chose branches inside the
ordinary panel, with folder-only descendants, explicit expansion/collapse, and
Enter/double-click navigation. The backend model and Slint disclosure widget
exist, but the production panel still renders flat rows. This plan does not
redefine completion as model tests or disclosure artwork alone.

Authority: `docs/developer/INLINE_TREES.md`, TREE-01/FILTER-01/VFS-01 in
`docs/IMPLEMENTATION_BACKLOG.md`, and the user's approved inline-branch behavior.
The filesystem backend baseline is committed as `9df66c5`; the current working
tree contains further reviewed application composition and tree primitives.

## Existing foundations and verified gaps

- `tree_actor::Session` owns branch work, exact projection acknowledgment,
  occurrence selection and atomic `TreePublication`. Its work budget persists
  across pane navigation. It is not connected to `Engine::run`.
- `TreeCommand`/`TreeEvent` exist but are not variants of the global protocol.
- Root seeding reuses the exact completed native/archive entries and index.
  Provider adapters preserve raw addresses and archive ancestry without parsing
  display strings. Content capture requires the installed native provider.
- Slint has a separate disclosure callback in all layouts. Rust does not bind it
  and hardcodes zero depth/disclosure in `FileRow`.
- Native action admission requires a flat pane snapshot and immediate children.
  Archive copy assumes one immediate archive parent. A flattened display index
  cannot be passed to either as operation authority.
- Folder listing currently omits all regular files, including browsable archives.
  Prefix projection, expanded-branch watches and container attachment are pending.

## Implementation rules

Use one production controller and one typed action-admission boundary, rather
than duplicating navigation, mutation or copy engines. All provider calls,
projection, target resolution and resource reclamation remain backend work.
Keep the ordinary complete root listing if tree admission fails. A tree budget
is not a new panel-size or native path-length limit.

Rows retain occurrence identity, mount-scoped node identity and full provider
ancestry. Source entries and entered container roots are different targets:
Copy/Delete on an archive row applies to its source file, while navigation enters
the attached archive. Context-only filter ancestors may be navigated but not
selected or mutated. A late reply cannot revive a collapsed or replaced branch.

Do not enable descendant actions through synthetic `SelectedFiles`, display paths
or relaxed immediate-parent checks. Capture exact acknowledged projection and
selection revisions on backend workers, then prepare existing native proofs for
each real parent scope. Retain registrations/mounts across later navigation.
Unsupported mixed mutations fail preflight before changing any selected item.

## Ordered implementation and acceptance

### 1. Production session lifecycle

- Add global `Command::Tree`/`Event::Tree` routing and a bounded controller.
- Retain backend root entries **and index** before publishing ListingFinished or
  ArchiveListed. Match both exact allocations when enabling a session.
- Run root seeding, branch admission/application and projection on backend workers.
  Preserve one branch budget per pane and the common listing admission; cancellation
  does not release a permit while native/provider code is still executing.
- Migrate selection and publish view/selection together. Require exact Commit
  acknowledgment before branch, navigation, selection or action requests.
- Publish loading and collapse changes promptly; reject stale generations and
  preserve flat rows on budget/provider failure. Dispose retired leases off UI.

RED/GREEN tests: native/archive completed-root handoff in either message order;
foreign allocations with equal contents; pending selection during adoption;
old branch completion after collapse/navigation; blocked provider retains budget;
queue saturation and cleanup. Test the real engine command/event boundary.

### 2. Typed action capture and existing operation integration

- Capture the exact acknowledged view, selection snapshot/revision and optional
  fallback occurrence. Nonempty selection never falls back to another caret.
- Resolve a bounded batch of `CapturedTarget`s, retaining its registration and
  ancestry. Selection/view changes before admission reject the request; navigation
  after admission cannot retarget accepted work.
- Native targets supply backend-owned per-parent entry scopes to the existing
  mutation preparation path. Keep flat-pane checks unchanged. Provider target
  resolution is not native mutation proof; obtain and revalidate that proof.
- Typed archive targets preserve their source mount and complete parent/directory
  context. Extend transfer input to support mixed native/archive Copy in one queue
  job, with mount-aware traversal identities and existing checksum/readback rules.
- Resolve overlapping selected ancestors/descendants explicitly before mutation,
  using provider identity/ancestry rather than caption or string-prefix matching.
  Do not silently process the same source twice or weaken alias protection.
- Route rename/delete/metadata, F5/F6, clipboard, viewer/editor, comparison,
  properties, sizes, checksums, commands, context menus and dragging through the
  common capture path. Preserve each provider's actual capabilities.

RED/GREEN tests: multiple immediate parents and duplicate basenames/tokens;
raw/Unicode names; replaced namespace; pending selection; collapsed/context rows;
navigation after capture; archive directories and mixed Copy; unsupported mixed
Move/Delete produces zero mutations; SHA-256 destination verification unchanged.

### 3. Panel presentation and interaction

- Add pane tree state and consume projection rows in display accessors without
  materializing full UI models. Fill depth/disclosure/expanded/selectable fields.
- Bind the existing disclosure callback using the captured row identity, session
  and revision. It must neither select, drag nor navigate.
- Keep Enter/double-click navigation separate. Add Session navigation and explicit
  container attachment using model targets, not reconstructed native paths.
- Preserve root caret/selection on initial adoption, immediate selection feedback,
  scrolling and row geometry. Cancel a rectangle when structural layout changes.
- Submit bounded viewport/overscan hints; background probes distinguish unknown,
  children, loading, empty and failure states without enumerating all rows.

RED/GREEN tests: native and archive branches in both panes, Details/Brief/Thumbnails;
disclosure click isolation; keyboard and Ctrl/Shift/range/rectangle selection;
stable focus across adoption/collapse; stale callbacks; 320x200, mouse/touch and
native Slint interaction. Rendering alone does not close TREE-01.

### 4. Containers, search/filter, watches and qualification

- Retain folders/links plus catalog-qualified browsable container candidates in
  expanded native branches; do not charge unrelated files as retained rows.
  Mount containers only after explicit typed capture, not from captions or hover.
- Integrate Unicode prefix matching and optional filtering into tree projection,
  preserving ancestor context and atomically rebasing selection. Sibling locale
  sorting must never globally scramble flattened branch order.
- Add bounded expanded-branch watches or polling, generation-tagged refresh and
  cancellation on collapse/navigation. Revalidate archive sources before refresh.
- True archives inside archives need the separately tracked provider-content
  container chain (ARCH-02); never fabricate `outer.zip/inner.zip` as a native
  source. Keep that full goal item open until its own typed representation and
  ownership/invalidation tests are implemented.
- Qualify actual installed native/archive plugins on Windows/Linux, ordinary flat
  panel regressions, background operations, cancellation/backpressure, and missing
  providers. Update feature status with actual behavior and remaining capabilities.

## Current state

Completed-root retention and typed selection capture are implemented as backend
prerequisites. They bind original listing/index allocations and retain typed
native/archive context. Windows/Linux regression qualification and review passed;
see the [admission report](../../verification/2026-11-09-tree-action-admission.md).
Production
controller routing, operation consumers and panel enablement remain to be
implemented. The ordinary UI is still flat; model/widget tests are not presented
as feature completion.
