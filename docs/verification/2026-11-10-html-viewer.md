# Embedded Chromium HTML viewer verification

The optional independent `midnight-viewer-html` provider renders inside the normal
Slint viewer window. It uses the separate public web-viewer API/SDK and does not
link Chromium into `me.exe`. Historical Windows x64 evidence is retained below.
The later [Linux qualification](2026-11-10-html-linux.md) covers Ubuntu 24.04 WSLg
XWayland host/Slint rendering with sandbox and delegated aggregate limits enabled.
Native Chromium Wayland, physical IME and macOS remain outside that qualification.

## Verified boundaries

- Public API/SDK: 12 tests, formatting, strict Clippy and a separately compiled
  consumer of the immutable local registry packages.
- Viewer engine: 104 passed, four pre-existing opt-in tests ignored. This includes
  native/archive resource scopes, source mutation, stale contexts, resize input
  releases, custom extension routing and existing text/image/Markdown geometry.
- Web host: five tests, including traversal/encoded-path denial, capability
  revocation, latest-frame ownership and bounded control queues. The State queue
  overflow regression failed before the capacity check was corrected.
- Viewer UI: 41 unit tests passed, including remapped Find/Copy/next-file actions,
  unassigned Space scrolling, AltGr text, genuine Enter character events and
  Find focus/key-release recovery.
  Existing viewer keyboard integration tests also passed earlier in this task.
- Independent CEF provider: eight tests and real native qualification. Canceled
  asset resolution and oversized clipboard selection regressions failed before
  their respective fixes. Overflow emits an error without clipboard text;
  a subsequent valid 40 KiB ASCII selection is copied intact.
- Linux process supervisor: the original 34-test pass with an ignored native test
  was followed by 35 ordinary tests and both actual delegated native tests passing.
  Strict Clippy and formatting pass. The native tests exercise independent process
  groups, full-tree termination and private temporary-directory lifetime with a long
  profile; invalid delegation still prevents execution instead of launching unbounded.
- Inventory generation, regeneration check and nine inventory tests passed:
  530 package/version/source records, 495 distinct crate names, 63 direct records.
  Source-size checking passed for 1,586 handwritten files, all below 100,000 bytes.

## Native Windows qualification

The installed provider uses CEF
`156.0.26+g8ea0c73+chromium-156.0.8078.88` and Rust bindings
`156.4.0+156.0.26`. Chromium's sandbox remains enabled. Native tests run outside
the additional Codex process sandbox, which otherwise prevents CEF subprocess
DLL startup; this does not disable the provider's own sandbox or host Job limits.

The independent test exercises real HTML and inline/nested CSS rendering,
denied default JavaScript and a denied loopback HTTP image request, Find/Find
Next, native select-popup composition and hiding, editable-field Space and Copy,
noneditable Space cycling, layout zoom, fractional DPI and clean process exit.
A separate real textarea test sends Enter and Shift+Enter through the protocol
and verifies exact copied text `first\nsecond\nthird`; it passed on the same
frozen provider DLL, with no provider change required.

The actual host probe additionally switches offline -> online with scripts ->
offline using fresh document generations. It verifies the script runs only in
the enabled context, checks rendered pixels and fractional-DPI surface geometry,
and closes the supervised browser tree. One installed run measured 1,671 ms to
first HTML pixels and 281 ms to completed close. These are individual debug-run
observations, not release performance guarantees.

The `native_html_viewer` example uses the same `ViewerWindows` entry point as F3.
Its actual Slint window rendered Unicode, CSS and tables, found and highlighted
the query, resized at 225% display DPI, zoomed to 125%, and closed on Escape.
Initial and resized snapshots were visually inspected. An installed run found
the query in 37 ms. The normal `_build/debug/me.exe --smoke-test` also resolved
its registered runtime and listed both fixture panels and five drives.

The provider's own crate uses `opt-level = 2` in the dev profile; dependencies and
the application's dev profile are unchanged. A 3,072,000-pixel conversion,
retention and frame-copy benchmark (20 samples) improved from 62.497 ms to
6.421 ms median. This is a CPU-path microbenchmark, not a browser frame-rate claim.
The combined scroll/resize/zoom probe's last paint changed from 507.6 to 416.5 ms;
that measure includes scroll animation and must not be presented as input latency.

The final installed run recorded 1,532 ms to the first Slint HTML frame, 36.9 ms
for Find and 343.5 ms to the first accepted resized surface after the combined
zoom/scroll/resize action. The final backend-only host probe recorded 1,006 ms to
first content and 314 ms to close. These debug measurements do not establish
mobile-style frame rates or replace release scroll/drag benchmarking.

The Windows executable for that original qualification is `_build/debug/me.exe`, SHA-256
`7771cfcebc6981a7ea069dea27e0ae9a37ca547752be446f0b50f039e4196f78`.
Its matching registered private installation is
`C:\Users\tjelinek\MidnightExplorer-20261110-html`; the HTML package is under
`viewers/html`. Provider DLL SHA-256 is
`347b99e554c5e111d9414ce14561c6f3b445e12284cf09e15d0a9c43f1fec3c2`.
Final GUI and normal debug-app smoke checks passed with this registration, and
no test Chromium processes remained. Two superseded test installations were
removed, reclaiming about 1.19 GB of generated files.

The subsequent archive-thumbnail integration rebuilt the main executable and
registered its new hash against the same protected runtime; the HTML provider
package is unchanged. That build's evidence is in the
[archive-thumbnail report](2026-11-10-archive-thumbnails.md). A subsequent
[tree-search integration report](2026-11-10-tree-prefix-search.md) records the
then-current executable hash, successful registered startup and two fresh native HTML
host runs. The latest [tree viewer integration build](2026-11-10-tree-viewers.md)
records the current main executable and startup; the HTML provider remains unchanged.
The actual Slint-window results above belong to the earlier executable
and probe recorded here; the provider DLL is unchanged.

## Reproduction and artifacts

### Historical integrated-build rechecks

The HTML request was rechecked against the current installed debug application,
SHA-256 `cfb3bed652e5aa91620e4383d322333272cfe05210a4e0d2462e405636579340`.
The installation and workspace executable hashes match; the provider DLL retains
the qualified hash above. Fresh host tests (5) and viewer UI tests (41) passed.
The installed native host probe passed Find, offline script denial, explicit
online/script activation, return to offline, fractional DPI and process shutdown.
The actual Slint viewer probe passed rendering, Find, scrolling, resizing, 125%
zoom and Escape; its initial and resized screenshots were visually inspected.
The normal workspace executable's startup smoke passed with five drives and both
panels populated. This run required no production code or dependency changes.

The Slint probe measured 1,917 ms to first content, 32.7 ms for Find, and 333.5 ms
to the first accepted resized surface after the combined zoom/scroll/resize
action. These are single debug-run observations, not frame-rate guarantees.
Fresh logs, screenshots and executable hashes are under
`_build/qa/html-viewer-recheck/run-0d6afd02899b46638162e5b1fcf5ce37/`.
The temporary installed UI probe was removed after verification. This historical
Windows recheck did not itself change Linux or macOS status; the later Linux
qualification is recorded separately below.

### Reproduction

The subsequent F5 tree-Copy build (`A1329EA01131807EDB2A58ECFC235CFE04A270B670F2DD774A51B35A436938D3`)
was also checked against the identical protected installation. The unchanged
Chromium provider passed the real native host policy/Find/shutdown probe, the
Slint render/Find/resize/zoom/Escape probe, and application startup. Initial and
resized screenshots were visually inspected. Evidence is under
`_build/qa/html-viewer-recheck/run-342f117f4a314919b61e0635cda0a833/`.
This was a concurrent debug-build run; its timings are not a release latency
qualification. That historical recheck did not itself qualify Linux or macOS.

The subsequent [native tree Move build](2026-11-10-tree-move.md) records the newer
main executable and matching protected-runtime registration. Its startup and
current-root viewer capture tests pass; the Chromium provider package is
unchanged. The native browser measurements above retain their original run scope.

Build/package instructions and the pinned native source/license coordinates are
in the [provider README](../../plugins/midnight-viewer-html/README.md).
Run API tests in `web-viewer-api`; provider tests in its independent workspace.
Run the affected application suites with the project's normal offline SDK config:

```text
cargo test -p midnight-web-viewer-host -p midnight-viewer-engine -p midnight-viewer-ui --lib
cargo clippy -p midnight-web-viewer-host -p midnight-viewer-engine -p midnight-viewer-ui --all-targets --no-deps -- -D warnings
cargo build -p midnight-web-viewer-host --example probe
cargo build -p midnight-viewer-ui --example native_html_viewer
```

The host probe accepts a canonical trusted installed executable, explicit HTML
fixture and report directory. On Windows, use the extended `\\?\C:\...\me.exe`
path returned by the installation resolver; an ordinary noncanonical spelling is
rejected by the existing package trust boundary. Place the UI probe beside the
private installation's executable and pass that fixture and report directory.
Generated evidence stays under
`_build/qa/html-viewer/`: host/UI logs and screenshots, application smoke log and
deployment registration log. Windows provider evidence is under
`_build/html-viewer/provider-target/qa-*.{log,json}`. Its native archive digests,
full notices and all 534 package-file hashes retain their historical run scope.
The later Linux package/evidence locations are recorded in the Linux report.

## Remaining qualification

Linux requires Linux 5.14+, cgroup v2, an existing user delegation with memory and
pids controllers, and a working Chromium sandbox. The earlier absence-of-delegation
statement is superseded by the actual user-manager tests and protected installed
host/Slint run in the [Linux report](2026-11-10-html-linux.md). The exact pinned
Linux archive, XWayland rendering, policy transitions, real broker crash and complete
process/cgroup/profile/temporary cleanup are now qualified in that environment.
Native Chromium Wayland, physical IME, physical HiDPI hardware and long-running
crash/termination stress remain additional qualification work. macOS is not
implemented. No unsandboxed fallback exists. Public redistribution must preserve
the bundled notices and corresponding-source obligations documented by the plugin.

The Linux follow-up also corrected the host example's measurement scope: it now
records the first accepted colored frame before buffered bitmap output. Historical
host-probe first-content values above were sampled after bitmap writing and include
artifact I/O. They remain historical run observations, not pure startup latency;
the separately measured Slint timings retain their original scope.
