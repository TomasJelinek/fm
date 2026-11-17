# Installed plugin runtime resolution

Application backend services call
`midnight_plugin_host::installed_executable() -> Result<PathBuf, Error>` before
discovering viewers, image/PDF providers, thumbnailers or native, network and
archive filesystems, and before locating the directory authorization helper.
This is an internal host API, not a public plugin ABI change. Hashing, registration
reads and filesystem checks run on workers. UI code receives results through its
existing messages. Explicit-root host APIs remain available for integration tests.

On Windows, a protected executable and parent directory take precedence. Adjacent
packages in a manually assembled installation may be absent; their absence never triggers fallback to another
installation. Linux and other platform implementations retain executable-relative
discovery and their existing loader support/limitations.

For a shared Windows executable, `MIDNIGHT_VERIFY_RUNTIME_SHA256` selects
own-EXE verification. Absent or `0` is **OFF**; `1` enables the legacy strict
hash/snapshot route. Empty, other or non-Unicode values return an error. This
adds no GUI/preferences setting; the result or error is cached until restart.

OFF hashes neither the launch EXE nor the selected runtime EXE. A protected
per-launch-path pointer at
`%USERPROFILE%/.midnight-explorer/plugin-runtimes/launchers/<path-key>.toml`
selects the existing schema-1 registration. Native normalized raw UTF-16 path,
length and LastWriteTime provide identity/staleness hints; ChangeTime is excluded.
Rust and PowerShell use `GetFinalPathNameByHandleW` for native normalization.
The path-key SHA names a short path; it hashes path material, not EXE contents.
OFF makes no EXE byte-equality claim. Missing, invalid or stale selection does
not fall back to another registration or choose the newest runtime.

ON hashes the exact running EXE and follows the legacy protected
`%USERPROFILE%/.midnight-explorer/plugin-runtimes/<sha256>.toml` registration.
Both routes retain the existing registration schema:

```toml
schema_version = 1
sha256 = "64 lowercase hexadecimal digits"
executable_utf16 = [67, 58, 92]
```

The example array illustrates encoding only. Real records contain the full
absolute native path of a deployed private `me.exe`, preserving UTF-16 including
unpaired surrogate units. Unknown fields, NULs, parent traversal, unsupported
namespaces and schema mismatches are rejected. ON additionally requires the
private EXE to have the same SHA-256 as the running executable. That comparison
matches builds; it is not publisher authentication and does not secure a shared
EXE that the user has already run. OFF retains ACL/owner, reparse-point,
regular-file, manifest, platform, package and ABI guards without claiming equal EXE bytes.

The registration directory, record, target and ancestors retain the existing
owner/DACL policy. Original path components are checked for reparse points before
canonicalization can conceal a junction. Content-read and registration handles
are opened without following the final reparse point, with read sharing only,
excluding writes/renames during the read. OFF source metadata handles do not
promise data-write exclusion. Deployment retains a separate `0x81` read lease
through copying and both publications. Guarded record reads retain length/native-revision checks; OFF EXE
staleness hints use only length/LastWriteTime. Records have a 1 MiB budget; ON
executable hashing has a 512 MiB budget and uses a 64 KiB buffer.
These are resource budgets, not product path-length restrictions.

Registration is separate from the preferences/cache tree: an existing AppData
directory may grant other identities write access and cannot then safely contain
plugin authority. Deployment creates its own protected directories below the
user profile and still refuses unsafe profile ancestors or pre-existing records.
It never repairs permissions on AppData or on the settings directory.

The location or error is cached until restart. Every loader still validates the
actual package at discovery/load. A standalone shared EXE with no plugin trees
and no matching record can run with optional capabilities absent; packages added
later still undergo loader security checks. An unresolved optional image provider
does not disable built-in text comparison.

## Deployment and build workflow

`build.bat [debug|release]` builds the host and its authorization helper, the native
and network filesystem providers, all six independent viewers, both thumbnailers and
the archive worker, then calls `scripts/deploy-build.ps1`.
It requires 64-bit PowerShell 7 and Python 3.12 or newer, checks both before
building, and honors `CARGO_TARGET_DIR`. A missing authorization helper, image/video
thumbnail worker, filesystem/viewer module, runtime library or corresponding
source/notices fails the build before deployment. Linux `build.sh` includes the
same module set in its native and Docker chains too. Independent modules remain
separate public-SDK consumers; their inclusion in normal builds is mandatory.
Direct Cargo builds require explicit deployment if their
output is not trusted:

```powershell
pwsh -NoProfile -File scripts/deploy-private.ps1 -SourceDirectory _build/release -RegisterRuntime
```

Add `-IncludeFilesystems` and `-IncludeThumbnailers` for a complete normal
installation, and `-CreateDesktopShortcut` if wanted. The lower-level deployment
tool retains selective package options for specialist fixtures. It creates a new runtime
installation under the user profile, verifies copied bytes with SHA-256 readback,
and then publishes the legacy record before the protected per-launch-path pointer,
using exclusive temporary files and atomic replacement. Existing hash-only
registrations need one deployment refresh for default-OFF selection.
It refuses unsafe pre-existing directories/records and never repairs their ACLs.
It changes permissions only on new deployment/registration directories and deployed
files. Installations made with `deploy-private.ps1` without `-ReplaceBuildDeployment`
are retained. `build.bat` (through `deploy-build.ps1`, which passes
`-ReplaceBuildDeployment` to `deploy-private.ps1`) marks its installation and, after
registering it, removes earlier deployments marked for the same build output that no
launcher pointer selects and no running process uses. Build deployments made before
this marker existed are never removed automatically. Build deployments of one build
output run one at a time per logon session. Compiler/test output stays under `_build`.
Replace compatible plugin packages with the app closed, then restart; there is
no per-plugin content pin. The EXE hash in an ON registration selects a runtime;
loaders still validate actual packages. Deployment never launches the app or
changes user preferences.

The deployment does not copy `midnight-elevated-listing.exe`, and its output says
that **Retry with authorization** needs an administrator-managed installation. The
authorization helper never runs from this per-user runtime: it runs as an
administrator, so the runtime's user must not be able to replace it. Authorized
listing needs an installation that only administrators can change, for example a
folder below `C:\Program Files` ([installation steps](../../README.md#build-and-run));
see the [directory authorization helper](../../SECURITY.md#directory-authorization-helper).
From a per-user runtime, Retry with authorization is refused with an explanation of
this requirement; plugin loading is unaffected. A runtime from an earlier deployment
that still copied the helper keeps that copy. Builds with this protected-location check
refuse it, because the runtime's user can change it; an older `me.exe` in such a
runtime may still use it until the runtime is removed.

Tests use `-RegistrationDirectory` with a protected disposable fixture. That
override requires `-RegisterRuntime`; no test needs to edit the actual profile.
With `-ReplaceBuildDeployment`, `deploy-private.ps1` refuses `-Destination` and always
deploys directly below the profile, so the retirement test points `USERPROFILE` at a
disposable directory.
See [the resolution design](../superpowers/specs/2026-11-07-trusted-runtime-resolution.md)
and [verification](../verification/2026-11-07-trusted-runtime-resolution.md).

Windows qualification of the default-OFF selection (16–17 November 2026 checkpoint
in [active work](../ACTIVE_WORK.md)): Rust plugin-host tests passed **97 / 0 failed /
6 ignored**. Prior policy coverage includes all **33 installation selectors**;
these are retained scopes, not additive totals. Native PowerShell scripts passed;
fresh Rust public-API children consumed a PS-produced pointer with flag absent,
`0` and `1`, all passed. Package formatting and strict Clippy passed with zero
errors/warnings; meaningful old-source RED is retained. Manual restart/update,
UNC and Linux qualification were not part of it. The later build-deployment
retirement and the omission of the authorization helper from private deployments
are covered by `scripts/tests/test-build-deployment-retirement.ps1` and
`scripts/tests/test-private-deployment.ps1`, not by this qualification.
