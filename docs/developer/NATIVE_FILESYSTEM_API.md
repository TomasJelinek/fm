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

## Prepared Rename core/host lifecycle

The core and native host implement this prepared-Rename lifecycle. Engine/UI
integration with the contract remains unimplemented. Installed-provider/runtime
qualification and deployment are tracked in [Active work](../ACTIVE_WORK.md#provider-increments-from-the-architect-review).
This internal Rust contract does not change the public plugin ABI/SDK.
`NativeProvider` requires native-address/prepared-Rename capabilities; unsupported
providers do not fall back to host path mutation.

`RenameService::prepare(path, cancel)` returns `Arc<dyn PreparedRename>` or
`RenamePrepareFailure`. Prepare, `attempt`, `query`, `finish` and final resource
destruction run on backend workers. Only cached `item()`/receipt data is I/O-free;
a preparation describes the object captured at prepare time, not an earlier
listing's identity.

Each `RenameAttempt` owns a nonzero sequential number, raw native basename and
policy. Numbering starts at one; only Conflict permits the next attempt. Reusing
a number requires the identical name and policy and returns its cached state
without another mutation. `NoReplace` is the baseline.
`RecoverableCurrentEntry` requires the latest owner-bound conflict token, the
same requested basename and an explicitly replaceable observation. Changed
names/targets require fresh conflict/consent; observed target spelling never
rewrites the immutable requested name.

`query()` recovers receipts without user cancellation or mutation resubmission.
A lost/malformed reply or transport failure does not authorize another Rename.
`RenameOutcome::Unconfirmed` blocks new mutating dispatch on that owner. An already admitted
handler or its associated receipt recovery can still establish authoritative
namespace facts without reopening admission. Preserve known terminal facts,
including Renamed after late cancellation or cleanup failure.

`RenameResource::finish()` is finite, idempotent and serialized with attempt
admission. Once finishing begins, new mutation is closed; receipt recovery may
continue while the known handle remains live. Finite does not promise a wall-clock
bound on a blocked native callback. Rename success and resource disposal are
recorded separately.

| Finish result | Required ownership |
| --- | --- |
| `Closed` | All owned or potentially published resources are disposed. Known handles require acknowledged CloseRename Unit, then acknowledged CloseMount, with callbacks and reply-buffer releases complete. Only then may the caller release its full job/capture/mutation gate/retirement slot. |
| `Executing` | Identifiable associated work still owns resources. Retain the complete chain until that actual work and receipt recovery finish. Busy alone is not proof of such work. |
| `Unconfirmed(error)` | Quarantine the complete ownership chain. A handle-only deferred close or endless cleanup retry is insufficient; an unassociated Busy close is unconfirmed. |

`RenamePrepareFailure { error, retained }` is an owned failure, even when no usable
preview or known handle exists. Uncertain Resolve/Prepare resource publication
requires `Some(retained)`; `None` is allowed only when no owned or potentially
published resource remains. Attach the retained owner to the complete caller
job/capture/gate/retirement payload before reporting the error, and keep it until
finish acknowledges Closed. Host Drop retains its own unclosed component; it
cannot preserve the caller's outer owners by itself. See the
[engine disposal boundary](ENGINE_LIFETIME.md#prepared-rename-disposal-boundary).

Recovery receipts preserve source/displaced-target/empty-stage roles, source
location, exact native paths and opaque identities. They are provider-reported
diagnostic observations, not independently verified mutation authority or
permission to open/delete recovery paths. Retain valid partial recovery details
when an untrusted response cannot establish a terminal result. Report
`durability_confirmed` separately from a confirmed Renamed namespace outcome.

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
