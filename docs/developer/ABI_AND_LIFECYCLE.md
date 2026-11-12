# ABI and lifecycle contract

This page describes native ABI 1.0 and public SDK 0.1.0. Read the
[architecture and trust model](README.md) before implementing a native consumer.
The [generated C header](../../api/midnight-plugin-api/include/midnight_plugin_v1.h)
is the cross-language layout reference. Rust names below come from
[`api::abi`](../../api/midnight-plugin-api/src/abi.rs); C types use the `Midnight`
prefix, for example `MidnightPluginApi` and `MidnightEntry`.

## Entry and function tables

The sole plugin export is:

```c
MidnightStatus midnight_plugin_entry_v1(MidnightPluginApi *output, uint32_t capacity);
```

The caller provides aligned, writable memory of the stated capacity. The plugin
must never write beyond it. The first 16 bytes are `Header`: `struct_size`,
`abi_major`, `abi_minor`, `kind`, `minimum_host_minor`, `reserved`. Do not read the
header until capacity covers it, or a function-table field until the required
prefix fits both capacity and `struct_size`. ABI major must equal 1, required host
minor must be at most 0, kind must match, and reserved fields must be zero.
Larger optional tails may be ignored; a different major is incompatible.

`PluginApi` supplies immutable UTF-8 `id`/`version` bytes and all seven required
callbacks: `create`, `submit`, `cancel`, `drain`, `release_buffer`,
`begin_shutdown`, `destroy`. ID and version must match `plugin.toml` byte-for-byte.
Metadata remains valid for the loaded library's lifetime. `create` receives a
validated `HostApi`, borrowed configuration bytes and an instance-handle output.
The current application passes empty configuration. A successful instance handle
is nonzero and belongs only to its producing plugin library.

`HostApi` provides `schedule_work`, `cancelled`, `submit_service`, `notify_ready`,
`retain_buffer`, `release_buffer` and `diagnostic`, plus its opaque context. The
current application's diagnostic callback deliberately does not log native
message content. Do not depend on it for user-visible errors; emit `Failure`.
All table callbacks are required at ABI 1.0.

No Rust allocator, `String`, `Vec`, trait object, exception, Tokio object or Slint
object crosses the C boundary. Callbacks are `extern "C"` and must contain
unwinding panics/exceptions. SDK panic guards cover ordinary unwinding, including
cleanup at audited boundaries; they cannot recover from process aborts, undefined
behavior or double panics during unwinding. Use `panic = "unwind"` for the SDK's
panic containment behavior.

## Ownership rules

| Value | Lifetime and release rule |
| --- | --- |
| Input descriptor/table/request pointer | Borrowed for the ABI call; caller guarantees valid mapped memory and alignment |
| Request payload `ByteSlice` | Borrowed for `submit`; SDK copies bounded CBOR into owned `Request` |
| Request `content: Buffer` | Host-owned immutable bytes; successfully retain before asynchronous use, then release each retain |
| SDK `Content` | Clones share one immutable native lease; the last clone performs its one matching release |
| Event `payload: Buffer` | Plugin-owned encoded payload; host releases its nonzero ID through that instance |
| Event `content: Buffer` | Independently owned raw bytes; release its nonzero ID separately from payload |
| Accepted `Work` | Host owns exactly one `drop(context)`; it may call `run(context)` once, or cancel before running |
| Rejected `Work` | Caller still owns it; host calls neither `run` nor `drop` |
| Rejected SDK `send` | `SendError` returns both `output` and `continuation`; neither is scheduled by that rejection |

`ByteSlice` is pointer + `u32` length + zero reserved field. ID zero represents an
empty buffer only; an empty buffer can still have a nonzero ID and then requires
release. A nonempty buffer needs a nonnull pointer. Never free a plugin buffer
with the host allocator, release it through another instance, or dereference a
released lease. Payload and raw content are separate buffers, not a CBOR array
of byte integers. The SDK accounts for empty allocations as well as byte totals.

`drain(instance, capacity, events, written)` accepts at most 64 output slots in
the SDK. `written` advances after each event: even when a later iteration fails,
every already-written buffer still needs release. Invalid handles must not free
another instance's allocation. Native pointer checks validate metadata, not
whether arbitrary addresses are mapped or trustworthy.

## Request flow

1. Discover the manifest, load the native library, validate the descriptor, then
   call `create`. The current host creates one plugin instance per viewer session.
2. Submit `REQUEST_OPEN` with `messages::Open`. The plugin records reader,
   revision, requested mode, session and generation, and emits `Output::opened`.
   `Opened` is ordinary output; finish the open request with a terminal too.
3. Submit `REQUEST_VIEWPORT` or `REQUEST_SEARCH` as needed. Each request has its
   own nonzero ID and the session/generation identity in the ABI envelope. Use
   bounded reads and continuations to generate zero or more data events.
4. End each accepted request exactly once with Completed, Cancelled or Failed.
   Rejected admission owns no accepted request and produces no terminal event.
5. `REQUEST_CLOSE` is the public logical session-close request, with an empty
   payload permitted. A plugin handling it drops that session's caches/contexts
   and completes the close request. Instance shutdown is a separate operation.

The current application closes a viewer by calling `begin_shutdown` and then
draining/destroying the instance, without submitting a logical `REQUEST_CLOSE`
first. Therefore make ordinary Rust destruction release all plugin state as well.
Do not retain self-owned contexts indefinitely waiting for a close message: that
can prevent quiescence and therefore prevent destruction.

Request IDs, service IDs, session IDs and generations are opaque `u64` values.
Service transaction IDs are a separate namespace from user request IDs, not
aliases of them. The SDK keeps internal tickets so an older retained context
cannot corrupt a reused request ID. ID allocation fails on exhaustion instead
of wrapping. Plugin session state must reject stale generations and callbacks
must verify that captured state remains live before modifying it.

## Rust author interface

Implement `Viewer: Send + Sync + Sized + 'static`:

```rust
const ID: &'static str;
const VERSION: &'static str;
fn create(configuration: &[u8]) -> Result<Self, api::abi::Status>;
fn handle(&self, request: Request, context: Context);
```

Use `midnight_plugin_sdk::export_viewer!(YourViewer)` from a `cdylib` crate.
`create` constructs bounded state; defer file/parsing work to accepted requests.
`Request::kind()` selects a schema and `Request::decode::<T>()` performs wire and
model validation. `Context::{request_id, session, generation}` expose identity.

The actual continuation aliases and operations are:

```rust
type Work = Box<dyn FnOnce(Context) + Send + 'static>;
type ReadContinuation =
    Box<dyn FnOnce(Context, api::messages::ReadReply, Content) + Send + 'static>;
// Methods on Context; Result values must be handled:
// read(&self, request: ReadAt, continuation: ReadContinuation) -> Result<(), Status>
// send(&self, output: Output, continuation: Work) -> Result<(), SendError>
// complete(&self) -> Result<(), Status>
// fail(&self, code: u32, message: &str) -> Result<(), Status>
// is_cancelled(&self) -> bool
```

`Output::{opened, text, binary, document, search_match, progress}` validate models
before encoding. `binary` additionally takes owned `Box<[u8]>` and verifies its
length equals `source.length`. Complete in the send continuation, after delivery
queue admission, rather than immediately after scheduling a send. Dropping the
last unfinished context produces failure; dropping a finished context is still
necessary for quiescence. The [quick start](QUICK_START.md#minimal-asynchronous-example)
contains a compile-checked read/send example using these exact methods.

## Scheduling, backpressure and cancellation

Entry methods for one instance are serialized by the backend dispatcher. Work
callbacks and host callbacks run on backend workers, never the UI thread. Host
callbacks must tolerate concurrent backend calls. Do not synchronously reenter
the same instance's entry methods from a callback. `notify_ready` signals the
dispatcher; it must not invoke `drain` inline. The current host uses bounded
polling for dispatch. A C `Work.context` is an opaque cookie: the SDK uses an
integer token and the host must not dereference it. `schedule_work` copies the
`Work` descriptor during the call; it never retains the descriptor pointer.

`Context::read` admits at most one pending read for that request and returns the
worker permit immediately. A validated `REQUEST_SERVICE_REPLY` schedules its
continuation on a fresh host CPU permit. Never block a worker waiting for a read,
spin on queue capacity, start a private worker pool, or run file I/O from UI code.
The raw ABI has CPU and I/O work classes; current SDK continuations use CPU work.
Nonzero `Work.affinity` is unsupported by the application host.

`Context::send` may retain one deferred output per request when the data queue is
full, without occupying a worker. Drain/release frees capacity and resumes the
continuation. `Ok(())` means the SDK owns the output/continuation, not that the UI
has displayed it. A rejected continuation schedule becomes a terminal failure.
Host implementations should reserve capacity for accepted work's continuations;
admission and data backpressure must not prevent terminal cleanup.

Poll `is_cancelled` during bounded CPU loops and between processing chunks.
Cancellation is cooperative; it does not promise immediate interruption of a
native OS read. Accepted read services still require matching replies after
cancellation/shutdown. The SDK retains physical service ownership until the
reply arrives, consumes late cancelled replies without invoking their
continuation, and checks service ID, session, generation, revision, offset and
raw byte count. The current host retains a QueueFull reply and its lease/admission
ticket while retrying.

The host releases event buffers even when discarding stale or malformed results.
Wrong session/generation, unknown request IDs and cancelled nonterminal output do
not reach the viewer. It also checks requested mode, revision, source bounds,
binary byte count and model validity. UI orchestration can cancel superseded
viewport/search requests; a plugin must not let their late work replace current
state. A stale result is not permission to release unrelated IDs.

## Quiescent shutdown

`begin_shutdown` refuses new user requests and cancels outstanding requests.
Continue receiving service replies, running/dropping accepted work, draining
terminal events and releasing output. `destroy` returns `NOT_QUIESCENT` until
jobs, live contexts, pending services, deferred/queued output, retained host
leases and host-leased plugin buffers have all ended. An already-completed
retained `Context` still counts. The SDK does not require an `EVENT_QUIESCENT`
notification; the current host uses the result of `destroy` as its readiness test.

The host additionally keeps its callback context alive until all in-flight
callbacks return. `Session::close`/Drop signal close without waiting on the UI;
backend cleanup returns the session quota only when complete. No timeout forcibly
unloads native code. An uncooperative trusted plugin can keep its session quota
occupied, and successfully loaded libraries remain pinned until process exit.

## Limits are separate layers

These are implementation limits, not a promise to cap arbitrary native plugin
allocations or whole-process RSS. Keep parser scratch, indexes and caches bounded
inside each plugin as well. The semantic and per-message limits are listed in
[the protocol reference](VIEWER_PROTOCOL.md#limits).

| Layer | Bound |
| --- | --- |
| Application sessions | 4 process-wide, including sessions still closing |
| Host decoded output queue | 64 events and 16 MiB per session; 64 MiB across four sessions; admission reserves terminal capacity |
| Host work queues | 32 CPU and 32 I/O jobs per session; one CPU worker and one I/O callback worker |
| Host command/service queues | 32 commands and 32 read-service requests per session; separate native read worker |
| Host input leases | 32 buffers / 4 MiB per session |
| Host read-reply channel | 32 chunks, at most 8 MiB, separate from leased input |
| SDK instances/jobs | 16 instances and 4096 registered jobs per loaded SDK library |
| SDK per instance | 64 admitted requests, 128 live contexts, 128 jobs, 64 pending reads |
| SDK ordinary output | 32 queued events; 128 allocations; 8 MiB |
| SDK deferred output | One per request; at most 64 and 8 MiB per instance |
| SDK retained host content | 128 leases / 8 MiB per instance; actual host may be stricter |
| SDK retained request CBOR | 8 MiB per instance |
| SDK terminal reserve | 64 allocations / 512 KiB total, 8 KiB each |

Host decoded-output accounting charges vector/string capacities, not just compact
CBOR size. These budgets coexist; do not add them into a claimed universal memory
ceiling, or mistake the SDK's 16-instance allowance for permission to open more
than the application's four sessions.

## Status and compatibility

| Status | Value | Meaning |
| --- | --- | --- |
| `OK` | 0 | Call accepted/succeeded; asynchronous work may still be pending |
| `INVALID` | 1 | Invalid metadata, model, state transition or exhausted ID space |
| `INCOMPATIBLE` | 2 | ABI/header mismatch |
| `QUEUE_FULL` | 3 | Bounded admission/output/lease capacity exhausted |
| `UNSUPPORTED` | 4 | Unsupported request, service, mode or scheduling option |
| `SHUTTING_DOWN` | 5 | Instance/context closing or already finished where relevant |
| `NOT_QUIESCENT` | 6 | Destruction must wait for physical ownership to end |
| `PANICKED` | 7 | Contained ordinary Rust panic |
| `UNKNOWN_HANDLE` | 8 | Unknown instance/request/service/buffer identity |

These are ABI call statuses. `ReadReply.status` and `Failure.code` carry `u32`
errors in asynchronous messages; existing implementations use the same constants.
A failure message is at most 4096 UTF-8 bytes. Do not treat a rejected `submit`
as an accepted request whose terminal will arrive later.

New host services/models must be explicitly versioned and negotiated in the
manifest. Unknown required services/models reject discovery; nonzero required
flags are not silently accepted. Optional CBOR fields can be skipped only after
the full bounded wire validation. Do not reinterpret existing field tags, units,
ownership rules or style flag bits. Breaking ABI changes require a new major;
breaking semantic models need new model identifiers. The reserved filesystem
kind does not create an implemented provider contract.
