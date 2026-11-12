# Archive navigation ownership

Baseline `81659a7`. Scope: archives.rs and its private helpers/tests; parent owns
startup ledger and dispatcher integration. Public/plugin ABI remains unchanged.

- [x] Reproduce unpolled input, busy refusal, completed/unconsumed listing and
      delivered-but-unretained listing native cleanup on the executor.
- [x] Reserve serialized intake before consuming an incoming ArchiveAction; guard
      accepted requests across catalog wait, native execution and result delivery.
- [x] Enforce the derived 132 retained native mount tokens through actual destruction.
      The host two-child quota alone is insufficient: supervisor exit can release
      it while channel Files and captured-source owners still remain in Mount Arcs.
- [x] Forward ArchiveMount through one canonical guarded wrapper per live exact
      source identity, including every optional/native/cache/read method.
- [x] Preserve published-size lookup across first raw-source adoption and subsequent
      canonical navigation; never equate sources using numeric mount IDs alone.
- [x] Keep two native worker permits through request cleanup, including cancellation.
      No accepted output is evicted to admit new work.
- [x] Test saturation, alias reuse, source replacement, stale/cancelled listing,
      destructor blocking/panic, and original service/native result lifetimes.
- [x] Complete final Windows/Linux tests and strict lint, and record actual evidence.

Approved capacity: **135 cleanup slots and one waiter**: 132 retained mount owners,
two admitted request owners and one serialized intake/refusal. The mount budget is
`2 * (ProviderRegistry::MAX_MOUNTS + 1) + 2`: both existing 64-mount pane registries,
one current root per pane outside those registries, and two in-flight replacement
listings. The existing registry constant is shared to prevent policy drift. The
initial two-retained-mount candidate failed the unchanged 64-target integration
contract and is superseded; it is not the implemented admission policy.

Every newly created mount reserves its native token and cleanup lease before
Open/restore creates it. Accepted outputs and provider captures share the same
canonical guarded mount. This remains an explicit aggregate bound: independently
retained outputs also consume it, including failed/closed sessions. Refusal preserves
accepted output; nothing is evicted to admit replacement work. Native host process
admission remains its existing independent two-child limit. No per-Drop thread,
inline fallback or unbounded queue.

Integration: `archives::Retirement { domain, intake }` and existing TaskLifetime
are new leading Controller constructor arguments; `navigate` becomes async and is
awaited by the existing serial dispatcher. Incoming action ownership during intake
wait remains covered by EngineRun. Parent integrates both shared files.

Both earliest and inner stale ArchiveNavigate refusals use the same serialized
intake guard; cancellation before lease admission remains inside EngineRun.

Evidence: [archive navigation retirement](../../verification/2026-11-12-archive-navigation-retirement.md).
