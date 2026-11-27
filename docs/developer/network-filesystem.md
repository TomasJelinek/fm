# Network Neighborhood

The independent `plugins/midnight-filesystem-network` Cargo workspace depends
only on the public filesystem API/SDK and its target-specific system bindings.
Its fixture and ABI tests belong to the plugin. The application does not link
the plugin implementation; the installed DLL/SO is discovered under `filesystems/`.
Windows and Linux adapters are separate files. Known limit: there is no macOS discovery.

## Public contract

ABI 1.2 adds explicit `Activate`, cache-only `Refresh`, and pure `ResolveText`.
Resolve restores a provider-owned opaque address. It must not activate a share.
Activation returns either another opaque address in the same provider or an
OS-supplied absolute native path. The host validates the platform/path and keeps
the Network view until native navigation succeeds. No display text, remote URI,
caption or generated path grants native filesystem authority.

`filesystem-network` manifests require `browse`, `stat`, `volumes`, `activate`,
`refresh` and `resolve-text`, opaque address encoding, and host minor version 2
or later. The bundled Linux network provider adds `sign-in` and requires ABI
minor 5. Network packages cannot serve as the installed native filesystem provider.
The host reuses its audited C transport, trust checks and response-buffer leases.

Resolve/stat/parent/manual parsing perform no discovery. Listing is incremental,
with exact cursor ordinals, bounded resource ownership and complete-only caching.
The host requests one entry in the first page, then up to 128 per page so a slow
second device cannot delay the first visible result (`max_entries` in
`network/listing.rs`). ListBegin and its first page share one ten-second deadline
(`REQUEST_TIMEOUT` in the plugin's `src/catalog.rs`, applied in `catalog/listing.rs`);
subsequent pages each receive a new deadline. The two shared
discovery permits (`network_neighborhood.rs`) include activation and cursor destruction;
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

Network tabs and discovery Hot Paths persist provider IDs and opaque addresses.
The provider contract forbids credentials in these addresses. The versioned codec
stores only the provider ID, opaque bytes and presentation label; the bookmark's
native return location is a separate field. It stores no live handles or cached
rows and never interprets the label as a UNC path or URI. Known UNC paths and
already-mounted shares continue to use native Hot Paths.

Discovery Hot Paths need preferences schema 29 and use the version-1 Network address
codec. Native and archive bookmarks keep their representation; older preferences
remain readable. The schema preserves all other settings,
including either value of copy checksum verification. Unknown address fields and
versions, invalid addresses, mixed archive/Network locations and duplicate shortcut
slots are rejected. The 4 MiB aggregate settings bound applies.

A Network Hot Path is identified by provider ID plus opaque bytes. Caption or
native-return-folder changes do not create a different location; re-adding a
published location refreshes its display while preserving the saved identity and
shortcut. An unfinished Network request cannot be saved as a guessed address.
Hot Path keys, reassignment and removal use the ordinary effective shortcut map.

Reopening sends `Browse`, never `Activate` or `Parse`, through the same typed
location machinery as restored tabs. Before the first publication it displays an
empty Network location with native operations disabled. Missing providers and
stale-address failures preserve that unavailable location and saved bookmark;
late replies cannot replace a newer navigation. Refused queue admission preserves
the previously displayed pane. Evidence and limits:
[Hot Path record](../verification/2026-11-11-network-hot-paths.md).

Choose **Network Neighborhood** in either panel's drive dropdown or drive bar.
Both controls place it directly after all drives and before local special folders;
the drive bar opens the Network view in its own panel. Its system/fallback network
icon has a tooltip, and the action is disabled when the plugin is unavailable.
Enter or double-click opens
the captured device/share; Up and Back retain the discovery context after a native
share opens. A failed native handoff keeps the Network view and error, without
recovering to an unrelated native directory. Known absolute native share paths
remain usable even when the discovery plugin is missing. Device captions receive
locale sort keys on the backend before partial/final publication. Quick search and
persistent panel filters match captions and identify Network rows by entry ID,
because these rows have no native path.

Accepted Network navigation immediately displays the requested location in the
path, breadcrumbs and active tab. Until the first provider publication, rows from
a different committed location are hidden and native operations remain disabled.
This pending presentation stores the original action, never a fabricated provider
address or native path. A refused enqueue leaves the current location intact;
a failed request restores the committed listing and displays its error. Partial
publications reveal their validated location and rows while retaining loading
feedback. Refreshing the same Network address retains its rows. Sorting, refreshing
or matching another panel cannot repeat pending share activation or text parsing.

The path bar names the shown Network location after its ancestors, the outermost
first, for example `Network › Microsoft Windows Network › WORKGROUP › SERVER` with
WNet/MPR or `Network › Windows Network › WORKGROUP › server` with GVfs. Each
part of the chain browses its location (`Browse`, never `Activate` or `Parse`); an
ancestor's part leaves the cursor on the location below it, as Up does, and the last
part reads the location again. ` › ` is display text, not a path separator: a part's
tooltip and right-click copy give the captions up to that part, and the path editor
still starts from the location's own caption. A shortened bar follows the native
rules: the root and the shown location stay whole while they fit, and `...` stands
for the omitted middle. A share opens its native folder with the native path bar; Up
or Back at its root returns to the Network location and its chain.

Every listing publication carries this chain (`NetworkListing::ancestors`, the
parent last). The engine asks the provider for the location's parent, then for that
one's parent, up to a root; these lookups resolve opaque addresses without discovery.
Nothing is inferred from how the location was reached, so Enter, Up, Back, a restored
tab and a Hot Path show the same chain, and the part before the last is what Up
opens. Only the parent is required, and failing to report it fails the request.
Further out, a failed lookup, a location already in the chain, 64 ancestors or 16 MiB
of address and caption bytes beyond the parent (`MAX_ANCESTORS`, `MAX_ANCESTOR_BYTES`
in `network_neighborhood/ancestors.rs`) end the chain with the nearer ancestors, so the bar may then start below the root. The bundled plugin's addresses
carry their discovery trail, so its chains reach the root. A location entered as text
(`\\server`, `smb://server/`) has only the root above it, since the provider does not
know its domain or workgroup; a provider that reports no parent leaves the single
caption. Until a new location publishes its chain, a pending request shows its caption
alone, except a refresh or sort of the shown location, a visit to one of its ancestors
(a part, Up) and a return from a share, which keep the known chain up to the
requested location.

Both native and Network listings show an indeterminate, accessible loading notice
within the panel, including before the first backend event and during refresh.
The notice overlays the bottom of the list without moving rows or changing the
viewport. Navigating elsewhere supersedes the request through the
generation/cancellation boundary; the notice does not claim to terminate a blocked
OS call or release its worker permit. Saved tabs continue to use committed typed
provider locations rather than provisional captions.

## Platform behavior and tests

Windows uses Unicode MPR (`WNetOpenEnumW`, `WNetEnumResourceW`) with bounded buffers
and RAII cleanup. It does not scan IP ranges or invoke implicit connection dialogs.
The Windows root quickly lists installed provider containers through
`RESOURCE_GLOBALNET`; discovering a particular provider starts only when it is
opened. This avoids a slow `RESOURCE_CONTEXT` sweep that can return only an unnamed
Entire Network wrapper. Native provider errors remain visible even after a slow
call, with cancellation taking precedence. An unavailable Windows browser service
does not imply that another provider or an explicitly entered UNC share is absent.
Linux loads system GIO/GObject/GLib lazily and uses GVfs target URIs. Explicit
activation can initialize a namespace or request a share mount. When a protected
SMB server or share requests credentials, the first mount attempt ends and the app
shows its sign-in dialog. No worker waits for the dialog. Submission starts a new
activation for the same opaque location, with one credential response to GIO. A
repeated challenge opens the dialog again. Canceling the dialog leaves the
previous listing in place. A server and a share may each request credentials.
A successful sign-in is remembered by the provider instance for its SMB server
(`Adapter::sign_in_scope`: `smb://` and the host and port in ASCII lower case;
`catalog/sign_ins.rs`). A later activation of a server or share there is sent as a
sign-in with the remembered credentials, which answer its password challenge once.
Another server, a container, listing and discovery get none. A rejection forgets
them, and the engine then shows the dialog as for a first sign-in. Nothing is
persisted, and GIO still receives `G_PASSWORD_SAVE_NEVER`. At most 32 servers are
remembered.
Native handoff requires the actual GIO/GVfs path, usually supplied by GVfs FUSE.

Use plugin tests with fake adapters for discovery, paging, deadlines, cancellation,
resource limits and failure states. Ordinary tests must not probe a real LAN.
Real local GIO fixtures test callback draining and cleanup. Known limit: real SMB
interoperability is not tested; it needs a controlled server fixture. Review the [design](../superpowers/specs/2026-11-05-network-filesystem-design.md)
and the plugin's platform dependency notes before adding another adapter.
