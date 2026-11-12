# Shared-build startup and protected runtime — 7 November 2026

The reported `_build/release/me.exe` reproduced the **Previews unavailable**
startup error with isolated UI-smoke settings. Its `thumbnailers/` directory and
ancestors inherit modification/replacement permissions for other accounts from
the shared D: tree. This is an actual unsafe plugin installation, not a false
positive in the native ACL mask. Evidence: `_build/qa/runtime-resolution/before.err`
and `before.png`; the original release exited 1 in UI-smoke mode.

The fix selects a separately deployed private runtime for that exact host hash.
It neither loads shared plugin code nor changes permissions on the checkout,
its ancestors or D:. All installed plugin consumers use the same backend resolver;
explicit-root test APIs and built-in text comparison remain available. See the
[developer contract](../developer/RUNTIME_DISCOVERY.md).

## Regression evidence

- The new matching-build regression failed against the original behavior before
  the resolver was implemented. All 14 Windows resolution regressions then passed:
  trusted portable precedence, exact build matching, stale source/target rejection,
  strict and oversized records, unsafe owners/ACLs/ancestors, registration/target
  junctions, raw UTF-16 paths and prevention of concurrent write/rename.
- Full plugin-host Windows unit suite: 39 passed, 5 existing native-plugin fixture
  tests ignored. Strict Clippy and formatting checks passed for that package.
- Deployment registration also has a recorded red run. Its green tests cover exact
  digest/native path, atomic record replacement, protected ACLs, unsafe registry and
  junction refusal. The existing private-deployment suite passed copy/readback,
  package completeness, ancestor/reparse and existing-destination checks.
  Registration also passed under Windows PowerShell 5.1, used by `build.bat`.
- Independent review found no blocking security/correctness issue. The test's
  default fixture location was corrected to the user's `_build` directory.
- Final deployment preflight also refused the original AppData registration
  location: both `Local` and the settings directory grant another capability SID
  full control. No deployment or registration was created by that failed attempt.
  Registration now uses a separate protected namespace under
  `%USERPROFILE%/.midnight-explorer/plugin-runtimes`; the private profile ancestors
  pass the same policy. A script regression verifies default registration succeeds
  there while an unsafe AppData fixture and its ACL remain untouched.
- Windows affected suites passed: engine 199, comparator 9, thumbnail host 5,
  viewer engine 23. The two installed-viewer integration tests were exercised in
  the separate native acceptance run; one unrelated performance test remains
  opt-in. Strict Clippy passed for all affected backend targets.
- Linux affected suites passed: engine 199, comparator 9, plugin host 26,
  thumbnail host 4, viewer engine 23. Eight fixture/performance tests remain
  explicitly opt-in in that run. Native Windows acceptance is not claimed as
  Linux GUI coverage.
  Strict Clippy also passed on Linux for every affected backend target.
- The dependency inventory was regenerated from current native Windows/Linux
  application graphs and checked: 486 package versions, 454 package names, 56
  direct dependencies. SHA-256 reuses pinned `sha2` 0.10.9 (MIT OR Apache-2.0);
  no additional third-party package or version was introduced.

## Final release acceptance

The final optimized Windows release built successfully in 3m 42s:

- Executable: `D:\experiments\dev\FM\_build\release\me.exe`, 38,997,504 bytes.
- SHA-256: `559CE0249758E6E2E2D961B0EFA4A2521B2C8AB7D083377FAA0539098E45D1FB`.
- Matching protected runtime:
  `C:\Users\tjelinek\MidnightExplorer-20261107-120804-5c2b92b9\me.exe`.
- All viewer, filesystem and thumbnail packages were copied with SHA-256 readback,
  and the matching record was published under the protected profile namespace.

The **actual D: release** then ran with `--ui-smoke-test --snapshot ...`, using
the application's default Windows GPU backend. Exit status was 0 and stderr was
empty. The screenshot was visually inspected: both panels, command line and
bottom toolbar rendered, with no Previews error. Evidence lives under
`_build/qa/runtime-resolution/`: `release-deployment.json`,
`release-acceptance.json`, `release-direct.out`, `release-direct.err` and the
`release-direct-6e782379fd7c440a9404ed1d5b531b46.png` capture.

Real preferences were never loaded/saved by this smoke mode. Their hash matched
the pre-task backup and the post-test file exactly. The separate native viewer
probe also uses no application preferences; saved tabs were not changed.

The optional desktop-shortcut update was refused because the existing OneDrive
Desktop is a reparse point. Deployment and registration had already completed;
the existing shortcut was untouched. Acceptance launched the exact requested D:
executable directly and does not claim that shortcut was updated.

Native viewer acceptance uses actual independently installed packages and an
in-memory host presenter, without loading application preferences. Its evidence
and controls are documented [separately](2026-11-07-viewer-runtime-acceptance.md).

## Reproduction commands

```powershell
$env:HOST_TEST_INSTALL_ROOT = Join-Path $env:USERPROFILE '_build/runtime-resolution-host-tests'
$env:MIDNIGHT_NATIVE_TEST_ROOT = Join-Path $env:USERPROFILE '_build/runtime-resolution-integration'
New-Item -ItemType Directory -Force $env:HOST_TEST_INSTALL_ROOT | Out-Null
./scripts/cargo.ps1 test --locked --offline -p midnight-plugin-host --lib
./scripts/tests/test-runtime-registration.ps1
./scripts/tests/test-private-deployment.ps1
./scripts/cargo.ps1 test --locked --offline -p midnight-engine -p midnight-viewer-engine -p midnight-thumbnail-host -p midnight-compare-engine --lib
```

Native cache fixtures must use a private directory: Windows tests need the explicit
fixture environment above; Linux cache tests need a native filesystem supporting
owned mode 0700 (for example `/tmp/fm-interaction/_build/runtime-resolution`).
The first combined run exposed these test-configuration requirements; no production
permission or cache check was weakened to accommodate the shared checkout.
