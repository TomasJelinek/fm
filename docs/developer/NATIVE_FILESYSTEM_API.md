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
The host consumer now lives in
[`midnight-native-filesystem-host`](../../crates/midnight-native-filesystem-host/src/lib.rs).
`Catalog::discover` performs bounded manifest discovery; `NativeProvider::open`
revalidates the complete package and the native descriptor before creating an
instance. It implements directory, metadata, volume and storage service adapters.
Its `capture` method returns content through the plugin's retained read handle.
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
arbitrary provider tokens.

This backend baseline makes the host and independent packages available for
composition. Application browsing and content routes are not changed by this
commit; build/deployment helpers and production cutover are separate work.
Watch/mutation contracts, generic provider identities and macOS remain
separate migration gates. Native tests are available in each package and
`midnight-native-filesystem-host`; this document makes no deployment claim.
See the [backend baseline checks](../verification/2026-11-09-native-backend-baseline.md)
for the tested candidate and its platform limits.
