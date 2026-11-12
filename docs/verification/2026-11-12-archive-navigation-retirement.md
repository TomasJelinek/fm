# Archive navigation native ownership

Baseline `81659a7`; scoped LIFE-01 follow-up for archive navigation. This record
covers archives.rs and its private request/mount helpers; the enclosing batch also
adds guarded archive-size lanes. No plugin or public archive trait ABI changed.

## Executed RED evidence

`_build/qa/archive-navigation-lifetime-20261112/windows-red.log`: **0 passed,
4 failed**, 965 filtered, 0.01 s, after a 1 min 33 s build. All failures observed the
actual native file handle's final cleanup on the executor/caller:

- An unpolled ArchiveAction::List dropped during task cancellation.
- A refused List when the two-worker budget was busy.
- A completed but unconsumed ArchiveListing after its async waiter was aborted.
- A delivered stale listing that was never retained in accepted snapshots.

The fixture reports after closing a real File. The unconsumed-result test gates
native listing until the current-thread runtime stops polling; a later marker on
the sole blocking worker proves completion before aborting the waiter. Cancellation
prevents accepted-root storage from masking the result's last-owner boundary.

A public Engine regression also reproduced raw-input cleanup before Controller
admission: **0 passed, 1 failed**, 975 filtered, 0.35 s, build 19.87 s, in
`windows-stale-red.log`. The test verifies the provider is never entered and shuts
the engine down before asserting cleanup-thread identity.

The first correction guarded only the inner archive dispatch refusal. Final gates
correctly exposed the earlier common navigation high-water refusal: Windows had
969 passing tests and one failure, Linux's archive filter had 23 passing tests and
one failure. Evidence is preserved in
`_build/qa/native-mailboxes-20261112/windows-final-stale-rejection-failure.log` and
`archive-navigation-lifetime-20261112/linux-before-high-water-archives.log` under
`_build/qa/`. Both archive rejection paths now use the same guarded intake. Other
navigation commands are unchanged: inspected VisibleDirectory/Entry values in
Resort contain metadata, not a native mount.

The subsequent targeted Windows run passed **all seven archive retirement tests**
in 0.35 s, build 20.72 s (`windows-high-water-green.log`). This includes the public
stale-command case and cancellation while cleanup admission is still pending.

## Bounded ownership

135 cleanup slots and one startup waiter cover **132 retained mount owners, two
admitted navigation requests and one serialized intake/refusal**. The full engine
ledger, including the two guarded archive-size lanes, is 303 slots and seven waiters. Request retains
its action, original service anchor and existing worker permit through native
execution, async delivery and actual guarded payload destruction. Original services
also have TaskLifetime envelopes around both catalog/navigation task layers.

Every output ArchiveLocation contains a private forwarding ArchiveMount wrapper.
Its native source precedes TaskLifetime and the retained-mount token in guarded
payload order. Native open/restore reserves the token and cleanup lease before
creating a mount; all native intermediate errors remain on the blocking worker.
Completed JoinHandles, aborted sends, accepted events, snapshots and provider
captures can therefore share the wrapper without exposing an unguarded native owner.

The retained-mount budget is derived from existing capability policy:
`2 * (ProviderRegistry::MAX_MOUNTS + 1) + 2 = 132`. It accommodates both 64-mount
pane registries, one current root per pane not yet adopted by a registry, and two
in-flight replacement listings. The registry's existing constant is shared rather
than duplicated. The original 64-target integration remains unchanged; added
coverage retains 64 distinct sources in each pane, admits a current 65th root in
each, observes the original tree-registry refusal, then releases consumers and
successfully enables both current roots.

This is **explicit aggregate retention admission**, not proof that existing owner
counts bound every public caller. Outputs independently retained outside registries
also consume the budget. The host's two active children are a separate policy:
its supervisor releases ACTIVE after process exit, while Mount channel Files and
captured sources may remain in surviving Arcs. Native open/restore process limits
are unchanged. A retirement token remains occupied until actual payload destruction,
including blocked cleanup or a destructor unwind. Refusal never evicts accepted
output to admit another session.

The initial two-retained-mount candidate was too restrictive. The existing Windows
and Linux retained-target integration failed with 17 passing tests, one failure and
one existing ignored test. This was a real admission-policy regression, not a stale
fixture; the original test was preserved. Linux failure evidence remains in
`linux-before-retention-budget-tree_controller.log` in the archive QA directory;
Windows preserves `native-mailboxes-20261112/windows-final-retained-budget-failure.log`
under `_build/qa/`.

A bounded weak registry recognizes exact original-source and exposed-wrapper Arc
identities. Repeated List navigation reuses the same live wrapper and does not
multiply tokens or create nested wrappers. Distinct sources with identical numeric
mount IDs remain distinct. All current ArchiveMount methods forward, including
cooperative reads, borrowed native container locators and persistent member identity.
Cache lookup first uses the canonical wrapper and may then borrow its exact original
source for a size map published before first adoption; no raw strong owner escapes.

Both normal admission and stale dispatcher refusal reserve through the same serial
waiter. Before reservation completes, EngineRun's already guarded suspended future
owns the incoming raw action. The added no-runtime regression exhausts all 135
archive slots, polls that actual envelope to Pending, then drops it and checks native
cleanup occurs off caller. After admission, WorkerOwned handles cancellation without
an active Tokio runtime. Cleanup uses the existing fixed worker, with no per-Drop
thread, executor fallback or unbounded payload queue.

Archive format discovery, native capture/revision validation, saved-address restore,
listing/sort, source revalidation and accepted-root generation/cancellation checks
remain intact. Stale refusal leaves current navigation intact and reports only a
cleanup panic through the existing engine stderr convention.

## Qualification and review

First coherent Windows GREEN (before the two final stale/intake regressions):
22 archive tests passed in 0.02 s; 32 archive-size-filter tests passed in 0.58 s; two tree
lifetime positive controls passed in 0.00 s. Build 18.95 s. Logs:
`windows-green.log`, `windows-green-archive-sizes.log`, `windows-green-tree-lifetime.log`
in the archive QA directory. Its test artifact SHA-256 is
`9e406f0ce18be41eab2c85cc19d876ec71b9793491378f33684397c25b53aa47`.
The initial combined compile failure was only an unmigrated test constructor;
`windows-green-constructor-fixture.log` preserves it.

Final corrected Windows qualification passed the complete engine library suite:
**970 passed, 7 existing ignored, 0 failed**, in 6.55 s. Archive selection, tree
controller and tree filters passed **33 integration tests**, with one existing
ignored case. Strict engine all-target Clippy passed in 19.49 s; the fresh debug
app/helper build passed in 1 min 18 s. Evidence:
`_build/qa/native-mailboxes-20261112/windows-final-tests.log`, `windows-clippy.log`
and `debug-build.log`. The original one-pane retention test and the new two-pane
case both pass; neither registry admission assertion was weakened.

Final corrected Linux tests passed **66 selected unit cases**: archives 24,
archive sizes 32, tree lifetime 2 and engine lifetime 8. Six affected integrations
passed **47 cases**, with one existing ignored tree-controller case. The source
build took 1 min 41 s. Per-filter logs and command/artifact metadata are in
`_build/qa/archive-navigation-lifetime-20261112/linux-final-*.log` and
`linux-final-tests.json`. The final library test artifact SHA-256 is
`c15bca68bfcd7e94f83df4102866494cd89cfad73a51ef1723e79fbe1ef78555`.
Linux strict engine all-target Clippy also passed in 2 min 08 s; the final driver
completed with status 0. `linux-final-clippy.log`, `linux-final-progress.log` and
`linux-final-status.txt` record that completed gate.

Independent final source review accepted the corrected 132-owner/135-slot ledger,
request/native destruction order, canonical registry, cache identity behavior,
both stale refusal guards and EngineRun coverage of a pending admission wait. It
also checked that the two-pane regression retains independent source identities,
preserves tree-registry refusal, and reuses both current roots after consumers
release their captures. The earlier two-mount candidate review is superseded.
No remaining actionable finding was reported; the reviewer ran no tests.

The related completed tree mailbox result was independently shown to hold metadata,
not native mount ownership; this batch does not claim a tree completion defect or
add an unnecessary cleanup slot for it. The queued native-branch shutdown test is
retained as a positive control for the existing Tokio behavior.
