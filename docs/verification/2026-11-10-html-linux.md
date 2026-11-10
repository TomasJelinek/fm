# Linux embedded Chromium qualification

The exact Linux x64 provider is qualified on Ubuntu 24.04, WSL2 kernel
6.6.87.2-microsoft-standard-WSL2 and WSLg XWayland. The actual Slint component uses
software presentation. Tests use a protected native installation beneath
`/home/tjelinek/_build/fm-html-linux-install`, including the installed native
filesystem provider. Existing [Windows evidence](2026-11-10-html-viewer.md) and
its provider remain unchanged. Full Linux main-application packaging is separate
from this installed host/component qualification.

## Pinned package and build boundary

CEF is `156.0.26+g8ea0c73+chromium-156.0.8078.88`; Rust bindings are
`156.4.0+156.0.26`, defaults disabled and sandbox enabled. The official Linux64
minimal archive is 333351579 bytes, published SHA-1
`cacfbb7d7261fea682fb67c108de7e4347519ff7`, local SHA-256
`600ac0b8f37cb953f884e7c89998a6c200d24959aa74820ad431b61d5ebcf360`.
Actual archive metadata was obtained before download and checked before extraction.

The Linux helper validates all 929 flattened SDK inputs before Cargo, rejects
unexpected entries, SDK symlink entries and CEF environment overrides, and preserves the exact
canonical UTF-8 SDK path through a NUL-delimited handoff. Missing/wrong metadata,
corrupt/incomplete files and relative Unicode paths ending in LF have regression
coverage. This prevents cef-dll-sys from interpreting an invalid path as permission
to download a fallback. The independent workspace still consumes only the public
registry API; no private host dependency was added.

The actual launcher has literal `$ORIGIN` DT_RPATH. Cleared-environment `ldd`
resolves CEF and deliberately supplied NSS/NSPR/ALSA from package siblings, with
remaining OS dependencies resolved. Launcher SHA-256 is
`4f96a8ebc1193c567e60cc16eb5b0f9c47d9f3be51314c95820da1aa7e007ff1`.
All 503 indexed package files were rehashed; native LICENSE/CREDITS, 125 resolved
third-party Rust notice directories, target dependency metadata and exact native
source/build archives are preserved. [Inventory](../THIRD_PARTY.md) and
[plugin instructions](../../plugins/midnight-viewer-html/README.md) retain the
selected free license branches and redistribution duties.

## Native boundaries and measured results

- Independent provider: eight Rust tests, six packaging safety tests, strict
  all-target Clippy and formatting passed. Both opt-in native CEF tests were run
  explicitly and passed. They cover actual HTML/CSS streaming, offline script and
  loopback HTTP denial, Find/Next, select popups, selection/Copy, editable Space,
  Enter/Shift+Enter, zoom and fractional viewport rounding.
- Worker platform: 35 ordinary tests and two actual delegated native tests passed,
  with strict all-target Clippy. The tree test includes independent process groups.
  An initial five-test store failure came from DrvFS permissions; the existing
  native-test-root option was used on native storage, without relaxing checks.
- The actual host switches offline to online/scripts and back through fresh
  contexts, with Find results proving script execution only in the enabled context.
  The final Cargo-built probe measured 471 ms to accepted colored content before
  artifact I/O and 41 ms to close; an earlier corrected run measured 1404/36 ms.
  These are individual debug runs with different cache state, not performance
  guarantees. Old 111780/117439 ms Linux measurements included unbuffered BMP I/O.
- The actual Slint viewer uses installed native capture and an explicitly pinned
  original selected-parent capability. Both screenshots were visually inspected:
  external CSS produces the expected blue body/white text from a 350-byte Unicode
  native source path. Initial frame was 1.488 s, Find two matches 19.6 ms, and first
  accepted resized frame 103 ms. Zoom to 125%, scroll, resize and Escape passed.
  The 2.01 s settled gate includes a deliberate two-second fixture wait.
- Kernel observations show real nested CEF descendants with NoNewPrivs=1,
  Seccomp=2 and PID namespaces. The cgroup retains 2 GiB memory, no swap and 256
  tasks. Normal close leaves no owned processes, cgroup, short temporary directory
  or long profile. An actual SIGKILL of the identified owned broker separately
  verifies crash reporting and complete process/cgroup/short-temporary cleanup.

## Fixed lifecycle defect and evidence

The original Linux host used its full private profile as TMPDIR. Real CEF failed
with `process_singleton_posix.cc:315`, an overlong AF_UNIX socket path and SIGTRAP.
A separate Linux guard now exclusively creates and pins a short private socket
directory beneath a validated user runtime, with a root-owned sticky `/tmp`
fallback. Protected ancestry and visible/pinned inode identity are checked.
Ownership lasts through whole-tree reaping; failed reaping retains state. Source
and profile paths are never truncated. No sandbox or cgroup policy was weakened.

Raw evidence is retained under `_build/qa/html-linux`: `provider-final-checks.log`,
`package-final-tests.log`, worker test/lint logs, `host.log`, `supervised-tree.json`,
`host-crash.log`, `crash-cleanup.json`, `native-runtime.log`,
`renderer-sandbox.txt`, `ui.log`, `ui-supervised-tree.json` and `ui-report/*.png`.
`review.md` records independent source/security and artifact review with no blocking
finding. Scoped before-images remain in `baseline/`. Windows evidence was not rerun.

## Explicit limits

Native Chromium Wayland, physical IME and physical HiDPI hardware remain unproven;
synthetic protocol input/scaling and XWayland do not establish them. The minimal
test OS lacks CJK fonts and displays glyph boxes; Czech text and Unicode path
identities are correct. The X11 frontend required private copies of the standard
libxkbcommon-x11/libxcb-xkb prerequisites, with actual versions, hashes and notices
in `ui-prerequisites.json`; these were not added to the HTML provider package.
Long-running crash stress and other desktop/system policies need separate runs.
