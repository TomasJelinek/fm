# Native filesystem host qualification — 8 November 2026

The new `midnight-native-filesystem-host` consumes the independent native Browse/Read
ABI. This is a host implementation and qualification milestone, **not completion
of VFS-01**: production main-window composition, complete native capabilities and
normal build/release packaging still need their cutover gates in this initial
record. The later [application cutover](2026-11-08-native-filesystem-cutover.md)
supersedes that initial composition status and records its additional regressions.

The shared native loader validates package and dependency-tree permissions, direct
library placement, symbol origin and callback origin. Windows uses its restricted
DLL search; Linux retains the module with `dlopen` and verifies callback provenance
with `dladdr`. Descriptor ID/version/capabilities must match the strict manifest.
Native packages are trusted code; these checks do not sandbox malicious DLLs/SOs.

The host validates bounded replies, expected response types, directory ordinals,
unique node identities, native address/name agreement, parent containment and exact
read lengths/EOF. Successful calls' C buffers are released even on decode failure.
Failed calls/creation never transfer ownership, even with contradictory nonzero
outputs. Invalid successful buffer replies poison the instance, preventing repeated
resource leaks; its destructor closes remaining plugin state. A failed destroy
quarantines the loaded module until process exit instead of unloading live code.
Cancellation after a successful
resource-producing call closes the newly acquired reader/cursor/mount. Cleanup
does not reuse the canceled operation token. All calls and final resource cleanup
are backend work.

Native path adapters preserve raw Windows UTF-16 or Unix bytes in separate files.
Their tests cover unpaired surrogates/invalid UTF-8 and paths exceeding conventional
MAX_PATH/PATH_MAX lengths without introducing an application path ceiling.
Directory, metadata, volume and capacity traits are implemented. This initial
ABI1.0 checkpoint omitted rich volume labels/details and positive SSD hints;
[ABI1.1 parity](2026-11-08-native-volume-parity.md) subsequently added them before
cutover. Watch/mutation capabilities remain separate migration work.

## Causal regressions and native evidence

The initial host stubs failed all five behavioral tests (`red.log`). The completed
host passes the expanded protocol, cancellation, resource ordering and path tests.
Actual independent native module tests exercise three listing pages (260 entries),
source replacement, exact content, 80 repeated early-stop listings/captures,
missing/duplicate/incompatible packages and module removal after discovery.

Viewer preparation captures every visible sibling's identity. A first retained-
reader design exhausted the real plugin's 64-reader budget (`cache-red.log`).
`CaptureCache` captures sibling revisions through temporary readers and retains
at most 16 active readers in an LRU. Evicted reads reopen only through the plugin
and must match the original length and revision. The real test retains 80 captured
identities, reads them all and then rejects a replaced source after cache eviction.
No native fallback opener is involved. Snapshot objects release their cached reader
when their final owner disappears; in-flight reads keep their own lease until done.

An independent review found three additional failure-path issues, reproduced by
`ownership-red.log`: cleanup of unowned failed-call/failed-create outputs and module
unloading after failed destruction. The ownership/lifetime tests now cover these
contracts. `discovery-red.log` demonstrated that a 64-package partial scan could
hide a duplicate/competing provider; exceeding the scan budget now fails discovery.
`admission-red.log` demonstrated the missing reserved cleanup capacity. The host
admits seven ordinary calls and one serialized cleanup call, holding admission
through reply release. This also reserves reply-buffer capacity under the SDK's
eight-call/16 MiB defaults. Cleanup ignores cancellation; unexpected cleanup errors
poison the instance. Native tests exercise twelve concurrent readers and subsequent
capture reuse to detect leaked resource slots.

Logs are in `_build/goal/native-filesystem-host/`. Windows qualification loads the
independently built DLL from a private QA subtree of the protected runtime; Linux
uses the independently built SO in `/tmp/fm-interaction/_build/native-host-qa`.
Application preferences and registered normal plugin packages are not changed.
Build/cache output remains in the existing `_build` targets. Temporary private
native installations are necessary to test permission checks on native filesystems.

The final native host gates pass **17 unit tests and 2 actual module integration
tests on each OS**, plus strict all-target Clippy. The shared viewer plugin host
gate passes 49 Windows / 36 Linux tests; its 5 separately enabled viewer-module
tests are excluded from those counts. The dependency inventory was refreshed for
both application targets and checked, with unchanged third-party package counts.

Archive worker discovery also delegates native manifests to this host. Its shared
64-package budget counts skipped native packages and rejects overflow rather than
selecting an incomplete catalog. `archive-kinds-red.log` preserves the old rejection
of native manifests; its protected-installation regression covers both coexistence
and overflow.

LiveValidated content remains a revision-checked view of a live file, not an
immutable snapshot or a cryptographic mutation/deletion authorization. Provider
installation is retained until restart. In-process calls cannot forcibly interrupt
a blocked OS call, and malformed native pointers remain a trusted-code ABI contract.
