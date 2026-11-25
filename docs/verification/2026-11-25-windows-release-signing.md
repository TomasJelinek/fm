# Windows ZIP signing and per-user MSI — 25 November 2026

Implemented in the shared checkout without staging, committing, publishing,
creating resources or changing the application's license. The implementation is
described in the [signing guide](../developer/WINDOWS_RELEASE_SIGNING.md).
This receipt qualifies local packaging behavior, not public signing or installation.

## Local validation

Python commands used the bundled Python 3.12 executable at
`C:/Program Files/WindowsApps/OpenAI.CodexPrimaryRuntime.v26-1123-915-0_26.1123.915.0_x64__3k8sg7r9htsxt/dependencies/python/python.exe`.
The commands below use `python` to denote that exact executable.

```powershell
python -B -c 'import sys, unittest; sys.path.insert(0, "scripts/tests"); names=["test_package_release", "test_worker_packages", "test_runtime_packages", "test_image_viewer_package", "test_installed_video_package", "test_windows_release_signing", "test_windows_msi", "test_github_windows_release"]; result=unittest.TextTestRunner(verbosity=1).run(unittest.defaultTestLoader.loadTestsFromNames(names)); sys.exit(not result.wasSuccessful())'
```

Result: **80 run, 71 passed, nine skipped**, zero failures/errors, 29.814 seconds.
Skips concern OS/privilege-specific existing packaging boundaries and a Windows
file-symlink privilege; the directory-junction refusal checks executed. Signing
finalization tests use explicitly simulated verification receipts to exercise
hash refresh and mutation detection. They do not establish certificate trust.
An actual unsigned PE assembly separately exercises native preflight acceptance
and release-signature refusal. GitHub tests substitute the network boundary and
exercise draft/asset/tag races, checksum preservation and uploaded digest refusal.

After MSI review changes:

```powershell
python -B -m unittest discover -s scripts/tests -p test_windows_msi.py -v
./scripts/tests/test-windows-msi.ps1 -Python '<bundled Python executable above>'
```

Results: **seven Python tests run, six passed, one file-symlink privilege skip**;
native MSI integration **PASS**. Final root-verified fixture artifacts:
`_build/tests/windows-msi-3ee932806c0b41b7bb7bf7c29b40dfa0/`.

The native test creates a real MSI through Windows Installer COM and makecab,
reads its imported tables, expands the CAB independently and compares every
payload SHA-256. It checks native Windows EXE/DLL version and language metadata,
executes `FindRelatedProducts`, `AppSearch`, `ValidateProductID`, `CostInitialize`,
`FileCost`, `CostFinalize` and `LaunchConditions` in a read-only package session,
and validates the active profile's default folder, a Unicode override and refusal
of `ALLUSERS=1`. It checks rejection of that unsigned MSI without generating
signature/checksum receipts. It never executes the install sequence.

PowerShell AST parsing passed for `verify-windows-release.ps1`,
`build-windows-msi.ps1`, `github-windows-release.ps1` and the native MSI test.
Scoped `git diff --check` passed. New implementation source uses LF and is below
the 100,000-byte source-file limit. Workflow YAML was reviewed manually; no local
YAML parser/actionlint or live GitHub workflow run was available.

## Independent review and corrections

- Bound actual manifest-selected plugin entry DLLs to the static signing policy.
  A self-consistent redirected `other.dll` package reproduced the gap before the
  correction; its regression then passed with the release signing group.
- Read version/language from frozen cabinet inputs into the MSI File table and
  verify native readback. The versioned fixture reproduced missing metadata before
  the fix. Standard Windows Installer replacement/version rules still apply.
- Removed the unexpanded `ARPINSTALLLOCATION` Property-table literal. The correctly
  formatted private registry installation-location entry remains for upgrades.
- Added the required empty Signature table. Native `AppSearch` returned failure
  before this correction; it succeeds in the final native pre-install test.
- Scoped recovery-artifact globs so MSI authoring copies/cabinets are not uploaded
  alongside the final distributables.

## Remaining acceptance

Azure identity eligibility/validation, resource setup, OIDC trust, GitHub protected
environment configuration, real signing/timestamping and downloaded-release
verification remain external setup/acceptance. No paid resource was created.
No actual install, uninstall, upgrade, repair, interactive installer UI, full app
smoke run or SmartScreen outcome was tested. MSI supports a single embedded CAB
with a 2 GB input cap, numeric MSI versions and Windows-1252 payload filenames
(unsupported names fail explicitly); runtime installation paths support Unicode.
No Cargo/application behavior changed, and no broad application suite was run.
