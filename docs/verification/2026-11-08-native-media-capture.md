# Native content capture for comparison and previews

Verified on Windows x64 on 8 November 2026. This extends the native filesystem
cutover to comparison and thumbnails; it does not complete generic VFS migration.

## Production route

`midnight-explorer/src/native_content.rs` creates one deferred capture factory
from the app's shared `InstalledProvider`. F3, both comparator inputs (including
manually chosen paths), panel thumbnails and configured command-icon thumbnails
use this authority. Capture and resource disposal stay on backend workers. An
absent/rejected native provider never triggers a built-in native-file fallback.

The thumbnail host adapts `CapturedFile` to its existing decoder `Source` without
opening a filename. It preserves length, content revision, read validation and
the caller's conservative storage hint. The native host retains its bounded
reader cache; evicted readers can reopen only the originally captured revision.
The independent image/video packages and public plugin protocols are unchanged.

Capture receives the active cancellation flag. A cancelled capture is not pinned,
and cancellation is checked before a disk-cache hit can be returned. A stopped
service rejects new views. The thumbnail timer waits while the main window is
temporarily hidden during startup; only destruction/shutdown stops the service.

## Evidence

- Comparator: 30 tests passed, including injected in-memory sources under absent
  Unicode paths, explicit Compare submissions, provider rejection and cancellation
  on either input. Session capacity remains occupied until a blocked capture exits.
- Thumbnail host: 22 tests passed (10 unit, 8 cache, 4 scheduling). Seven opt-in
  deployment/performance/worker tests remain ignored in this suite. The real native
  media integration checks below were run separately.
- Regression tests first reproduced cached-frame delivery after capture
  cancellation, accepting views after stop, and stopping preview initialization
  while the startup window remained hidden. All pass with the fixes.
- The real software-window/timer startup regression passed. It holds the window
  hidden for 150 ms, verifies the service is still accepting work, then shows it.
- Four installed-provider app tests passed against the final build in 11.56 s:
  actual text/image comparison; actual BMP and animated AVI thumbnails; missing
  provider/cancellation without fallback; and 40 Unicode sources exceeding the
  16-reader cache, with source modification rejected after eviction.
- Targeted strict Clippy passed for comparator, thumbnail host and application.
  The existing `midnight-engine` dependency reports 39 warnings from unfinished
  provider/tree wiring; this is not a clean full-workspace qualification.
- Final debug application build passed. Headless startup listed three volumes and
  46 entries in each owned-fixture panel. Native UI startup/snapshot passed without
  preview errors. The snapshot was visually inspected. Real preferences retained
  their pre-test SHA-256; smoke modes use in-memory settings.

The first sandboxed startup checks could not read protected Windows plugin-path
permissions and failed closed with access denied. Both checks passed outside that
filesystem sandbox; no trust check or ACL was weakened to make them pass.

## Reproduction and artifacts

Build/test output is under `_build/qa/native-media/`, including
`thumbnail-final-tests.log`, `hidden-startup-{red,green}.log`, `stopped-red.log`,
`native-app-capture-final.log`, `clippy-final.log`, `app-final-build.log`,
`smoke.{stdout,stderr}.log`, `ui-smoke.{stdout,stderr}.log` and `native-startup.png`.

The app tests are opt-in in `midnight-explorer/src/native_content_tests.rs`.
Run the test executable inside a protected installation containing the independent
native, image-viewer and image/video-thumbnail packages. Set
`MIDNIGHT_NATIVE_TEST_INSTALLATION` to that installation and
`MIDNIGHT_NATIVE_MEDIA_TEST_FILES` to owned fixtures, then run:

```text
native-media-tests.exe native_content::tests --ignored --test-threads=1 --nocapture
```

Fixtures include forty `capture-{index}-文档.bin` files containing each u32 index
in little-endian form, `left 文档.txt`, `right.txt`, `picture 文档.bmp` and
`video.avi`. The QA-only Rust fixture generator uses the independent video plugin's
existing authored AVI test fixture. Regenerate before rerunning: the reader-cache
test deliberately changes the first owned binary fixture to test invalidation.

Final app SHA-256:
`9BA584CFE4C3167126F69E84BA0E14CC905DD8F28385FF4296BC6079D45E6F61`.
Private deployment:
`C:\Users\tjelinek\MidnightExplorer-20261108-224503-eb237483`.
The build's hash-specific runtime registration and redirected-desktop shortcut
were updated by `scripts/deploy-private.ps1`.

Independent code review found the hidden-startup lifecycle issue and confirmed
its fix and regression evidence. Linux/macOS execution, generic archive/remote
thumbnail sources, watches, mutations, transfer and Find migration are not covered
by these checks. The media slice depends on the broader native-host cutover still
under review; it must be committed with that coherent dependency set. The separate
archive performance change is already committed/pushed as `d4c912b`.
