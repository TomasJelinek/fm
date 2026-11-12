# Network Neighborhood filesystem plugin

Independent Windows/Linux browse provider built against the public filesystem
API/SDK only. The package contains a shared library, its platform manifest and
embedded third-party notices; no application crate is needed to compile it.
macOS discovery is not implemented.

Windows uses WNet provider enumeration. Linux loads system GIO/GLib lazily and
follows the exact `standard::target-uri` supplied by GVfs for network, SMB and
DNS-SD entries. A bare hostname can be entered explicitly. Listing does not scan
subnets, mount shares or prompt for credentials. Discovery visibility depends on
the operating system's installed providers, services and network configuration.

Opening a namespace or share explicitly may initialize it without interactive
credential UI. A share activates only when the OS supplies a usable native path;
Linux therefore needs the actual GIO native/FUSE path. Unavailable runtime,
offline, mount required, authentication required, permission denied and unsupported
handoff are separate outcomes. This version has no credential-entry workflow.

The catalog streams up to 128 entries per page and caches only complete listings
for 30 seconds. Refresh retires old cursors and prevents stale publication.
All plugin instances share two active discovery permits, 4096 retained nodes,
64 mounts and a 16 MiB common catalog budget. Cancellation and the 10-second
per-call deadline are cooperative: a blocked OS call retains its permit until it
returns and cursor cleanup finishes. There are no replacement worker threads.
Display captions stay fixed for an issued node lease; a fresh mount/listing uses
newly discovered captions without changing provider identity.

## Build and package

From the repository root, build the package beside an application:

```powershell
./scripts/build-network-filesystem.ps1 -Profile debug -Test -Offline
```

```sh
MIDNIGHT_TEST_NETWORK_FILESYSTEM=1 MIDNIGHT_NETWORK_FILESYSTEM_OFFLINE=1 \
  bash scripts/build-network-filesystem.sh debug
```

Use `-AppDirectory`/the second shell argument to select the application directory,
and `-BuildRoot` or `MIDNIGHT_NETWORK_FILESYSTEM_BUILD_ROOT` for isolated compiler
output. Defaults are `_build/network-filesystems/{windows,linux}` and the current
application target/profile. The ordinary `build.bat`/`build.sh` entry points also
build this plugin. Packages install to `filesystems/network.{windows,linux}`.

Standalone public-only checks use `cargo test --manifest-path
plugins/midnight-filesystem-network/Cargo.toml --locked`, formatting and strict
Clippy. Ordinary tests use injected fixtures or owned temporary local files;
they never discover the real LAN. The ignored Linux unavailable-runtime probe
requires `GIO_USE_VFS=local` and rejects use with real network discovery enabled.

## Linux runtime

The loaded GLib/GObject/GIO ABI must expose the required GLib 2.28-era symbols
(in particular [`g_list_free_full`](https://docs.gtk.org/glib/type_func.List.free_full.html));
missing libraries or symbols report Unavailable. This is a symbol compatibility
floor, not a tested operating-system support promise. The tested local runtime is
Ubuntu 24.04 GLib/GIO 2.80.0; the main application's Linux requirements still apply.

Install the distribution's GLib/GIO and GVfs network backends for discovery, plus
its GVfs FUSE bridge for native-path handoff when required. These are optional
system components, not bundled by this package. A desktop session's D-Bus/GVfs
services and provider configuration also affect availability. Minimal/headless
systems can correctly report Unavailable/Unsupported rather than an empty list.
See [THIRD-PARTY.md](THIRD-PARTY.md) for pinned crate licenses, runtime provenance
and redistribution obligations. Live SMB/FUSE interoperability has not been
qualified in the current fixture-only environment.
