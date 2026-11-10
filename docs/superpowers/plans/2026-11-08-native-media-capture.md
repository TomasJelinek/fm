# Native filesystem capture for comparison and previews

> **For agentic workers:** Use superpowers:executing-plans for the owned task.
> Root coordinates independent comparator/backend and thumbnail/composition work.

**Goal:** Production comparison and thumbnails read native content only through
the installed native filesystem plugin, matching panel browsing and F3.

**Architecture:** Inject a backend capture factory into the comparator service.
Thumbnail targets carry a cancellable factory that wraps the installed provider's
captured content in the existing decoder Source contract. The shared installed
provider and bounded reader cache retain native identities and library lifetimes.

**Tech Stack:** Existing Rust services, Tokio, native filesystem public ABI and
restricted image/video workers; no additional third-party dependency.

**Spec:** VFS-01 in `docs/IMPLEMENTATION_BACKLOG.md`,
`docs/developer/FILESYSTEM_MIGRATION.md` and the approved completion goal.

## Global constraints

- Plugin implementations remain independent Cargo projects consuming public APIs.
- Non-UI work and native-resource disposal stay on backend workers.
- Preserve raw native paths, cancellation, revision validation and bounded cache.
- No fallback to direct native capture when the installed provider fails.
- Artifact output under `_build/`; Rustfmt100 and handwritten files <=100,000 bytes.

## Review focus

1. A native file exists but its filesystem plugin is absent: no content frame.
2. Cancellation during capture: propagate it and retain permits until return.
3. A cached thumbnail's source changes: reject stale data before publication.
4. Explicitly chosen comparison paths and opposite-panel defaults use the same
   capture authority; changing one must not bypass the provider.
5. More visible thumbnails than resident reader slots: retain bounded resources
   and reopen only the captured original revision.

### Task 1: Comparator capture seam

Files: `crates/midnight-compare-engine/src/{lib,worker}.rs` and focused tests.
Interface: `NativeCapture = Arc<dyn Fn(&Path, &AtomicBool) ->
Result<CapturedFile, String> + Send + Sync>`;
`Service::with_native_capture(Handle, NativeCapture) -> Service`.

- [x] Add failing real-service tests for injected content, rejection, cancellation
  and explicit Compare submissions. Supply in-memory versioned ContentSource
  values under non-existent native paths to detect any accidental path open.
- [x] Thread the factory through the worker and both source captures. Keep the
  existing standalone constructor for test/embedding compatibility, as in F3.
  Never try that constructor's native factory after an injected factory fails.
- [x] Run comparator tests and strict Clippy; root reviews integration.

### Task 2: Thumbnail capture and production composition

Files: `crates/midnight-thumbnail-host/src/{service,service_tests}.rs`;
`crates/midnight-explorer/src/{thumbnails,comparator,main}.rs` and app capture tests.
Interface: thumbnail `Capture` receives `&AtomicBool`; a public
`captured_source(CapturedFile, bool) -> Arc<dyn Source>` adapter preserves version,
length and conservative SSD metadata without opening a path.

- [x] Add failing tests for capture cancellation and source/revision forwarding.
- [x] Forward admitted-job cancellation through pinned capture; remove the
  implicit path opener from thumbnail service composition.
- [x] Inject the app's shared InstalledProvider into comparator and thumbnails;
  factories run only on admitted backend workers.
- [x] Test actual installed-provider text comparison and image/video preview
  capture plus missing-provider failure. Preserve existing cache/revision tests.
- [x] Build debug app, run focused suites/strict checks and native smoke checks;
  install the current independent packages without weakening runtime trust.
- [ ] Update migration/status evidence, review and commit only coherent reviewed
  changes. Remaining watch/mutation/transfer/Find/provider migration stays open.

Review found a startup lifecycle bug: the first 100 ms thumbnail timer pulse
stopped the service permanently while the main window waited hidden for settings.
A real timer test now holds the window hidden for 150 ms and proves that it still
accepts work before showing. Stop is reserved for destroyed windows/controller
shutdown. A stopped service rejects later target submissions. Both failures were
observed before the fixes and pass afterward.

Ruling: this capture change is not committed independently of the still-uncommitted
native-filesystem host cutover it requires. The separate reviewed archive change
was committed and pushed as `d4c912b`; the broader VFS change must be reviewed and
committed with its required public API/host/plugin composition, not as a broken
partial commit.
