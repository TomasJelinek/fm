# Archive container capture through the native filesystem plugin

> **For agentic workers:** Use superpowers:executing-plans; root owns host/engine
> composition, delegated work owns native-host revision compatibility only.

**Goal:** Both new and restored native archive mounts obtain content only through
the installed native filesystem plugin, preserving saved archive addresses.

**Architecture:** Supply the engine a cancellable capture callback taking an
optional persisted revision. Keep the exact native locator separate from the
pathless content lease in the archive host. Native-host code checks revision
compatibility; archive decoders remain independent and receive only bytes.

**Tech Stack:** Existing Rust/Tokio and public native/filesystem protocols. No new
dependency or plugin/API rebuild is required.

**Spec:** VFS-01 and ARCH-02 in `docs/IMPLEMENTATION_BACKLOG.md`, native media
capture plan and `docs/developer/FILESYSTEM_MIGRATION.md`. The user already asked
to finish migration; this is an implementation boundary within that scope.

## Global constraints and review focus

- No built-in native fallback in production when a package/capture fails.
- Preserve exact native locator bytes; a display name never becomes a path.
- Backend-only capture/validation/disposal; cancellation before and after capture.
- Existing two-worker, source-memory and IPC limits stay unchanged.
- Saved archive revisions cannot silently accept changed content. Compatibility
  recognizes only exact v1 encodings with identical complete metadata payloads.
- New/native and persisted archive opens use the same capture callback.
- Source change after mount invalidates listing/read; missing providers fail closed.
- No nested-archive or generic write capability is implied by this change.
- Rustfmt100; source files <=100,000 bytes; artifacts under `_build`.

## Task 1: Native revision-compatible capture

Owned files: `crates/midnight-native-filesystem-host/src/installed.rs`, new focused
revision module and separate Windows/Linux adapters/tests. Existing APIs unchanged.

Interface: `InstalledProvider::capture_matching(path, expected: Option<&[u8]>,
cancelled: Cancel) -> Result<CapturedFile, Failure>`.

- [x] Tests for exact current revision, equivalent legacy prefix/payload, changed
  fields, truncated/extended/version-unknown tokens and cancellation/missing plugin.
- [x] Implement strict mapping: Windows v1 has 48 bytes of metadata after its
  prefix; Linux v1 has 56. No native I/O or plugin source dependency in comparison.
- [x] Run native-host tests/strict Clippy; document compatibility evidence.

## Task 2: Archive host, engine and app composition

Owned files: filesystem-host session/tests; engine archives/lib/tests;
app native_content/main/integration tests; migration docs/evidence.

Interfaces: `Mount::open_captured(package, format, locator, source, cancel)`;
engine `ArchiveCapture` callback accepts path, optional expected revision and
`Cancellation`, returning `CapturedFile`/`FsError`. Engine has
`configure_archive_capture()?.install(...)`. Missing composition returns explicit unavailable.

- [x] Add real-worker tests proving a pathless in-memory source opens/reads,
  retains its explicit Unicode locator, and rejects subsequent source changes.
- [x] Add pre-launch tests for cancellation, invalid locator and source limits.
- [x] Pass callback through new opens and saved restore, checking saved version
  before worker creation and storing the new validated token on successful restore.
- [x] Wire installed capture in app and verify actual native->archive->read and
  legacy saved-address restoration plus missing provider.
- [x] Run focused host/engine/app tests, scoped fmt/Clippy; independent review and fixes.
- [x] Build/deploy debug, smoke/startup checks and update qualification docs.
- [ ] Finish coherent reviewed native-host/cutover commits (generic hook is pushed).

## Task 3: Remove redundant native content wrapping

Final production-route measurements found a regression: 406-433 ms warm Open/List
versus 82-87 ms in the direct-capture host probe on the same 19.8 MB archive.
Source review identifies the native cache wrapping an already captured provider
inside a second CapturedFile, multiplying native validation calls for each chunk.
Do not deploy the regression without investigating/qualifying this overhead.

Owned files: native-host `content.rs`, `capture_cache.rs`, focused cache tests.
Use a crate-private raw content lease factory; retain one outer CapturedFile at
the public boundary. The LRU stores raw source leases rather than nesting another
CapturedFile. Keep exact length/revision checks on cache reopen and validation,
the 16-reader limit, and cleanup outside the mutex. Native SDK ReadAt validation
and public captured-source before/after validation must remain effective.

- [x] Reproduce redundant request count; test source replacement/mutation and
  bounded eviction/reopen behavior across the changed layering.
- [x] Implement, run native host tests/Clippy and review the new ownership path.
- [x] Rerun actual Windows/Linux app integration and Windows production-route
  timing; document measured results without claiming cold-start latency fixed.

Task 3 evidence: three resident chunks now issue 9 ValidateRead calls rather than
18, with 3 ReadAt calls unchanged. 38 native-host tests and strict Clippy pass.
Independent review found no introduced ownership/integrity issue. Final Windows
installed app tests passed all seven native archive/comparison/media scenarios;
the read-only user-archive probe measured 315/297/306 ms Open/List and ~1.3 ms relist.
The 17 ordinary app tests passed; 14 opt-in/native tests remain separate.
Linux baseline qualification completed: 52 tests passed, targeted strict Clippy
passed, and the audit confirmed 254 unchanged source inputs, matching staged
hashes and no remaining qualification workers. Evidence is preserved under
`_build/goal/archive-native-linux/baseline-raw-source/`.

## Next performance boundary: native SDK-validated reads

The raw-lease cache removes one redundant wrapper, but the outer CapturedFile
still issues three explicit native validations around a ReadAt whose SDK already
validates the pinned reader and pathname before/after reading. Independent design
review confirmed the following bounded host-only extension can avoid repeated OS
work without changing native plugin ABI, SDK or binaries:

- Add a provided checked-read hook to host `ContentSource`. Default behavior keeps
  generic source revision/length validation before and after reading.
- Keep cheap bounds, overflow, clamping and returned-length validation in the
  outer shared CapturedFile/Reader helper, independent of any hook override.
- The trusted native Content override compares the original captured metadata
  with its immutable lease metadata and delegates nonempty reads to the SDK's
  required validated ReadAt contract. Snapshot forwards original expected metadata
  through cache reopen; it must never replace it with newly opened metadata.
- EOF/beyond-EOF remains validated empty bytes, not an invalid native ReadAt;
  explicit revalidate always performs actual source validation.
- Generic sources that change during read still fail. Native conformance tests
  must model the SDK's before/after validation; a fake that returns success despite
  detectable in-read mutation violates that native protocol contract.

Implementation is in progress after the completed Linux baseline audit. Host RED
tests demonstrated the ignored checked-read hook and redundant EOF validations;
all three new regressions now pass with the shared host dispatch. The complete
ordinary Windows plugin-host suite passes 52 tests in its required protected
fixture directory (five separately gated native-plugin tests are ignored).
Finish the native overrides, run all affected suites and repeat actual
Windows/Linux app checks and the supplied-archive benchmark. Do not weaken live
source validation, silently switch back to direct native reads or claim current
300 ms production navigation is equivalent to the prior ~80 ms direct host probe.

Checked-read Windows follow-up is implemented and reviewed: 42 native-host unit
tests pass, resident chunks issue no extra ValidateRead, and all actual Windows
archive/native/app scenarios pass. Production Open/List is now 176/149/168 ms.
Targeted strict Clippy and debug build pass. Linux follow-up qualification passes
100 tests and strict Clippy, with a final unchanged-input/artifact audit under
`_build/goal/checked-read-linux/`. Windows deployment, runtime registration,
desktop shortcut and headless/native startup smoke checks pass; saved preferences
are unchanged. The independently buildable generic host hook and its contract/tests
were committed and pushed as `f80c47e`; native cutover changes remain a separate
coherent commit task.

The independently reconstructed native backend baseline is now committed and
pushed as `9df66c5`. Its API/SDK, Windows/Linux packages, host, trusted loader and
volume-information contract passed isolated Windows and Linux checks; see
`docs/verification/2026-11-09-native-backend-baseline.md`. Production composition,
build/CI integration and the remaining mixed feature changes still require their
own coherent commits. No unrelated working-tree changes were staged with it.

## Investigations already completed

The user-supplied 19,818,839-byte SDK 7z archive opened in 1.713 s initially and
81.6/86.5 ms subsequently with the optimized plugin. Listing/relisting was ~1 ms.
Temporary instrumentation then measured 133/82/78 ms opens, spawn+ready 74/27/23 ms;
the source/indexing phase was ~55-60 ms. The one-off cold delay did not recur and
is not claimed fixed. Instrumentation was restored byte-for-byte before this work.
Evidence: `_build/qa/archive-speed/user-archive-{before,trace}.log`.

This migration builds on the still-uncommitted broader native-host cutover. Commit
only a coherent reviewed dependency set, never a partial composition that cannot
build from its parent. No user approval is required for this already requested work.
