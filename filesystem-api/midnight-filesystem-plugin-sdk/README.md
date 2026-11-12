# Native filesystem author SDK

The standard-library-only SDK supplies the synchronous C entry, bounded instance
and buffer admission, borrowed cancellation and panic containment. Implement
`Filesystem::create(Config)` to construct bounded state without a private worker
pool, and `Filesystem::handle(Request, &CallContext)` to return `Result<Reply,
Failure>`. Export the descriptor with `export_filesystem!`. Calls, encoding and
native cleanup belong on host backend workers.

`native::NativeFilesystem<A>` provides shared mount, node, cursor, reader and
prepared-rename policy around an OS-specific `NativeAdapter`. Native operations
remain the adapter's responsibility; the helper does not establish platform
qualification or application host/provider cutover.

Prepared Rename is optional. Advertise `api::abi::CAP_PREPARED_RENAME` (1024)
explicitly in `export_filesystem!` only when implemented. The SDK sets minimum
host minor 3; the C descriptor layout is unchanged. Implementing the adapter
method does not advertise the capability automatically.
`NativeAdapter::prepare_rename(&Path, &CallContext)` defaults to Unsupported,
preserving adapters that do not offer this extension.

Prepare captures and binding-validates the current source and parent without
writes. A Browse node is a namespace locator, not listing-era object identity.
Return an `Arc<dyn native::NativeRename>`; retain those pins across confirmation
instead of replacing them with a fresh pathname open. Its `prepared()` method
returns an immutable cached `PreparedRenameInfo` snapshot without native I/O.
Its `execute(name, policy, context)` must revalidate retained source, parent and
target pins, reserve exact receipts before writes and complete recovery after
the commit gate even if canceled. SDK conflict tokens express consent and are
not native authority.

Resource responses (Resolved, Opened, Page and RenamePrepared) require
`Reply::owned(response, completion)`; `Reply::stateless` rejects them. Capture
ownership in the closure. Completion runs exactly once: `true` commits after
encoding and buffer retention succeed; `false` rolls back unpublished resources.
Cleanup must ignore cancellation, release publication gates on both paths and
never reenter the instance. On `completion(true)`, StaleReference is allowed only
after fully rolling back resources whose mount or generation retired; the SDK
releases the buffer and reports UNKNOWN_HANDLE without poisoning admission.
Other completion errors or panics stop admission, as does failed rollback.

Prepare uses this publication rollback to dispose an undisclosed prepared lease
after encoding/publication failure or late observed cancellation. Effectful
Rename has a different completion: it releases the busy gate while preserving
the prepared handle and recorded outcome. Delivery failure does not request
namespace rollback or prove that nothing changed. Query the same handle with
GetRenameState after a lost reply.

State starts at attempt 0/Ready. Begin with attempt 1; an identical current
attempt replays its recorded result without executing again. Changed input for
that attempt is Invalid; older attempts are stale. Only Conflict admits the
exact next attempt. RecoverableCurrentEntry additionally requires the latest
conflict token, the same raw destination and eligible distinct regular files.
Unchanged, Renamed and RecoveryRequired are terminal. A malformed provider
receipt becomes an unknown RecoveryRequired outcome. A provider panic poisons
instance admission, so post-panic state retrieval is not guaranteed.

Cancellation is borrowed for one synchronous call and cannot interrupt an
arbitrary OS call. Prepare permits late cancellation rollback. Rename preserves
the actual result instead of rewriting a completed response as Cancelled;
adapters cooperate before the commit gate and finish recovery after it.
GetRenameState and CloseRename ignore cancellation. Query can observe Executing;
concurrent Rename or CloseRename returns Busy through execution and publication.
CloseRename disposes the lease; it does not remove residual recovery objects.
Every native execution retires the mount's Browse epoch conservatively; retained
prepared leases support query, replay and close while the instance remains usable.

The native helper admits 32 opening/live prepared resources, charging 1 MiB each
within its shared 256 MiB native cache budget: at most 32 MiB of rename charges.
Slots are acquired before native opens and held through native disposal. Authors
must fit private pin, conflict and receipt capacities within each reservation,
alongside the SDK ledger and captured paths. Hidden allocations cannot be
inspected by the SDK; these charges are not a total process RSS guarantee.

The first admitted PrepareRename, GetRenameState or CloseRename reserves one
shared control call, one buffer and `Config.reply_bytes` within existing Config
ceilings for queries/closes, reducing ordinary capacity for the instance.
Admission requires at least two calls, two buffers and twice `reply_bytes` in
retained-buffer capacity; smaller legacy configurations can still serve
Browse/Read but refuse Prepare before pins. Release a control reply before the
next control call: its shared lane can also saturate. Rename requires a reply
budget of at least `api::MAX_RENAME_RECEIPT_BYTES` (384 KiB) before execution.

Prepared leases retain their mount until native disposal; CloseMount remains
Busy while dependent owners exist. The host must retain the loaded library
through instances, calls, callbacks, reply buffers and resource disposal.
Release every successful buffer exactly once through its original instance and
descriptor. A failed C call returns a zero buffer; destroy refuses active calls
or retained buffers, then disposes remaining resources outside registry locks.

See the [prepared Rename contract](../NATIVE_PLUGIN_PROTOCOL.md#optional-prepared-rename-abi-minor-3)
and [protocol and limits](../NATIVE_PLUGIN_PROTOCOL.md). This is trusted
in-process code, with no sandbox or immutable-content guarantee.
