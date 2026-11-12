# Midnight Explorer plugin SDK — implementation draft

This package supplies ownership and asynchronous dispatch for independent native
viewer plugins. It depends only on the public API. It contains no file decoder,
syntax catalogue, Markdown parser, application crate, Tokio runtime or Slint UI.
The ABI is still under integration testing; this is not a released compatibility
promise. A standalone exported DLL/SO fixture exercises this contract on Windows
and Linux. Integration with the actual application loader is the next gate.

A plugin is an independent Cargo workspace in the shared repository's `plugins/`
directory. Its tests, dependencies and lockfile belong to that plugin. It must
build from an exported directory using versioned API/SDK packages, without the
application checkout, source patches or changes to another plugin. No plugin may
modify the main application's source code.

## Author interface

Implement `Viewer` and use `export_viewer!(YourViewer)` in a `cdylib` crate. The
macro exports only `midnight_plugin_entry_v1`. `create` constructs bounded state;
`handle` receives an owned `Request` and a `Context` on a host worker. Decode the
request using `Request::decode<T>()` and the public API models.

- `Context::read` submits one bounded positional read, then returns its worker
  permit. A validated reply schedules the continuation on another host permit.
- `Context::send` accepts a validated `Output` and a continuation. When the data
  queue is full, the SDK retains one output for that request without occupying a
  worker; drain/release resumes it. Rejection returns the output and continuation.
- `Output` constructors validate semantic pages, source ranges and raw binary
  length. Themes, glyph rendering, windows and filesystem authority stay in the host.
- Finish with `complete` or `fail`. Dropping the last unfinished context produces
  failure. Poll `is_cancelled` during bounded CPU loops; cancellation prevents late
  output from changing a finished request.
- `Content` clones share one immutable native lease. The last clone releases it
  exactly once. Do not copy a raw ABI pointer into plugin-owned asynchronous state.

Plugin code may keep continuations and bounded parsing state, but must not block a
worker while waiting for a host service, start private worker pools, or retain
finished contexts indefinitely. Close requests must release session caches and
plugin-held contexts. All callbacks and large cleanup run on backend workers.

## Host contract

The host copies work descriptors during `schedule_work`; it does not retain the
descriptor pointer. On rejection it invokes neither callback. On acceptance it
owns exactly one `drop`, whether `run` is invoked once or cancelled before running.
Callbacks run on backend workers, never the UI thread. The SDK's opaque work
cookie is an integer token; the host must not dereference it.

Reserve scheduling capacity for continuations of accepted requests. Admission may
return `QUEUE_FULL`; an already accepted request must be able to finish. The SDK
reports a terminal failure if the host rejects continuation work. Work and service
callbacks must not synchronously reenter the same instance's ABI entry methods.
`notify_ready` signals the backend dispatcher; it must not call `drain` inline.

Service transaction IDs are distinct from user request IDs. The host returns the
service ID, original session/generation, revision and offset in its reply. A read
can be short without EOF. Accepted services require a reply even after cancellation
or shutdown. The SDK keeps physical service ownership until that reply arrives;
cancellation does not promise immediate interruption of an OS/network read.

Shutdown rejects new requests and emits cancellation terminals. Drain every event
and release each nonzero payload/content ID independently. Destruction returns
`NOT_QUIESCENT` until jobs, live contexts, services, retained host leases, queued
output and host-leased plugin buffers are gone. Reused request IDs cannot hide
older retained contexts. The host additionally tracks in-flight callbacks and keeps
the library and host callback context loaded until their calls return. No hot
unloading is supported.

`drain` accepts at most 64 output slots. Its written count is updated per event,
including if a later iteration fails; those buffers still need release. Invalid
handles never release allocations belonging to another instance.

## Bounds and trust

Per instance: 64 admitted requests, 128 live contexts, 128 jobs, 64 pending reads,
32 ordinary queued events, 128 ordinary output allocations and 64 reserved terminal
allocations. The library allows 16 instances and 4096 registered jobs. IDs fail on
exhaustion instead of wrapping.

Separate byte budgets cover ordinary output (8 MiB), deferred output (8 MiB), host
input leases (8 MiB), retained request CBOR (8 MiB), and reserved terminal messages
(512 KiB total, 8 KiB each). Allocations also have count bounds, including empty
buffers. These are transport limits, not a replacement for the host's aggregate
viewer budget or each plugin's decoded-page/index cache budget.

Native plugins execute as trusted code in the host process. The FFI modules check
null/alignment, lengths, versions and ownership; they cannot validate arbitrary
mapped addresses or sandbox a DLL. The caller guarantees live immutable input and
writable output memory for each ABI call. Panic guards contain ordinary unwinding
panics, including a panicking payload destructor. They cannot recover from aborts,
double panics during unwinding or native undefined behavior.

The SDK tests cover ownership, queue pressure, rejected admission, continuations,
late replies, cancellation, ID reuse, shutdown, panic boundaries and ABI metadata.
Run them independently from the repository root:

```text
cargo test --manifest-path api/Cargo.toml --workspace --locked
```

Build output stays under `_build`.
