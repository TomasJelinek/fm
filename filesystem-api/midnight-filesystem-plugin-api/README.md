# Native filesystem API

Public, standard-library-only C ABI model and bounded binary codec for independent
filesystem plugins. Crate version 0.2.0 exposes ABI **1.3**; the FSBR payload format
remains version 1, with unchanged entry symbol and C descriptor layout. Browse/Read
and optional volume/activation services remain alongside optional prepared Rename.

See [the protocol](../NATIVE_PLUGIN_PROTOCOL.md), [C header](../include/midnight_filesystem.h),
[ABI definitions](src/abi.rs), [request/response model](src/model.rs) and
[Rename types](src/rename.rs). The encoder/decoder define field order; Rust
layouts, strings and allocations never cross the C boundary.

## Optional prepared Rename

`CAP_PREPARED_RENAME = 1024` requires descriptor ABI minor and
`minimum_host_minor` of at least 3. Upgrading the crate does not advertise this
capability or authorize a host to send Rename requests to an unsupported provider.
Older descriptors without the capability retain their existing compatibility.
`Header::v1` defaults to minimum host minor 0; handwritten descriptors must raise
it when enabling Rename. The SDK export helper sets it from the capability.

The extension exposes:

- `PrepareRename` (20): select an issued mount/node and return a retained prepared
  handle, source snapshot and parent identity before confirmation.
- `Rename` (21): use that handle, a nonzero attempt, exact raw basename and
  `NoReplace` or conflict-token-bound `RecoverableCurrentEntry` policy.
- `GetRenameState` (22): query the known prepared handle after a lost response
  while the instance remains usable;
  an unsuccessful transport call does not prove an effectful Rename was undone.
- `CloseRename` (23): dispose retained resources without a namespace mutation.

Identity is captured during explicit preparation, **not at listing time**. Browse
node IDs, read handles, revisions, captions and paths alone grant no mutation
authority. Existing observed names are lossless; proposed destination names have
their own namespace validation. Recovery locators are observations, not later
write authority.

The API validates structural fields and bounds: 64 KiB names, 256-byte identities,
1–4096-byte revisions, at most four recovery records and 256 KiB aggregate recovery
locator bytes. Native binding checks, attempt/replay state, publication ownership
and cancellation behavior require an implementation; the codec does not supply
them. The [SDK](../midnight-filesystem-plugin-sdk/README.md) provides a bounded
native-author helper. See the [complete prepared Rename contract](../NATIVE_PLUGIN_PROTOCOL.md#optional-prepared-rename-abi-minor-3)
for receipt states, limits and lifecycle rules.

These public types and helpers do not establish platform implementation,
capability advertisement, host/UI cutover or native qualification. Archive worker
protocol 1 and the viewer ABI remain separate.

## Development

From the repository root:

```sh
cargo test --locked --offline --manifest-path filesystem-api/Cargo.toml --workspace --target-dir _build/filesystem-api
```

Keep build/test output under `_build/`. Independent plugins depend only on the
public API/SDK and their own dependencies; application engine/platform crates
remain private.
