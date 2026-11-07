# Internal provider registry verification — 2026-11-07

Scope: internal host identities, stored-address types, linked cancellation,
revocable Browse/Read leases, and transitional native/archive adapters. No public
plugin ABI, production panel route, selection model or native write capability
was replaced. This is partial VFS-01 progress; TREE-01 and NET-01 remain pending.
See [the contract](../developer/PROVIDER_REGISTRY.md).

## Tests and review

- Windows integrated Cargo provider tests: **33 passed**,
  `_build/goal/vfs/final-providers-windows.log`.
- Linux integrated Cargo provider tests: **33 passed**,
  `_build/goal/filters/root-vfs-linux.log`.
- Full core tests: Windows **94 passed, one intentionally ignored native argv
  child fixture**; Linux **92 passed**. Logs `final-core-windows.log` in the VFS
  artifact directory and `final-core-linux.log` in the filter artifact directory.
  These include four identity/address tests and one linked-cancellation test.
- Final Windows core/common/native/engine all-target strict Clippy passed after
  module formatting, in `final-backend-clippy-windows.log`.
- Workspace rustfmt check passed in `workspace-format.log`.

The provider tests include actual native file capture/readback and detection of
later source modification, plus fake-provider routing, cancellation and resource
boundaries. Archive adapter tests use captured fake archive mounts, not a fresh
external-worker conformance run. This does not prove the whole production
navigation → selection → F3 migration, which is not yet implemented.

Behavioral RED → GREEN runs retained under `_build/goal/vfs/` established the
initial missing registry and adapter behavior. Review also found and fixed:

1. Provider ownership could end before retained content/mount destructors. A
   sentinel test now requires content → mount → provider destruction, even after
   explicit revocation.
2. Reissuing a token with another kind could change earlier references. Conflicting
   kinds now fail transactionally; neither a partial listing nor parent response
   can rewrite an issued file into a directory.
3. In-flight and retained mounts lacked aggregate admission. The registry now
   counts both against 64 live leases and releases capacity only after the last
   consumer drops.
4. Direct returned content sources needed the same bounded-read checks as viewers.
   The wrapper now reuses `CapturedFile`, including zero/overflow/oversized/short
   read rejection and revision checks, instead of duplicating the broker.
5. A synthetic native `..` listing entry could be issued as a child. Native
   listings now require a real basename consistent with the raw child path.
6. A stored native address could allocate before the path-cache budget rejected
   it. Codec-derived admission now occurs before decoding; native encoding checks
   its raw charge first and no longer performs a redundant encode/decode round trip.

Independent review reports are retained in `_build/goal/transfers/vfs-readonly-review.md`
and the archive report in `_build/goal/vfs/archive-report.md`.

## Remaining qualification

The native path-cache budget is per mount, while the registry bounds mount count.
It is not a global RSS cap; before production migration, align aggregate cache
admission with the application's shared budgets. macOS has neither an implemented
native adapter nor a runner here. Public independent filesystem packages,
capability-based operations, persistence migration, watch routing and the native
cutover still require their specified implementation and conformance tests.
