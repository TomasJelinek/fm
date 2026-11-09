# Native filesystem author SDK

The std-only SDK implements the audited C entry, instance/buffer admission,
cancellation borrowing and panic containment. Implement `Filesystem`, return a
`Reply`, and use `export_filesystem!`. `native::NativeFilesystem<A>` supplies
bounded native mount, node, cursor and reader policy around an OS-specific
`NativeAdapter`; the Windows/Linux packages demonstrate real implementations.

Resource replies must use `Reply::owned`. Its completion receives `true` after
encoding and buffer publication succeed and `false` on rollback, including late
observed cancellation. Capture resource ownership in the closure, perform cleanup
without cancellation, and release any publication gate on both paths. Cleanup
must not reenter the instance. A closure failure or panic poisons admission.
Stateless responses use `Reply::stateless`; it rejects resource response variants.

See [protocol and limits](../NATIVE_PLUGIN_PROTOCOL.md). This is trusted
in-process code, with no sandbox or immutable-content guarantee.
