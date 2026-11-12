# Explicit report arena plumbing

Status: implemented and scoped qualification passed; see the dedicated verification record.

Every producer receives the engine startup `ReportArena` explicitly. A report handle may
outlive its scanner, controller, tree session, or event delivery; its native storage
therefore belongs to the shared arena rather than the producer's asynchronous task.
The arena implementation and storage tests are owned separately in `size_warnings`.

- [x] Enumerate native folder scans, archive scans, per-occurrence tree failures,
  folder-presence probes, tree listing reports, and terminal tree diagnostics.
- [x] Pass the arena through existing controller service contexts and native workers.
- [x] Require an explicit arena in the public `DirectoryTree::new` constructor; never
  start a cleanup worker implicitly per tree. The engine lifetime exposes its factory.
- [x] Keep selection aggregation and composed diagnostics unchanged: they retain
  existing reports and create no native report storage.
- [x] Migrate dedicated fixtures to explicit test arenas and preserve all existing
  scan validation, cancellation, immutable snapshots, and complete diagnostic text.
- [x] Add focused producer coverage for retained diagnostic output after its producer
  and factory handles are released, including late tree diagnostics.
- [x] Run scoped producer/tree tests and strict engine lint in coordinated Cargo leases.

Shared integration belongs to the root agent: startup arena/retirement lease, public
export, and `lib.rs` controller calls. No plugin ABI or report text format changes.
Existing archive scan descriptors will group address/ancestry fields to keep factory
injection within the source style's argument-count limit. All factory creation remains
fallible at startup, before provider-owned work is admitted; report construction itself
is metadata-only and first-record native storage admission remains in the arena.
