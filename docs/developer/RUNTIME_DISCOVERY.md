# Installed plugin runtime resolution

Application backend services call
`midnight_plugin_host::installed_executable() -> Result<PathBuf, Error>` before
discovering viewers, image/PDF providers, thumbnailers or native, network and
archive filesystems, and before locating the directory authorization helper.
This is an internal host API, not a public plugin ABI change. Hashing, registration
reads and filesystem checks run on workers. UI code receives results through its
messages. Explicit-root host APIs remain available for integration tests.

On Windows, a protected executable and parent directory take precedence. Adjacent
packages in a manually assembled installation may be absent; their absence never triggers fallback to another
installation.

An executable that is refused only because other accounts can change it or its folder
through the ACL (a **shared installation**, for example a release package extracted
below `C:\`, which grants Authenticated Users Modify) still consults the registrations
below, which only developers create. Without a valid registration, a missing or stale
pointer, it resolves to itself: release packages need no script, and the application
reports the shared folder once (`installation_sharing`, the shared-installation notice;
see [SECURITY.md](../../SECURITY.md)). Before 0.5.1 this case refused every plugin with
an error that pointed end users at a developer script. An executable refused for
another reason (a redirection, another account's file, an unsupported ACL entry) keeps
the earlier behaviour: without a registration it runs only without plugin folders, and
the error says to extract the download again into a folder of the user's own.
Registration roots, records, pointers and their targets are still checked strictly:
a shared registration is refused. Linux and other platform implementations retain executable-relative
discovery and their loader support and limitations.

For a shared Windows executable, `MIDNIGHT_VERIFY_RUNTIME_SHA256` selects
own-EXE verification. Absent or `0` is **OFF**; `1` enables the legacy strict
hash/snapshot route. Empty, other or non-Unicode values return an error. This
adds no GUI/preferences setting; the result or error is cached until restart.

OFF hashes neither the launch EXE nor the selected runtime EXE. A protected
per-launch-path pointer at
`%USERPROFILE%/.midnight-explorer/plugin-runtimes/launchers/<path-key>.toml`
selects the schema-1 registration. Native normalized raw UTF-16 path,
length and LastWriteTime provide identity/staleness hints; ChangeTime is excluded.
Rust and PowerShell use `GetFinalPathNameByHandleW` for native normalization.
The path-key SHA names a short path; it hashes path material, not EXE contents.
OFF makes no EXE byte-equality claim. Missing, invalid or stale selection does
not fall back to another registration or choose the newest runtime.

ON hashes the exact running EXE and follows the legacy protected
`%USERPROFILE%/.midnight-explorer/plugin-runtimes/<sha256>.toml` registration.
Both routes use the registration schema:

```toml
schema_version = 1
sha256 = "64 lowercase hexadecimal digits"
executable_utf16 = [67, 58, 92]
```

The example array illustrates encoding only. Real records contain the full
absolute native path of a deployed private `midnight-explorer.exe`, preserving UTF-16 including
unpaired surrogate units. Unknown fields, NULs, parent traversal, unsupported
namespaces and schema mismatches are rejected. ON additionally requires the
private EXE to have the same SHA-256 as the running executable. That comparison
matches builds; it is not publisher authentication and does not secure a shared
EXE that the user has already run. OFF retains ACL/owner, reparse-point,
regular-file, manifest, platform, package and ABI guards without claiming equal EXE bytes.

The registration directory, record, target and ancestors keep the owner/DACL
policy. Original path components are checked for reparse points before
canonicalization can conceal a junction. Content-read and registration handles
are opened without following the final reparse point, with read sharing only,
excluding writes/renames during the read. OFF source metadata handles do not
promise data-write exclusion. Deployment retains a separate `0x81` read lease
through copying and both publications. Guarded record reads retain length/native-revision checks; OFF EXE
staleness hints use only length/LastWriteTime. Records have a 1 MiB budget; ON
executable hashing has a 512 MiB budget and uses a 64 KiB buffer
(`MAX_REGISTRATION_BYTES`, `MAX_EXECUTABLE_BYTES` in `installation_windows.rs`).
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

`build.bat [debug|release] [--validate]` builds the host and its authorization helper,
the native and network filesystem providers, all six independent viewers, both
thumbnailers and the archive worker, then calls `scripts/deploy-build.ps1`. With
`--validate` it first runs `scripts/validate.ps1` ([full validation](TEST_EXECUTION.md#full-validation));
a build that fails validation is not deployed.
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
using exclusive temporary files and atomic replacement.
It refuses unsafe pre-existing directories/records and never repairs their ACLs.
It changes permissions only on new deployment/registration directories and deployed
files. Installations made with `deploy-private.ps1` without `-ReplaceBuildDeployment`
are retained. `build.bat` (through `deploy-build.ps1`, which passes
`-ReplaceBuildDeployment` to `deploy-private.ps1`) marks its installation and, after
registering it, removes earlier deployments marked for the same build output that no
launcher pointer selects and no running process uses; an unmarked deployment is never
removed automatically. Build deployments of one build
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
folder below `C:\Program Files` ([installation steps](../USER_GUIDE.md#build-and-run));
see the [directory authorization helper](../../SECURITY.md#directory-authorization-helper).
From a per-user runtime, Retry with authorization is refused with an explanation of
this requirement; plugin loading is unaffected. A helper found in a per-user runtime is
refused, because the runtime's user can change it.

Tests use `-RegistrationDirectory` with a protected disposable fixture. That
override requires `-RegisterRuntime`; no test needs to edit the actual profile.
With `-ReplaceBuildDeployment`, `deploy-private.ps1` refuses `-Destination` and always
deploys directly below the profile, so the retirement test points `USERPROFILE` at a
disposable directory.
Tests: the plugin host's installation tests (`installation_windows_tests.rs`), and
`scripts/tests/test-private-deployment.ps1` and
`scripts/tests/test-build-deployment-retirement.ps1` for deployment, retirement and the
omitted authorization helper. Design: [resolution design](../superpowers/specs/2026-11-07-trusted-runtime-resolution.md);
evidence: [resolution record](../verification/2026-11-07-trusted-runtime-resolution.md).
