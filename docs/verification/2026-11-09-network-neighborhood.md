# Network Neighborhood qualification — 9 November 2026

Implementation follows the interaction model of Salamander's `nethood` plugin:
discover containers/devices, open a selected share explicitly, then hand the
OS-supplied path to the native panel. No Salamander source or artwork was copied.
Windows and Linux have independent adapters; macOS is a later implementation.

## Qualified boundaries

- Public filesystem ABI 1.2: additive capabilities/operations, malformed response
  rejection, platform-native activation validation and transactional cleanup.
- Independent plugin: Windows WNet/MPR; Linux lazy GIO/GVfs bindings; raw scoped
  identities, Unicode, parent trails, manual parsing, partial pages, cache expiry,
  refresh epochs, cancellation, worker admission and retained-capacity accounting.
- Actual installed DLL/SO: trusted discovery/loading and 80 root/manual/resolve/
  stat/parent cycles per platform, exceeding the simultaneously allowed mount
  count to detect leaks. These probes never call List or Activate.
- Host/controller: first-page latency, malformed/foreign/canceled results,
  exact snapshot identity, stale mixed navigation, partial failures, full output
  queues, locale keys and native-operation fencing.
- Presentation: Network menu and shared toolbar, Enter/double-click, refresh, saved/parked tabs,
  share handoff, native-navigation failure, queue refusal, parent/back history,
  selected share restoration, and explicit file comparison choices.

## Regression findings fixed

The first listing request now asks for one entry, preventing a slow second
discovery result from hiding the first one. ListBegin and its first page share
one deadline. Cancellation retains OS permits until the actual call and cleanup
return. An ABI fixture blocks the second read to exercise this behavior.

Catalog charges now include nested vector arrays, both retained identity copies,
spare string/key capacities, cursor layout, and pending records from live/cache
enumeration. Rollback shrinks retained maps once per batch. The 16 MiB counter
bounds common retained catalog payload/container capacities with conservative
overhead; it does not claim to bound all OS/library/temporary allocations.

Rejected or failed handoff settles loading without redirecting to another share
or system directory. Returning from a native share restores the full Network
history and selected share. An unavailable restored tab retains its typed saved
address. Comparison never derives a native counterpart from a discovery panel's
previous native origin; the explicit File A/B picker remains available.
Pending handoffs keep the committed typed Network address and its original
native return location in both active and parked saved tabs. Only successful
native listing commit removes the virtual address from preferences.

## Evidence

Reproducible commands and immutable hash/log ledgers are under `_build/qa/`:

| Evidence directory | Checks |
| --- | --- |
| `network-boundaries` | API/host/controller review and 85 focused tests |
| `network-accounting` | Causal regression failures, final 54 Windows plugin tests, strict Clippy and formatting |
| `network-delivery` | Windows debug/release 54 tests each; Linux 52 plus the opt-in local runtime probe; installed metadata probes; packaging and license inventory |
| `network-neighborhood` | Application/core/UI tests, real input bridge, debug build, and deployment verification |
| `network-ui-linux` | Linux application integration and input/source hash audit |

Packaging tests pass on both OSes (55 tests, 11 platform/symlink skips each).
The plugin has its own Cargo project and tests, importing only the public API/SDK.
Inventory checks cover the pinned crate features and optional system runtime
licenses; GLib/GIO/GVfs are not bundled in the Linux package.

Windows application checks include 132 core tests (one subprocess-only fixture
ignored), 564 UI library tests, 10 focused engine/network tests and actual menu,
keyboard and pointer input. Application Clippy with `-D clippy::all` passed;
the pre-existing inline-tree unused-code warnings remain as described below.
All 1,421 handwritten source files pass the 100,000-byte limit.

The toolbar addition has a causal failing test for the missing button, followed
by two passing Network input tests and ten shared-toolbar regressions on both
Windows and Linux. Real pointer input verifies left/right active-panel targeting,
swapped panels, and disabled behavior with no plugin. The button follows Refresh
and uses the host system Network icon with the existing fallback; the tooltip
explains browsing or the unavailable plugin. Linux application checking also
passes. Logs are in `network-neighborhood/toolbar-*.log` and
`network-ui-linux/toolbar/`; the preceding full Linux run passed 559 UI tests.
Windows uses the Network Neighborhood stock icon (`SIID_MYNETWORK`) rather than
the mapped-drive icon. The mapping regression failed before the change; all 15
Windows icon tests then passed, including native artwork and bounded path-free
lookups. `network-icon-red.log` and `network-icon-green.log` retain this evidence.

The debug application and new Windows plugin are installed together at
`C:\Users\tjelinek\MidnightExplorer-20261109-network-final` with the existing
unchanged viewer/archive/thumbnail packages. The private deployer verifies all
copied files and ACLs; desktop shortcut and matching-build registration point to
the new installation. Headless and real-window software-renderer startup smoke
tests both exit 0. Screenshot inspection found no startup error dialog; the
preferences hash was unchanged. Exact hashes and shortcut target are recorded
in `network-neighborhood/runtime-results.json`.
The real-window startup smoke also passes when launched directly through
`D:\experiments\dev\FM\_build\debug\me.exe`, using the matching protected
plugin registration; `build-startup-results.json` records exit 0 and unchanged
preferences for that path as well.

The final screenshots referenced by both reports show the Network icon after
Refresh and no startup error dialog. The first repeated smoke attempt tried to
reuse an existing screenshot and correctly hit exclusive-create protection
(Windows error 80). The QA helper now assigns a unique screenshot filename;
both final headless and real-window runs exit 0 for both executable locations.
This required no change to the application's no-overwrite behavior.

- Application SHA-256: `ee5ebfa16ebd7701d3d5d2ac74fc531d26f8a7408eb4631d7e2e89a5f6ea504c`.
- Network DLL SHA-256: `3ef9287fb2a345e74f6af7aa16dc84bd5e0f656d683af41682bf9ba48174eeb8`.

## Limits

No live corporate LAN scan or uncontrolled endpoint access was performed. Fake
adapters and owned local GIO fixtures do not establish SMB/FUSE interoperability.
Qualification against a controlled real SMB server is still outstanding.
Discovery depends on OS providers and cannot promise a complete device inventory.

Linux credential challenges return AuthenticationRequired; this milestone does
not add an interactive credential workflow. A native handoff needs an actual
GIO/GVfs native path. Discovery-node Hot Paths are pending; ordinary mounted/native
share Hot Paths work. macOS discovery is not implemented.

The application tree still has existing unused inline-tree code warnings. These
are separate from the network plugin's strict warning-free checks. The earlier
full application test run exposed an obsolete comparison-toolbar assertion;
its corrected test preserves the already-requested manual File A/B picker.
