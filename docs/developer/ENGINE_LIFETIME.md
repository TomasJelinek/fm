# Engine ownership and retirement

This is an internal Rust API, separate from the independently versioned plugin APIs.
Reserve cleanup before constructing provider-bearing services:

```rust,ignore
let lifetime = midnight_engine::Engine::lifetime()?;
// Construct native providers only after the fallible worker startup succeeds.
let mut engine = midnight_engine::Engine::new(lifetime, filesystem, events);
let slot = engine.configure_find()?;
// Build/transfer the optional service only after admission succeeds.
slot.install(find_service);
let task = runtime.spawn(engine.run(commands));
```

`Engine::run` synchronously owns the command receiver and complete pinned execution
future before returning `EngineRun`. Dropping an unstarted Engine or unpolled/aborted
run transfers its protected owners to a plain cleanup thread, independently of Tokio.
Normal completion acknowledges retirement of the run bundle. Independently surviving
native work can retain its own protected owner afterward; this acknowledgement is not
a promise that a cancellation-ignoring native call has returned. No UI or executor
destructor joins the cleanup thread. The application shutdown grace remains bounded.

## One-time service configuration

Optional services use `configure_*(&mut self)` to obtain a vacant `ServiceSlot`, then
`slot.install(service)` transfers the owner into the existing startup guard. Admission
accepts no native argument or captured factory. A duplicate returns a metadata-only
`ServiceAlreadyConfigured` error before another service is constructed or transferred.
Installation cannot fail, displace the original owner or reuse the consumed token.
Dropping an unused token leaves the field vacant. An unavailable network provider
simply skips installation; there is no owner-taking clear/replacement operation.

The former consuming `with_*` setters were removed because field replacement bypassed
the guarded bundle's destructor and could destroy a last provider owner on the caller.
This internal Rust API migration does not change plugin ABIs, service traits, startup
capacity or runtime behavior. Existing application composition installs each service
once. Configuration is finished before `run` consumes the engine. Native provider and
comparator builders are separate APIs and are unchanged by this migration.

## Capacity and ownership

As of 17 November 2026, one engine reserves one shared retirement domain with
330 payload slots and eight reusable waiter tokens:

| Owners | Slots |
| --- | ---: |
| Copy/Move accepted work | 32 |
| Mutation accepted work, including Delete | 32 |
| Serialized intake for each actor | 2 |
| Copy and mutation service contexts | 2 |
| Drag state, accepted drop registry and one serial backlog drain | 3 |
| Engine run, command receiver, shared panel snapshots and shared services | 4 |
| Shared comparator policy store, guarded from startup | 1 |
| Accepted comparator policy pins, acquired dynamically under the two comparison credits | 2 |
| Checksum accepted work | 32 |
| Checksum manifest saves | 2 |
| Checksum serialized intake and service context | 2 |
| Custom Command accepted work and service context | 33 |
| Captured desktop Open/Edit/external-comparison launches, Properties, and one serialized intake/rejection | 6 |
| Captured shell menu worker and one serialized intake/rejection | 2 |
| Watch shared control, one per pane | 2 |
| Clipboard inbox and one serialized rejection | 2 |
| File List retained jobs and generated outputs | 12 |
| Dynamically discovered network provider | 1 |
| Archive navigation: 132 retained mounts, two requests and one intake | 135 |
| Archive-size pending state, one per pane | 2 |
| Shared diagnostic report arena | 1 |
| Native tree Rename/Metadata captures, sharing the four tree-action permits | 4 |
| Retained Find View/Edit/Open captures | 16 |

The eight waiter tokens serve Copy, mutations, drag backlog, archive navigation,
clipboard rejection, checksums, shell-menu intake and desktop intake. They are
reusable reservation metadata; their payload reservations are included in the
table. Cloning the shared `RetirementDomain`, including `comparator_policy_pins`,
creates neither another domain nor another payload slot.

Comparison admission is separately limited to two counter-only session credits.
Each accepted policy pin reserves one of the two dynamically budgeted payload
slots in the same retirement domain and retains that same credit through actual
policy destruction. Cloning a credit does not admit another comparison. These
two credits are not two additional payload slots. External comparison shares the
existing four desktop Open/Edit launch permits; its captured pair uses one
guarded payload owner, not one slot per file. The two policy-pin slots are
separate from the six desktop capture slots.

Native drag admission uses Copy's existing 32 permits. It does not add another queue.
A reservation is acquired before accepting owned payloads and remains occupied until
actual destruction finishes, including blocked native cleanup. No thread is started
per destructor and no fallback executes cleanup inline on an executor.

Copy retains payload, terminal receipt, mutation permit and capacity in one guard.
Prepared mutations keep native pins before captured provider owners in destruction
order. Guarded state remains owned while waiting for output delivery, confirmation,
predecessors and mutation admission. Moving a raw value out of a guard is worker-only;
its lease must remain until destruction or immediate transformation into a new guard.

The guarded command inbox closes admission and drains until previously acquired send
permits are released or deliver their command. Late admitted commands therefore retire
on the same worker. A producer that holds such a permit forever can stall this bounded
cleanup domain; it cannot grow replacement capacity or block the UI's Drop call.
Similarly, an arbitrary blocked provider destructor has no guaranteed completion time.
One unwinding destructor is isolated; Rust's double-panic/process-abort rules still apply.

## Prepared Rename disposal boundary

The implemented [core/host Rename contract](NATIVE_FILESYSTEM_API.md#prepared-rename-corehost-lifecycle)
adds a finite `RenameResource::finish()` acknowledgment and an owned
`RenamePrepareFailure.retained` carrier. Engine integration remains unimplemented;
current destructor ordering/retirement acknowledgment does not implement that
handshake.

The caller must retain the prepared owner or prepare-failure carrier together
with its full job, captured provider owners, mutation permit and outer retirement
slot. Release that chain only after `Closed`; `Executing` retains it through
actual work, and `Unconfirmed` quarantines the whole chain. Dropping a host owner
alone cannot retain caller-owned captures/gates. This contract adds no cleanup
pool or retirement capacity and does not establish engine/UI cutover.

## Child tasks and service ownership

The service bundle has its own startup lease, shared through `TaskLifetime`. The run
and migrated Find, preferences, font, desktop, watch, clipboard, File List, shell,
chooser, volume, listing and folder-size service work retain that same guarded owner.
A task envelope drops its inner future/closure before releasing the anchor. Nested blocking
work receives its own clone, so cancellation of the parent cannot expose a final raw
service owner. This also covers a native closure cancelled before it starts. Plain
metadata return values need no additional service guard; provider-bearing return
values need their own admitted owner and must not escape through a raw JoinHandle.

Checksum jobs retain the same lease from captured input through preparation, queue
and native execution. Their service context follows both actors and native children.
Custom Commands protect prepared native pins through process admission and launch.
Startup creates two bounded process trackers: eight shared terminal/editor/Custom
Command/external-comparison process slots and sixteen independent opener slots.
These are child-process admission/tracking limits, separate from the 330 retirement
payload slots, the six desktop capture slots and the two comparison credits.
Each tracker reserves before native launch and adopts its returned child before
querying even its ID. They outlive
Tokio, retain the existing process permit through observed exit and cleanup, and
do not kill the child or poll on the shared retirement thread. Waiting failures
retain the existing slot rather than admitting replacement processes. Failed
tracking sends and unconsumed native outputs keep the same guarded child.

Captured desktop requests have a six-slot payload budget: four shared
Open/Edit/external-comparison launch owners, one Properties owner and one
serialized intake/rejection owner. Native captures retain their associated
permits and payload leases through actual destruction. A serial refusal retains
its existing owner until cleanup is acknowledged. Retirement failure stays
visible instead of being silently treated as a normal refusal.

Each watch pane shares a guarded control state, including captured mounts retained
by inline-tree scopes. Native subscriptions remain local to their watch worker;
pre-start cancellation or a native panic cannot expose unguarded shared captures.
The clipboard similarly guards its private sixteen-entry inbox, including a native
worker cancelled before first invocation. Open ClipboardSession objects stay on
one native thread through create/pump/Drop. Controller Drop cancels even a pump
waiting on a full result queue. Rejected clipboard requests retire through one
serialized acknowledgement; cancelling that await never cancels resource cleanup.

## Image viewer spare shutdown

The [image viewer](IMAGE_VIEWER_API.md) keeps at most one source-free decoder
spare. `ImageSpareRetirement` is an application-owned completion ticket for that
spare; it does not retain the service or wait for active foreground sessions.
`stop_spare()` is memory-only and idempotent: it seals speculative configuration
and refill, cancels a starting spare and marks a ready spare for backend cleanup.
Foreground sessions and ordinary cold admission remain independent.

Normal close stops and drops the preparation controller, requests spare stop,
then closes viewer windows. After the event loop exits, including startup failure,
the application stops the spare and awaits `wait()` outside UI callbacks before
shutting down Tokio. Success follows any late spare creation, actual native
cleanup and normal owner exit. Keep the ticket through runtime cleanup and fatal
handling; a UI destructor does not wait for this acknowledgment.

The application also catches unwinding around the window attempt while keeping
the runtime and retirement ticket outside that scope. It requests spare stop,
awaits the owner and uses bounded runtime shutdown before resuming the original
panic payload. This prevents retained UI service clones from leaving an idle
image owner in Tokio's unbounded runtime destructor. A caught application panic
never enters software-renderer startup retry.

`OwnerFailed` is terminal but leaves cleanup unconfirmed. Preserve residual
ownership, still run runtime cleanup and suppress software-renderer startup retry.
A retry is allowed only after successful spare retirement and runtime cleanup.
This fence does not drain active foreground sessions or establish a general
natural auxiliary-process lifetime guarantee.

On Windows, worker creation includes the configured resource-limited Job in
`PROC_THREAD_ATTRIBUTE_JOB_LIST`. Assignment occurs during `CreateProcessW`;
the suspended main thread resumes only after its owned process guard is installed.

## Generated File Lists and network discovery

File List uses a separate enforced six-token retention bound, accounting for current
jobs, superseded native work and generated viewer/clipboard content retained outside
the job map. Every token reserves two cleanup leases before acceptance: one job and
one possible generated spool. Both guarded owners retain the same token through
actual destruction. Four visible jobs and two workers remain separate limits; worker
admission spans native execution and reply delivery. Refused admission leaves the
previous generation intact. Native destination pins and captured size reports stay
inside the guarded job, including confirmation queued before native execution.

Network discovery runs once and reserves one startup lease before opening the native
plugin. Its successful provider is adopted before leaving the native discovery call.
The write-once result, navigation tasks and controller share only that guarded owner;
no raw provider Arc is exposed for cloning. The existing two-request budget stays
independent. A future provider retry/replacement would need its own admission design.

PathPicker Drop cancels its native chooser/reply token, including a full live output
queue. Original service anchors cover both task layers in volume discovery/details,
listing, storage/size queries, shell menus, locale/network settings and folder probes.
Platform choosers still cannot always be forcibly interrupted. Newly created report
spools use a separate shared arena rather than relying on original service anchors.

## Diagnostic report storage

Startup reserves one additional retirement slot and starts one fallible plain report
cleanup worker before any report file can be created. Folder-size, presence and tree
diagnostics share that arena explicitly. Standalone `DirectoryTree` callers clone
`EngineLifetime::report_arena()` before handing the lifetime to an engine and pass it
to `DirectoryTree::new`. No global or thread-local fallback creates hidden workers.

Each report keeps its own append file and failure state. Creation reserves a reusable
registry node before opening native storage. Writers and immutable text prefixes
share that node's retention token and release raw references before the token. Its
last drop only links the already allocated node into a queue and wakes the worker;
it does not close a file, join a thread, allocate a queue node or require Tokio.
Unused reservations return their metadata-only nodes immediately, so repeated file
creation failures cannot grow an empty queue behind blocked native cleanup.
Actual native destruction occurs outside the registry lock. A node becomes reusable
only after cleanup finishes. Registry memory follows the concurrent report high-water
mark; there is no arbitrary report-count quota or shared append-file failure scope.

A stopped report worker rejects new reservations before native creation. Existing
tokens keep the guarded registry alive; after its final owner disappears, the normal
retirement worker destroys remaining storage. This exceptional fallback can delay
cleanup while other reports remain retained, but never transfers final native cleanup
to the UI or executor. Individual destructor unwinds are isolated; Rust process-abort
and double-panic behavior still apply.

## Archive navigation and queued size work

Archive navigation guards accepted input through native work and reply delivery.
Busy or stale input retires through one serialized intake acknowledgement; aborting
that wait does not cancel cleanup. Native mounts are adopted before leaving their
worker and exposed only through a canonical guarded proxy. A shared 132-token retention budget
bounds native lifetime, including externally retained listings whose worker already
exited. Its capacity covers the existing two pane registries of 64 mounts each,
two current roots outside those registries and two workers producing replacements.
This explicit aggregate bound is distinct from the two-request admission and host
process limits. Arbitrary external output holders also consume this budget; it is
not a proof that those holders cannot retain more outputs. Closed/failed sessions
still occupy their token until final destruction. Saturation reports busy and
accepted content is never evicted.
Only exact raw/proxy identity reuses a guard, not matching plugin IDs or names.

Each archive-size pane reserves one guard for its pending lane before it accepts a
root. Closing marks/cancels the lane; ordinary worker draining remains unchanged.
If a native worker unwinds with queued roots, final lane destruction still retires
native owners outside the executor, retaining its slot through completed cleanup.

Tree branch completions were separately checked: completed rows contain metadata
and no longer retain the consumed native mount. They keep branch admission until
mailbox drain. Queued native branch closures also retain admission through runtime
shutdown and close their native owners on the blocking worker. These are tested
positive controls, not reasons to add redundant completion guards.

## Qualification boundary

This protects the outer run, command inbox, Copy/Move/mutation handoffs, shared snapshots,
drag-owned captures and the migrated actor paths above. It is not a blanket guarantee
for every independently spawned engine task. Other actors still require individual
lifetime audits. Repeated builder replacement is now prevented by one-time service
admission before ownership; see the [configuration qualification](../verification/2026-11-12-engine-service-configuration.md).
The desktop/watch/clipboard follow-up is described in the
[native-service plan](../superpowers/plans/2026-11-12-native-service-retirement.md);
[remaining native results](../superpowers/plans/2026-11-12-remaining-native-results.md)
covers File List, dynamic network discovery and subsequent service hooks, with
[qualified results](../verification/2026-11-12-remaining-native-results.md). See the historical
[ownership review](../verification/2026-11-12-engine-lifetime-review.md) and the current
[child-task plan](../superpowers/plans/2026-11-12-engine-task-ownership.md).
[Archive and mailbox qualification](../verification/2026-11-12-native-mailboxes.md)
records the next ownership boundaries.
[LIFE-01](../IMPLEMENTATION_BACKLOG.md#lifecycle-follow-up-found-on-12-november) remains open.
The plugin ABI, operation authority, confirmation and checksum policy are unchanged.

## Accepted private resident-worker follow-up

The accepted private resident-worker design is **13 Engine /5 Viewer /2 Compare**
resident threads, keeping2 async/16 transient blocking. These are planned domains,
not implemented threads, process measurements or new payload/waiter capacities.
Preserve worker affinity, distinct external ownership, checked retirement and current
window/session credits. The later old-scheduler run `production-pool-7e458befac` is genuine production RED.
With the original2 async/16 blocking threads,13-resident readiness and four windows,
the prepared-save callback had not entered and the second Compare was incomplete
at the original20,000ms pressure boundary. Closing two Viewers recovered the same
prepared save (exactly one callback, coherent raw publication) and the same Compare.
Strict cleanup and exclusive capture-reader reopen checks completed before the
intended progress-failure assertion; no owned processes/monitor errors remained.
All3,941 guarded identities were unchanged; the unconsumed4917→F5 test-fixture
exception is separately recorded. Raw test time was23.02s. This establishes the
old-source failure and same-work recovery, not migration GREEN, an unrecoverable
deadlock or a performance result. The earlier missing-plugin setup failure remains
historical and distinct. The old-source production-pool attempt `production-pool-27587e6553` reached real
watch/report and13-nonwindow readiness, then failed Viewer setup because no compatible
Basic Viewer plugin was installed beside the test executable. The raw test ended in
0.23s before four-window pressure, the20-second pressure deadline, pre-rescue counters
or recovery. Cleanup/exclusive-open checks preceded the failed outcome assertion;
remainingOwned[] and source/harness identities were unchanged. This is a setup
failure, not a production-starvation reproduction or a progress pass. Production
implementation promotion stays held.
The catalog-only host constructor is canonical; the old scheduler and dependency
pins are unchanged. Its six selected catalog/startup checks actually recorded
**5 passes /1 fixture-precondition failure**: constructing a dot alias had already
normalized the path, so the test failed before the product validator. The exact
raw-OsString test-only repair is canonical `f5d09b3b…`; replay is pending. Root's
44,433-byte `eb391200…` consumer remains canonical with unchanged registration
parent `e3a869…`. The later genuine pressure RED exercised qualified catalogs through this constructor;
the exact F5 alias test replay and migrated-consumer acceptance remain open. The
prior missing-plugin setup failure is a separate historical attempt.

Archived private common-worker round-two QA passed **13 default-feature-off tests**
and **24 resident-feature-on tests**. The latter follows exactly three unit-channel
type annotations (18 test-only bytes); production `resident.rs` remains
`60908d11119f…`. Retain the original feature-on test-build E0282 failure separately.
These are isolated immutable round-two snapshots, not final round-four, migrated
consumer, native, performance or production qualification. In particular, the
known repeated-panic-retention finding is not closed by these passes.
The faithful native fixture4/0 is separate and cannot qualify this production repair.

The newer immutable resident common round-four checkpoint `6c04f591…` passed
**29 tests /0 failures**, with13 default tests filtered out rather than rerun.
Production source is `31178fbd…`, tests `fb306359…`; raw runtime was0.20s and
compilation completed in4.05s with no warnings. This is separate from archived
round-two13/24 evidence. It does not qualify Engine/Viewer/Compare integration,
native cleanup, performance or the full resident migration.

The Architect accepted the Engine's additional200ms pool-exit observation only
after existing stop/abort controls and actual resident-ticket drain, followed by
tree-root shutdown. It stays inside Explorer's original shared absolute2,500ms
deadline and cannot reset/extend it or shorten actor observation. Expiry means
unconfirmed retirement and detaches the checked waiter; it grants no cleanup or
resource credit. The original supplied transient runtime stays time-enabled and
driven. This is an accepted private contract, not integration/runtime evidence.
