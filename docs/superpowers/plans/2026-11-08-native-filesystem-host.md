# Native filesystem host and application cutover

This executes VFS-01 from the already approved completion goal. The independently
developed Windows/Linux packages and public Browse/Read ABI already exist. The
application must consume that ABI, without a hidden builtin browsing/reader fallback.

1. Reuse the audited platform-specific native loaders through a small trusted-library
   boundary. Validate package permissions, the library basename and symbol origin;
   retain the loaded module for every callback and allocated resource.
2. Add a backend-only native filesystem host. Strict manifests and capability checks
   precede loading. Bounded wire replies, exact response variants, cursor ordering,
   address encoding and read lengths are checked. Resource leases close even on
   cancellation, malformed replies, early enumeration stop or viewer close.
3. Adapt the native provider to DirectoryService/VolumeService/StorageService and
   CapturedFile's ContentSource. Preserve native paths and names losslessly in
   separate Windows/Linux files. Native path services remain an explicit migration
   boundary; this does not claim a generic mutation/watch ABI already exists.
4. Route application browsing and native F3 captures through the installed provider.
   Missing packages produce an unavailable-provider error. Archive worker discovery
   distinguishes native manifests. Supply the matching native package in build and
   private deployment helpers before switching the production composition.
5. Verify protocol failure/resource cleanup with fake transports, then load actual
   independent packages for Unicode, long paths, changed-source and cancellation
   tests. Check absence/duplicate/incompatible packages without weakening trust.
   Run focused format/Clippy/tests and debug application smoke tests; update the API
   documentation and backlog with exact evidence and remaining capabilities.

All filesystem/plugin calls, loading and resource reclamation run on backend
workers. The installed provider stays pinned for the application lifetime; a
restart discovers changed installations. No new third-party dependency is needed.
Root owns the host, loader reuse, app composition and deployment helpers. Concurrent
viewer and move agents own separate files; Cargo execution is coordinated.

## Progress

The shared trusted loader, host protocol consumer, platform path adapters and core
service/content adapters are implemented. Native qualification and evidence are in
`docs/verification/2026-11-08-native-filesystem-host.md`. A real 64-reader quota failure
led to a 16-reader cache for large captured viewer orders, without changing the
viewer worker's captured-order contract.

Application composition now routes Directory/Volume/Storage and native F3 capture
through InstalledProvider, retaining explicit built-in desktop/watch/mutation
services until their independent capability migration. Public ABI minor 1 adds
optional raw volume labels and rich information; old wire operations retain their
layouts. Per-platform packages migrate the existing conservative SSD hints.
Root build helpers now build/install the native package, and release packaging
requires exactly one compatible provider alongside optional archive workers.
CI independently builds native packages with only public API sources, then tests
protected runtime discovery and the application's F3 composition.

Host gates pass 23 unit and two actual-module tests on each OS. Both debug app
builds and strict checks pass; Windows native/software startup, all five viewer
formats through the app capture seam, and Linux headless navigation/basic F3 pass.
Both reject missing native providers. The new Windows debug build is deployed to
a protected registered runtime with unchanged user preferences; see the
[cutover report](../../verification/2026-11-08-native-filesystem-cutover.md).
This plan does not mark VFS-01 complete:
generic provider identities, write/watch capabilities and macOS remain separate
requirements of the overall goal.
