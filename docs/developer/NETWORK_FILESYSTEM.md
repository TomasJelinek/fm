# Network Neighborhood

The independent `plugins/midnight-filesystem-network` Cargo workspace depends
only on the public filesystem API/SDK and its target-specific system bindings.
Its fixture and ABI tests belong to the plugin. The application does not link
the plugin implementation; the installed DLL/SO is discovered under `filesystems/`.
Windows and Linux adapters are separate files. macOS discovery remains pending.

## Public contract

ABI 1.2 adds explicit `Activate`, cache-only `Refresh`, and pure `ResolveText`.
Resolve restores a provider-owned opaque address. It must not activate a share.
Activation returns either another opaque address in the same provider or an
OS-supplied absolute native path. The host validates the platform/path and keeps
the Network view until native navigation succeeds. No display text, remote URI,
caption or generated path grants native filesystem authority.

`filesystem-network` manifests require exactly `browse`, `stat`, `volumes`,
`activate`, `refresh` and `resolve-text`, opaque address encoding, and host minor
version 2. They cannot qualify as the installed native filesystem provider.
The host reuses its audited C transport, trust checks and response-buffer leases.

Resolve/stat/parent/manual parsing perform no discovery. Listing is incremental,
with exact cursor ordinals, bounded resource ownership and complete-only caching.
The host requests one entry in the first page, then up to 128 per page so a slow
second device cannot delay the first visible result. ListBegin and its first page
share one ten-second deadline; subsequent pages each receive a new deadline.
The two shared discovery permits include activation and cursor destruction;
cancellation never releases a permit while native code is still running.
Every issued mount/node/cursor belongs to its instance and retained parent trail.
Reply publication commits resources transactionally; an abandoned response rolls
back. A fully rolled-back late publication may return `StaleReference` without
poisoning unrelated mounts, as documented by `Reply::owned` in the SDK.

## Application boundary

`NetworkNeighborhood` and `NetworkAddress` live in `midnight-core`.
`midnight-native-filesystem-host::NetworkProvider` adapts the C transport without
calling native content services on discovery nodes. Its backend controller uses
Tokio blocking workers, bounded admission and navigation-generation fencing.
`Entry::native_path()` returns `None` for both archive and network virtual entries;
native selection indexes exclude them. UI controls never perform discovery.

Network tabs persist the provider ID and opaque bytes, not live handles, credentials,
caption-derived paths or cached rows. Missing plugins leave saved Network tabs
unavailable. Known UNC paths or already-mounted shares still use the native provider.
Discovery Hot Paths are not implemented; native share Hot Paths remain supported.

Choose **Network Neighborhood** in either panel's drive dropdown or drive bar.
Both controls place it directly after all drives and before local special folders;
the drive bar opens the Network view in its own panel. Its system/fallback network
icon has a tooltip, and the action is disabled when the plugin is unavailable.
Enter or double-click opens
the captured device/share; Up and Back retain the discovery context after a native
share opens. A failed native handoff keeps the Network view and error, without
recovering to an unrelated native directory. Known absolute native share paths
remain usable even when the discovery plugin is missing. Device captions receive
locale sort keys on the backend before partial/final publication.

Accepted Network navigation immediately displays the requested location in the
path, breadcrumbs and active tab. Until the first provider publication, rows from
a different committed location are hidden and native operations remain disabled.
This pending presentation stores the original action, never a fabricated provider
address or native path. A refused enqueue leaves the current location intact;
a failed request restores the committed listing and displays its error. Partial
publications reveal their validated location and rows while retaining loading
feedback. Refreshing the same Network address retains its rows. Sorting, refreshing
or matching another panel cannot repeat pending share activation or text parsing.

Both native and Network listings show an indeterminate, accessible loading notice
within the panel, including before the first backend event and during refresh.
The notice overlays the bottom of the list without moving rows or changing the
viewport. Navigating elsewhere supersedes the request through the existing
generation/cancellation boundary; the notice does not claim to terminate a blocked
OS call or release its worker permit. Saved tabs continue to use committed typed
provider locations rather than provisional captions.

## Platform behavior and qualification

Windows uses Unicode MPR (`WNetOpenEnumW`, `WNetEnumResourceW`) with bounded buffers
and RAII cleanup. It does not scan IP ranges or invoke implicit connection dialogs.
The Windows root quickly lists installed provider containers through
`RESOURCE_GLOBALNET`; discovering a particular provider starts only when it is
opened. This avoids a slow `RESOURCE_CONTEXT` sweep that can return only an unnamed
Entire Network wrapper. Native provider errors remain visible even after a slow
call, with cancellation taking precedence. An unavailable Windows browser service
does not imply that another provider or an explicitly entered UNC share is absent.
Linux loads system GIO/GObject/GLib lazily and uses GVfs target URIs. Explicit
activation can initialize a namespace or request a share mount; credential
challenges abort with a distinct authentication-required result. Native handoff
requires the actual GIO/GVfs path, usually supplied by GVfs FUSE.

Use plugin tests with fake adapters for discovery, paging, deadlines, cancellation,
resource limits and failure states. Ordinary tests must not probe a real LAN.
Real local GIO fixtures test callback draining and cleanup. Real SMB interoperability
still requires a controlled server/runtime fixture and is not established by those
tests. Review the [design](../superpowers/specs/2026-11-05-network-filesystem-design.md)
and the plugin's platform dependency notes before adding another adapter.
