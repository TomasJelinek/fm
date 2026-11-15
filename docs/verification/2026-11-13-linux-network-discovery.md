# Linux Network Neighborhood — 13 November 2026

Scope: `codex/filter-linux-drive-list`, merge base `2b62996`, with uncommitted
Linux desktop and network changes. Evidence is under `_build/qa/linux-network/`.
Only Debug compilation was performed for this request.

## Diagnosis and changes

The user confirmed that clicking the drive-bar Network button opens Network but
shows no entries. UI routing already sends the correct `OpenRoot` action. The
running release process had loaded its adjacent network plugin. Its GIO/D-Bus
environment matched the diagnostic process, and system GIO independently returned
zero entries for `network:///`.

Direct `gio list wsdd:///` exposed the hidden runtime failure: GVfs could not spawn
its WSDD helper. Ubuntu's `wsdd` package was not installed. The combined GVfs
network backend can omit entries from a failed child discovery backend. The
inspected implementation was GVfs 1.54.4:
[network backend](https://github.com/GNOME/gvfs/blob/1.54.4/daemon/gvfsbackendnetwork.c),
[WSDD service](https://github.com/GNOME/gvfs/blob/1.54.4/daemon/gvfswsddservice.c).

The official Ubuntu package `wsdd` 2:0.7.1-5 was downloaded and inspected before
installation. Its upstream 0.7.1 code and Debian packaging use MIT, and its only
declared dependency is system Python 3. This package has no maintainer scripts or
system service; the separate `wsdd-server` package was not installed. The dry run
selected one new package, no upgrades or removals. Installation through desktop
authorization succeeded. GVfs now runs `/usr/bin/wsdd --no-host --discovery` with
its private session socket. No user ME instance was killed or restarted.

A separate application compatibility bug was reproduced and fixed:

- The Linux plugin silently rejected GVfs's separate `wsdd:///` namespace.
  It now accepts exactly that URI as a discovery container. Credential-bearing,
  non-root, malformed and unsupported addresses remain rejected.
- Entered `wsdd://` text previously fell through to native path navigation.
  The presenter now forwards it to the network provider, which owns validation.
- A Linux-only, ignored, opt-in native integration test exercises installed plugin
  loading, pure parsing/parent identity, explicit namespace activation, refresh
  and listing through the actual GIO/GVfs desktop session. It requires explicit
  `MIDNIGHT_NETWORK_TEST_ALLOW_LAN=1` and never opens a share or native file.

The API/SDK and Cargo dependencies/features are unchanged. Runtime/license notes,
system requirements and Linux packaging instructions now document `wsdd` and
keeping `filesystems/` beside the executable. No system runtimes are bundled.

## Verified results

| Boundary | Result | Artifact |
| --- | --- | --- |
| Exact namespace and backend filtering, before implementation | 2 expected failures | `plugin-wsdd-red.log` |
| Entered namespace routing, before implementation | 1 expected failure | `ui-red/` |
| Network UI integration | 4 passed | `ui-green/summary.json` |
| Network presenter behavior, including new routing regression | 23 passed | `ui-green/summary.json` |
| Applicable Linux plugin library/integration targets | 54 passed; 1 opt-in runtime case ignored | `host-test-results.json`, per-target `*-host.log` |
| Explicit local-only unavailable-backend runtime test | 1 passed | `gio-local-only-host.log` |
| Installed trusted ABI metadata/cleanup test | 1 passed | `installed-metadata-host.log` |
| Installed WSDD and combined root through the real desktop | 1 passed | `native-wsdd-final.log`, `native-wsdd-final.json` |
| Debug app/elevation helper and staged network package build | exit 0 | `debug-build.jsonl`, `package-build.log` |

Total: **84 targeted tests passed**. The build image lacks system GIO, so its first
plugin test run failed five local GIO fixtures with `Unavailable`. The same compiled
artifacts passed on the host with system GIO and `GIO_USE_VFS=local`, which prohibits
remote discovery for ordinary tests. The explicit installed integration test used
the real desktop session instead. Its initial assertion incorrectly assumed an
opaque provider address was a raw URI; that test assumption was removed, and the
rerun passed. The earlier failures remain in the artifacts rather than being
reported as passing runs.

Artifact paths are selected from successful Cargo JSON receipts, with explicit
container-to-host path mapping and binary hashes. Tests ran serially; Root held
the shared Cargo build lease. Formatting, whitespace and source-size checks passed.

## Qualification limits

After installation, both the native WSDD namespace and combined Network root
enumerated successfully with **zero discovery entries**. The running helper and
successful host/plugin calls establish that discovery can start; they do not
establish that any particular remote computer is reachable. No target hostname/IP
was supplied during this run. SMB authentication, mounted-share file browsing and
FUSE interoperability therefore remain unverified here.

Discovery is asynchronous. Refresh after devices have had time to announce
themselves; the plugin's complete-list cache remains 30 seconds, with explicit
Refresh invalidation. This change does not introduce continuous monitoring or
automatic refresh. The user's existing Release process remains unchanged; the
updated application and package are under `_build/debug/`.
