# Installed plugin runtime resolution

Application backend services call
`midnight_plugin_host::installed_executable() -> Result<PathBuf, Error>` before
discovering viewers, image/PDF providers, thumbnailers or archive filesystems.
This is an internal host API, not a public plugin ABI change. Hashing, registration
reads and filesystem checks run on workers. UI code receives results through its
existing messages. Explicit-root host APIs remain available for integration tests.

On Windows, a protected executable and parent directory take precedence. Adjacent
packages in a manually assembled installation may be absent; their absence never triggers fallback to another
installation. Linux and other platform implementations retain executable-relative
discovery and their existing loader support/limitations.

For a shared Windows executable, the resolver hashes the exact running EXE and
reads `%USERPROFILE%/.midnight-explorer/plugin-runtimes/<sha256>.toml`:

```toml
schema_version = 1
sha256 = "64 lowercase hexadecimal digits"
executable_utf16 = [67, 58, 92]
```

The example array illustrates encoding only. Real records contain the full
absolute native path of a deployed private `me.exe`, preserving UTF-16 including
unpaired surrogate units. Unknown fields, NULs, parent traversal, unsupported
namespaces and schema/hash mismatches are rejected. The private EXE must have the
same SHA-256 as the running executable. This matches builds; it is not publisher
authentication and does not secure a shared EXE that the user has already run.

The registration directory, record, target and ancestors retain the existing
owner/DACL policy. Original path components are checked for reparse points before
canonicalization can conceal a junction. Files are opened without following the
final reparse point, with read sharing only; writes and renames cannot race an
open read. Length and native revision are checked before/after reads. Records have
a 1 MiB budget; executable hashing has a 512 MiB budget and uses a 64 KiB buffer.
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

`build.bat [debug|release]` builds the host, filesystem providers, independent
all six viewers, both thumbnailers and the archive worker, then calls
`scripts/deploy-build.ps1`.
It requires 64-bit PowerShell 7 and honors `CARGO_TARGET_DIR`; missing image/video
helpers, filesystem/viewer modules, runtime libraries or corresponding source/notices
fail before deployment. Linux `build.sh` includes the same module set in its native
and Docker chains too. Independent modules remain separate public-SDK consumers;
their inclusion in normal builds is mandatory.
Direct Cargo builds require explicit deployment if their
output is not trusted:

```powershell
powershell -NoProfile -File scripts/deploy-private.ps1 -SourceDirectory _build/release -RegisterRuntime
```

Add `-IncludeFilesystems` and `-IncludeThumbnailers` for a complete normal
installation, and `-CreateDesktopShortcut` if wanted. The lower-level deployment
tool retains selective package options for specialist fixtures. The helper creates a new runtime
installation under the user profile, verifies copied bytes with SHA-256 readback,
and then publishes the record by an exclusive temporary file and atomic replacement.
It refuses unsafe pre-existing directories/records and never repairs their ACLs.
It changes permissions only on new deployment/registration directories and deployed
files. Existing deployments are retained. Compiler/test output stays under `_build`.
Restart the app after changing/deploying plugins; the helper never launches it or
changes user preferences.

Tests use `-RegistrationDirectory` with a protected disposable fixture. That
override requires `-RegisterRuntime`; no test needs to edit the actual profile.
See [the resolution design](../superpowers/specs/2026-11-07-trusted-runtime-resolution.md)
and [verification](../verification/2026-11-07-trusted-runtime-resolution.md).
