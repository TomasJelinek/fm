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
During a normal shutdown the Copy/Move and mutation actors are drained, for at most
five seconds (`ACTOR_SHUTDOWN`), before the remaining engine tasks are aborted: each job then releases
its native and captured owners through its checked retirement instead of being
dropped by an abort. The same drain waits for the clipboard worker, which runs on one
of the engine's resident worker threads outside Tokio's blocking pool: stopped by the
shutdown, it closes its native session on that thread (which can still finish an X11
transfer or hand copied text over to a Linux clipboard manager), and the run returns
only after that close or the five-second bound. The drain is prompt when the event
consumer either keeps receiving or closes its receiver; the explorer closes it before
sending `Shutdown`, so a blocked event send fails at once. A receiver that stays open
without being drained delays the run by up to those five seconds, after which the
remaining actors are aborted. An aborted Copy/Move actor ends each running job's
batch of steps after the step in flight before its own abort completes: it holds a
twin of each job's batch guard (`worker::Awaited` in `copy_operations.rs`), and
whichever twin is dropped first ends the batch, so no further step starts after the
abort (engine `copy_queue_tests`, `shutdown_during_blocked_link_sync_*`). The explorer's
2.5-second exit grace (`EXIT_GRACE` in
`midnight-explorer/src/main.rs`) can expire first; the dropped run then transfers its
owners to the cleanup thread as described above.

## One-time service configuration

Optional services use `configure_*(&mut self)` to obtain a vacant `ServiceSlot`, then
`slot.install(service)` transfers the owner into the existing startup guard. Admission
accepts no native argument or captured factory. A duplicate returns a metadata-only
`ServiceAlreadyConfigured` error before another service is constructed or transferred.
Installation cannot fail, displace the original owner or reuse the consumed token.
Dropping an unused token leaves the field vacant. An unavailable network provider
simply skips installation; there is no owner-taking clear/replacement operation.

No setter replaces an installed service: replacing a field would bypass the guarded
bundle's destructor and could destroy a last provider owner on the caller. The
application installs each service once (`midnight-explorer/src/main.rs`), and
configuration is finished before `run` consumes the engine. Native provider and
comparator builders are separate APIs.

## Capacity and ownership

One engine reserves a shared retirement domain (`RetirementDomain::try_start` in
`EngineLifetime::new`) with eight reusable waiter tokens (`DOMAIN_WAITERS`) and
**332 payload slots** (`DOMAIN_CAPACITY` in `midnight-engine/src/engine_lifetime.rs`).
The test `lifetime_document_states_the_domain_capacity_and_its_breakdown` reads this
document and fails unless the total and the table below match the code:

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
| Desktop launches for Open (including captured tree Open), Edit and external comparison, Properties, and serialized intake or rejection | 6 |
| Captured shell-menu worker and serialized intake or rejection | 2 |
| Watch shared control, one per pane | 2 |
| Clipboard inbox and one serialized rejection | 2 |
| File List retained jobs and generated outputs | 12 |
| Dynamically discovered network provider | 1 |
| Archive navigation: 132 retained mounts, two worker inputs, one waiting navigation per pane and one intake | 137 |
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
These are child-process admission/tracking limits, separate from the retirement
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
pre-start cancellation or a native panic cannot expose unguarded shared captures. A
panic inside a watch worker's loop unwinds its targets and subscriptions on that
worker, which then restarts in place: at once the first time, afterwards no sooner
than a minute after its previous restart; a stop while it waits drains the shared
state on that worker. A panic outside that loop, and a panic on a platform
notification thread (such as the process-wide Windows watch service thread), are
outside this restart policy and still leave the captures to the guarded control.
The clipboard similarly guards its private sixteen-entry inbox, including a native
worker cancelled before first invocation. Open ClipboardSession objects stay on
one native thread through create/pump/Drop. Controller Drop cancels even a pump
waiting on a full result queue and wakes an idle session wait. Rejected clipboard
requests retire through one serialized acknowledgement; cancelling that await never
cancels resource cleanup. A worker that has stopped is reported as stopped, not as a
busy queue.

The operations and checksum actors do not poll while idle. The operations actor
rescans its polled cancel/pause flags every 50 ms only while jobs are registered;
admission, cancellation and shutdown send it a message. The checksum actor and its
backend wake from shutdown through a notification, and the backend queues for the
serial checksum worker. The clipboard worker blocks on its inbox while no native
owner session is open, and closes an idle session that no longer owns clipboard
content (`ClipboardSession::owns_clipboard`). A Windows session owns the clipboard
only from a file publication until another owner empties it (WM_DESTROYCLIPBOARD);
meanwhile the worker waits in `wait_pending` (`MsgWaitForMultipleObjectsEx` on the
session's event and its owner thread's messages). A queued request or Controller
drop ends that wait through the session's waker; its five-second safety timeout is
not needed for correctness. Over two idle seconds after a clipboard command the worker
does not wake (`measure_idle_wakeups_of_the_real_windows_session`). A Linux session waits in
`poll()` on its eventfd (the waker) and, once it has an X11 owner connection, on
that connection, after serving the events x11rb already queued. It owns CLIPBOARD
(never PRIMARY) from a file publication until a `SelectionClear` from a newer owner
and while an incremental transfer it started is unfinished. Text goes through
arboard, which serves it on its own connection, so the session cannot see that
text's `SelectionClear`: it counts the text as owned until its own file publication
replaces it, so after a text copy the session stays open, idle in that wait.
Measured on Linux (Xvfb) over two idle seconds after a read, a file copy and a text
copy alike, the worker does not wake. The drag/drop workers do not
poll while idle either. The preparation worker sleeps until queued work, a retired
owner or shutdown signals it, and the hover forwarder until a drag event, a changed
hover or refusal report, or shutdown. The native drag actor waits in its open
native session (`FileDragDropSession::wait_pending`): on Windows in
`MsgWaitForMultipleObjectsEx` on the session's wake event and its STA's messages,
which carry other applications' OLE calls into the drop target; on Linux in
`poll()` on an eventfd and the X11 connection, after serving the events x11rb
already queued. A host attach or detach, a drag start or shutdown ends that wait
through the session's waker, so they need no five-second safety timeout. The actor
closes the session on a detach, a new host attach or shutdown, and on Linux also
when the host window's `DestroyNotify` cancels the host lifetime. A lifetime
cancelled without a detach ends no wait (the UI detaches after cancelling): such a
session, on Windows even one whose window was destroyed, refuses drops and closes
within that safety timeout. While an incoming X11
drop's destination is shown or its file list is awaited, the Linux session rechecks
them every 10 ms, because no event marks the UI changing its drop targets. Without a
session (a native Wayland window, or no platform drag service) the actor sleeps
until the controller signals it. Over five idle seconds with an open session the
session wait does not wake (`measure_idle_wakeups_of_an_open_session` of the
platform crates), so the actor wakes only at its safety timeout, and without a
session it does not wake at all. A task of the operations actor that
panics or is aborted fails its own job, so the registry entry cannot hold one of
the 32 slots (and actor exit) indefinitely.

## Image viewer spare shutdown

The [image viewer](IMAGE_VIEWER_API.md) keeps at most one source-free decoder
spare. `ImageSpareRetirement` is an application-owned completion ticket for that
spare; it does not retain the service or wait for active foreground sessions.
`stop_spare()` is memory-only and idempotent: it seals speculative configuration
and refill, cancels a starting spare and marks a ready spare for backend cleanup.
Foreground sessions and ordinary cold admission remain independent.

Normal close drops the preparation handle, requests spare stop,
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
Generated content uses an anonymous spool (`O_TMPFILE` on Linux, delete-on-close on
Windows) in the first usable private root of this user, chosen by the root policy
that all generated-content spools share (`private_spool::private_spool_roots`). On
Linux the candidates start with `$XDG_CACHE_HOME` (or `~/.cache`)
`/midnight-explorer/file-list`, which is disk-backed (small diagnostic reports skip
it without resolving its base, which could block on an unresponsive network home),
then `$XDG_RUNTIME_DIR/midnight-explorer/file-list`. The temporary directory
comes last on both systems (on Windows the per-user `%TEMP%` is the only base): first
a per-user name, `midnight-explorer-<uid>-file-list` (Windows
`MidnightExplorer-<hex of the account name>-file-list`), then a per-process name
ending in `-file-list-<pid>`, which is removed once the last spool in it closes (a
crash leaves it behind). Each root must open as an owner-only directory of this user
reached without links. A root that is refused or lacks anonymous files yields to the
next candidate, and output fails with the last error when none is usable. Links in
the cache, runtime and temporary bases (a linked `~/.cache`, or `/home` linked to
`/var/home` with `$TMPDIR` below it) are resolved first, as the filesystem host does
for its staging bases, while links at and below the `midnight-explorer` names in a
base stay refused. Unlike the cache base, which reports skip, the runtime and
temporary bases are resolved for every content before any root is tried, so a
`$TMPDIR` on an unresponsive network filesystem can block a report that the runtime
root would have served. The cache root fails on an NFS home without `O_TMPFILE`;
sessions without `XDG_RUNTIME_DIR` (su, sudo, ssh, containers) then reach the
temporary directory, whose predictable names another account can occupy. The
private-root check refuses such a directory instead of using it, so output fails
only when the per-process candidate is unusable as well.

Saving to a file keeps one queued acquisition of the mutation gate, so a refilled
Copy queue cannot starve it. While queued, the save holds neither a worker nor a
blocking thread: it waits on its async task, cancellation wakes it without a timer,
and runtime shutdown drops it instead of polling it. It takes a worker once the gate
is granted; worker holders never wait for the gate, and the gate stays held until
native publication returns.

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
diagnostics share that arena explicitly; the engine passes it to each
`DirectoryTree` it creates. No global or thread-local fallback creates hidden
workers.

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
The newest navigation of each pane waits for one of the two workers; a newer request
or a cancel retires a waiting navigation with an Interrupted result. Each accepted
navigation holds one of four navigation shares, returned when the cleanup worker
destroys its input. The serial intake reserves its slot, cancels the pane's previous
navigation (recording the new navigation first, so a superseded result never acts as
current) and then waits for a share; the wait always ends because at most two holders
run and each pane has one current navigation, so one holder is a superseded input on
its way to cleanup. Only stale input retires through the serialized intake
acknowledgement; aborting that wait does not cancel cleanup. Native mounts are adopted before leaving their
worker and exposed only through a canonical guarded proxy. A shared 132-token retention budget
bounds native lifetime, including externally retained listings whose worker already
exited. Its capacity covers the existing two pane registries of 64 mounts each,
two current roots outside those registries and two workers producing replacements.
This explicit aggregate bound is distinct from the navigation shares, the two-worker
admission and host process limits. Arbitrary external output holders also consume this budget; it is
not a proof that those holders cannot retain more outputs. Closed/failed sessions
still occupy their token until final destruction. Saturation reports busy and
accepted content is never evicted.
Only exact raw/proxy identity reuses a guard, not matching plugin IDs or names.

Each pane's snapshot keeps the root of its last archive listing
(`SnapshotState::tree_root`) for tree captures, and that root holds the archive's
session. A successful native listing replaces it. A published Network listing drops it,
and so does every navigation of the pane that opens an archive from a native file
(Enter on an archive file, a saved tab, a Hot Path, Go to file, the recovery of a
failed session), before it takes its slot: a pane that shows no archive keeps no
session alive, also after a refused open. Such a navigation then reserves its native
slot asynchronously (`archives/native.rs`, `NativeAllowance`), holding one of the two
navigation workers meanwhile: at once while a slot is free. Otherwise, when the command
names the session that its tab let go of (`Command::ArchiveNavigate::replaces`), it
waits for that session's teardown, at most 10 seconds and only until the navigation is
cancelled; any other case is refused, and `Event::ArchiveSessionsFull` names the counted
sessions before the failed listing. Dropping the wait withdraws it. The slot moves into
the blocking open and then into the new session, and a failed or unwound open releases
it. The presenter names a session only when no other tab, the other panel or a viewer
holds it (`presenter/archive_sessions.rs`), and the tab lets go of every reference it
has to it (its location, history entries, the listing in flight and a pending
recovery), keeping their addresses, so revisiting one opens the archive again. After
leaving an archive natively or for Network, the tab names the session it left for its
next archive once, since that session's teardown may still run. The panel's offer to
close an archive (`presenter/archive_prompt.rs`) lets the chosen session's tabs go (a
shown one through a navigation to the archive's folder, one in the background at once)
and closes its viewers after the presenter's callback returns. It then opens the
refused archive once, naming the closed session even if a holder that it does not know,
such as a copy, still holds it: a shown tab first finishes its listing, and that wait
is the bounded hand-off above. A tab that moved on meanwhile drops the retry.

A release never destroys a session whose cleanup is unconfirmed (tainted, for example
by a failed source close). Releasing a shared reference of its guard, also from a Copy
whose retirement is already quarantined (`ArchiveMount::release_quarantined`), only
gives up that reference. Exactly one releaser of the last reference (atomic
`Arc::into_inner`) hands the guard's single host reference to the host's tainted
release, which ends the worker and returns its process permit
([filesystem API](FILESYSTEM_API.md)); the source, the retention token and the cleanup
slot stay quarantined until exit. The same holds when the last reference is dropped
inside a Copy, quarantined or not. A last reference released from a quarantined Copy
whose session is not tainted stays retained with its worker. An ordinary drop of the
last reference outside a Copy, such as a closed tab, retires the guard on the archive
cleanup worker like any other session, which also ends the worker.

Owners that hold the guard give up their reference the same way when the session is
tainted: a tree's captured archive location (which holds one reference), tree target
and navigation wrappers, and archive member content. A shared one only loses its
count; the last one passes its guard reference on, also inside a quarantined Copy.
The provider-level owners above them give up the same way when their release fails
or a quarantined retirement skips it (`MountedFilesystem::release_quarantined`, and
`CapturedTransferSources::release_quarantined` for a tree Copy capture held by Copy
input): a tree target wrapper or resolved target whose archive source release failed
hands its target context down, and a target context, mount lease, root seed or tree
Copy capture (with the tree targets it captured) only gives up its count, while the
last one gives up what it holds. A tree Copy job's prepared archive roots and their
children drop directly, also in a quarantined Copy, so their archive references are
given up rather than forgotten. After a failed entry, the remaining entries of a
target context or tree capture (Copy or clipboard) are given up instead of released,
and a captured tree target or selection whose own release step finds the retirement
quarantined is given up rather than forgotten. Giving up never
releases a registration or permit: the last mount lease keeps its provider registration
and mount permit, tree targets and navigations keep the tree registration, and the last
owner of a tree Copy capture keeps its capture permit, all until exit. A give-up runs
synchronously on the backend worker of the failing or skipped release (a Copy's
teardown worker, or the worker of the owner that releases), only gives up counts and
frees plain memory, and never waits: for a tainted session's last reference it only
asks the host to stop the worker, and the host's supervisor ends it and returns its
process permit. A decrement can leave an ordinary owner, such as the tab, as the last
one, which then releases or drops the session as described above; a give-up that
consumes the last reference of a session that is not tainted keeps it retained with
its worker, as a quarantined release does.

An [archive inside an archive](../ARCHIVE_FORMATS.md#archives-inside-archives) holds
its enclosing archive's guard only through its host session's captured source
(`filesystem-host/src/nested.rs`). That session's last release ends its own worker
first, then closes the staged member and releases the enclosing guard like any other
owner, so the enclosing session may end with it. The enclosing session's unconfirmed
cleanup also taints the nested one, which then follows the tainted path above. Its
enclosing folder is reported by `ArchiveMount::parent` (every wrapper forwards it), and
nested sessions use their own allowance of three host processes beside the two archive
process permits.

Limitations: a Copy job whose seal is quarantined before its payload is dropped,
because it observed the taint during the job or because a teardown cleanup step such
as closing the current entry failed, keeps its whole payload with the rest of its
Copy authority, including its archive references. Unless a failed host exchange
already stopped the worker, that session keeps its worker and one of the two archive
process permits until exit (ARCH-05 in the [backlog](../IMPLEMENTATION_BACKLOG.md)).
Member content leased through `MountedLocation::open_content` (compiled for tests only
until a production consumer exists, ARCH-06) does not give up a tainted reference either.

Each archive-size pane reserves one guard for its pending lane before it accepts a
root. Closing marks/cancels the lane; ordinary worker draining remains unchanged.
If a native worker unwinds with queued roots, final lane destruction still retires
native owners outside the executor, retaining its slot through completed cleanup.

Tree branch completions were separately checked: completed rows contain metadata
and no longer retain the consumed native mount. They keep branch admission until
mailbox drain. Queued native branch closures also retain admission through runtime
shutdown and close their native owners on the blocking worker. These are tested
positive controls, not reasons to add redundant completion guards.

## Resident worker threads

Long-lived jobs that need a stable physical thread run on fixed resident worker sets
(`midnight_work_budget::resident`), separate from Tokio's runtime:

- the engine starts 13 resident threads in `EngineLifetime::new`: two each for the
  model, presence, archive and watch lanes, two for drag and drop, one for the
  clipboard and two report readers;
- the viewer starts `midnight_viewer_engine::RESIDENT_WORKERS` (5) and comparison 2
  (`ContentWorkers::start` in `midnight-explorer/src/main.rs`);
- the application's Tokio runtime has 2 async worker threads and at most 16 blocking
  threads.

Each set has a fixed number of threads and adds no payload or waiter capacity to the
retirement domain. At shutdown the engine first stops and aborts its
actors and drains the resident tickets, then shuts down the tree roots, and only
then observes the resident pool's exit for at most 200 ms; expiry leaves the
retirement unconfirmed and grants no cleanup or resource credit. That wait stays
inside the explorer's absolute 2.5-second exit grace and cannot extend it. Viewer and
comparison workers are sealed together after the engine and image-spare waits and
get a 2.5-second grace of their own.

## Scope of these guarantees

This protects the outer run, command inbox, Copy/Move/mutation handoffs, shared
snapshots, drag-owned captures and the actor paths above. It is not a blanket
guarantee for every independently spawned engine task. Its known retained-owner
residuals are LIFE-02, ARCH-05 and ARCH-06 in the
[backlog](../IMPLEMENTATION_BACKLOG.md).
One-time service admission before ownership prevents builder replacement
([configuration record](../verification/2026-11-12-engine-service-configuration.md));
[archive and mailbox ownership](../verification/2026-11-12-native-mailboxes.md) and
the [ownership review](../verification/2026-11-12-engine-lifetime-review.md) are the
dated evidence. The plugin ABI, operation authority, confirmation and checksum policy
are unaffected.
