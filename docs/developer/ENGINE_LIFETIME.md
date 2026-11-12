# Engine ownership and retirement

This is an internal Rust API, separate from the independently versioned plugin APIs.
Reserve cleanup before constructing provider-bearing services:

```rust,ignore
let lifetime = midnight_engine::Engine::lifetime()?;
// Construct native providers only after the fallible worker startup succeeds.
let engine = midnight_engine::Engine::new(lifetime, filesystem, events);
let task = runtime.spawn(engine.run(commands));
```

`Engine::run` synchronously owns the command receiver and complete pinned execution
future before returning `EngineRun`. Dropping an unstarted Engine or unpolled/aborted
run transfers its protected owners to a plain cleanup thread, independently of Tokio.
Normal completion acknowledges retirement of the run bundle. Independently surviving
native work can retain its own protected owner afterward; this acknowledgement is not
a promise that a cancellation-ignoring native call has returned. No UI or executor
destructor joins the cleanup thread. The application shutdown grace remains bounded.

## Capacity and ownership

One engine reserves a domain with 153 payload slots and six reusable waiter tokens:

| Owners | Slots |
| --- | ---: |
| Copy/Move accepted work | 32 |
| Mutation accepted work, including Delete | 32 |
| Serialized intake for each actor | 2 |
| Copy and mutation service contexts | 2 |
| Drag state, accepted drop registry and one serial backlog drain | 3 |
| Engine run, command receiver, shared panel snapshots and shared services | 4 |
| Checksum accepted work | 32 |
| Checksum manifest saves | 2 |
| Checksum serialized intake and service context | 2 |
| Custom Command accepted work and service context | 33 |
| Desktop editor launches and one serialized intake/rejection | 5 |
| Watch shared control, one per pane | 2 |
| Clipboard inbox and one serialized rejection | 2 |

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

## Child tasks and service ownership

The service bundle has its own startup lease, shared through `TaskLifetime`. The run
and migrated Find, preferences, font, desktop, watch and clipboard work retain that
same guarded owner. A task
envelope drops its inner future/closure before releasing the anchor. Nested blocking
work receives its own clone, so cancellation of the parent cannot expose a final raw
service owner. This also covers a native closure cancelled before it starts. Plain
metadata return values need no additional service guard; provider-bearing return
values need their own admitted owner and must not escape through a raw JoinHandle.

Checksum jobs retain the same lease from captured input through preparation, queue
and native execution. Their service context follows both actors and native children.
Custom Commands protect prepared native pins through process admission and launch.
Startup creates two bounded process trackers: eight shared terminal/editor/Custom
Command slots and sixteen independent opener slots. Each reserves before native
launch and adopts its returned child before querying even its ID. They outlive
Tokio, retain the existing process permit through observed exit and cleanup, and
do not kill the child or poll on the shared retirement thread. Waiting failures
retain the existing slot rather than admitting replacement processes. Failed
tracking sends and unconsumed native outputs keep the same guarded child.

Captured editor requests use one owner from intake through native execution or
rejection. The four existing launch permits remain attached through native target
destruction; a serial refusal can occupy one further retirement slot. Retirement
failure stays visible instead of being silently treated as a normal refusal.

Each watch pane shares a guarded control state, including captured mounts retained
by inline-tree scopes. Native subscriptions remain local to their watch worker;
pre-start cancellation or a native panic cannot expose unguarded shared captures.
The clipboard similarly guards its private sixteen-entry inbox, including a native
worker cancelled before first invocation. Open ClipboardSession objects stay on
one native thread through create/pump/Drop. Controller Drop cancels even a pump
waiting on a full result queue. Rejected clipboard requests retire through one
serialized acknowledgement; cancelling that await never cancels resource cleanup.

## Qualification boundary

This protects the outer run, command inbox, Copy/Move/mutation handoffs, shared snapshots,
drag-owned captures and the migrated actor paths above. It is not a blanket guarantee
for every independently spawned engine task. Other actors still require individual
lifetime audits; repeated builder replacement remains a separate configuration-path
limitation. The desktop/watch/clipboard follow-up is described in the
[native-service plan](../superpowers/plans/2026-11-12-native-service-retirement.md). See the historical
[ownership review](../verification/2026-11-12-engine-lifetime-review.md) and the current
[child-task plan](../superpowers/plans/2026-11-12-engine-task-ownership.md).
[LIFE-01](../IMPLEMENTATION_BACKLOG.md#lifecycle-follow-up-found-on-12-november) remains open.
The plugin ABI, operation authority, confirmation and checksum policy are unchanged.
