# Native filesystem API

Public, std-only C ABI model and bounded binary codec for native Browse/Read
plugins. See [the protocol](../NATIVE_PLUGIN_PROTOCOL.md) and
[C header](../include/midnight_filesystem.h). Archive worker protocol 1 remains
in its separate `midnight-filesystem-api` crate.

From the repository root:

```sh
cargo test --locked --offline --manifest-path filesystem-api/Cargo.toml --workspace
```

Supply a target directory below `_build/`. Plugin projects must depend only on
the public API/SDK; application engine/platform crates are private.
