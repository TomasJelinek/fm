# Native filesystem plugin development

The native Browse/Read service is a separate C ABI from archive workers and
viewers. Start with [the public protocol](../../filesystem-api/NATIVE_PLUGIN_PROTOCOL.md),
[C header](../../filesystem-api/include/midnight_filesystem.h) and
[author SDK](../../filesystem-api/midnight-filesystem-plugin-sdk/README.md).
The [Windows](../../plugins/midnight-filesystem-native-windows/README.md) and
[Linux](../../plugins/midnight-filesystem-native-linux/README.md) packages are
independent Cargo workspaces using only the public API/SDK and OS bindings.

The host calls `midnight_filesystem_entry_v1`, verifies descriptor and manifest
identity/version/capabilities, then creates bounded instances on backend workers.
Browse, Read, Stat, Volumes and Storage are explicit service families. IDs are
opaque and owner scoped; native provenance paths/captions never substitute for
retained read handles. First listing pages stream before full enumeration.
Cancellation is borrowed for one synchronous call and cleanup remains available.

Resource-producing handlers return `Reply::owned` and close undisclosed resources
when encoding/publication fails or cancellation is observed after the handler.
The SDK retains instance/call ownership through cleanup; the host must retain
the library through instances, calls, resources and retained reply buffers.
Native packages are trusted in-process code, without a sandbox claim.

Build and test output belongs under `_build/`; inspect each pinned dependency's
license/features and ship its notices. The project's own license is undecided.
See [independent package evidence and limits](../verification/2026-11-07-native-filesystem-packages.md).
The host consumer now lives in
[`midnight-native-filesystem-host`](../../crates/midnight-native-filesystem-host/src/lib.rs).
`Catalog::discover` performs bounded manifest discovery; `NativeProvider::open`
revalidates the complete package and the native descriptor before creating an
instance. It implements directory, metadata, volume and storage service adapters.
The manifest may include an optional friendly `name` of at most 128 UTF-8 bytes,
nonblank and without control characters. Missing names preserve old packages.
The host retains the declared native/network kind separately from that display
name; neither names nor captions grant provider or native-path authority.

The application's `FilesystemPluginsListed` event exposes this manifest-only
catalog together with archive package metadata. It does not load the library or
add any public ABI operation. Its Installed status describes validated manifest
and artifact presence, not successful runtime initialization. The Options overview
is read-only and supplies no filesystem enable/disable or hot-reload capability.
`NativeProvider::capture` returns content through the plugin's retained read handle.
ABI minor 1 optionally advertises `volume-details`: labels preserve raw native
text, while rich information keeps physical free/total and caller-available space
distinct. Old plugins remain loadable; an absent capability returns Unsupported.
Volume text also accepts valid NUL-free UTF-8 independently of the platform's raw
name encoding. Timestamp display localization belongs to the host's backend
locale service; plugin timestamps do not require another native stat just to
format the date and time.

Use `CaptureCache` when preparing many file identities, such as the viewer's panel
order: capturing a revision must not pin a reader for every sibling. The cache holds
16 active readers, validates revisions on reopen and releases entries when their
captured source is dropped. `InstalledProvider::shared(home)` is a lazy backend-only
installation binding pinned until application exit. It has no builtin fallback.
Do not call these APIs or drop their resource owners on the UI thread.

Nonempty native reads use the host's [checked-read contract](CONTENT_READS.md).
The native SDK validates the pinned handle and pathname before and after ReadAt,
so the adapter avoids duplicate ValidateRead calls around the same read. Original
captured metadata, byte/EOF checks, explicit revalidation and validation of empty
reads remain enforced. This host optimization requires no plugin ABI change.

For a persisted native content reference, `InstalledProvider::capture_matching`
also accepts an optional expected revision. It captures through the same installed
provider and checks cancellation before returning. Exact current v1 tokens and
their documented Windows/Linux legacy equivalents are accepted only when every
identity/revision field agrees; mismatches return `SourceChanged`. This is a
host-side migration adapter, not a new wire operation or permission to reinterpret
arbitrary provider tokens. Archive restoration uses it before launching the decoder
and stores the current validated token. See [archive capture qualification](../verification/2026-11-09-archive-native-capture.md).

[Host qualification](../verification/2026-11-08-native-filesystem-host.md) includes
actual DLL/SO loading, missing/duplicate/incompatible packages, removed-library
refusal, cancellation cleanup and the reader-budget regression. The application
composition now uses this installed service for browsing, storage queries, native
F3, comparison/thumbnail capture and native archive-container capture. Root build
helpers install the matching native package under
`filesystems/native.<platform>/`; direct Cargo builds need the
`scripts/build-native-filesystem.ps1` or `.sh` helper before launching.
Missing/incompatible packages have no static browsing or viewer-read fallback.
Composition qualification is tracked in the
[cutover report](../verification/2026-11-08-native-filesystem-cutover.md). Watch/mutation
contracts and generic provider identities remain separate migration gates.

## Building the native package

The root `build.bat` and `build.sh` helpers build the host, then the platform's
native filesystem package, then the viewers. Native packages depend only on the
public filesystem API/SDK and keep their own lockfiles and notices. Building or
packaging a provider does not prove that an application route uses it.

To build and test just the Windows package, run:

```powershell
scripts/build-native-filesystem.ps1 -Profile debug -Test -Offline
```

`-AppDirectory` selects the executable directory; `-BuildRoot` selects the native
plugin build directory. Defaults are the current Cargo target's `debug` or
`release` directory and `_build/native-filesystems/windows`, respectively.
`MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE=1` also enables offline builds. Remove `-Offline`
when locked dependencies still need downloading.

On Linux:

```sh
MIDNIGHT_TEST_NATIVE_FILESYSTEM=1 MIDNIGHT_NATIVE_FILESYSTEM_OFFLINE=1 \
    bash scripts/build-native-filesystem.sh debug
```

The optional second argument selects the executable directory;
`MIDNIGHT_NATIVE_FILESYSTEM_BUILD_ROOT` overrides `_build/native-filesystems/linux`.
The executable directory defaults to the current Cargo target's profile directory.
Change `debug` to `release` for either helper. Tests run before the build/install
step; omit the test switch or variable for a build alone.

Both helpers stage the library, manifest, README and notices before publishing
them under `filesystems/native.windows` or `filesystems/native.linux`. Linux
replaces the shared library by renaming a completed new inode, preserving existing
mapped readers. Windows refuses to replace a locked DLL before changing the
manifest or notices. Restart hosts to load a newly installed library. These are
package-file installation steps, not an atomic update of an entire running app.

CI builds each platform plugin with only its source and the public filesystem
API/SDK present, checks the API/SDK and plugin, and passes the package artifact to
the host's protected-installation integration tests. Release archives require one
compatible native provider; optional archive workers can coexist alongside it.
