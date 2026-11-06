# Midnight Explorer public plugin API — implementation draft

This is an independent Cargo workspace with its own dependency lock. It shares
no application crates. Concrete plugins belong in independent workspaces under
`plugins/`, each with its own tests/lock/build. A plugin may depend on released
`midnight-plugin-api` and `midnight-plugin-sdk`, never on application internals,
another plugin, or a source-tree patch. The SDK's ownership/runtime adapters are
implemented and unit tested; the loader, independent DLL/SO conformance fixture
and concrete plugins remain in progress. This is not a compatibility promise yet.

From the repository root with Rust available:

```text
cargo test --manifest-path api/Cargo.toml --workspace --locked
cargo clippy --manifest-path api/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo run --manifest-path api/Cargo.toml -p midnight-api-headergen --locked
```

Generated and intermediate files stay under the shared `_build` root. Running
Cargo from `api/` uses `../_build/api`; running from the repository root uses its
`_build` configuration. Explicit `CARGO_TARGET_DIR` can select another directory
under that root. `include/midnight_plugin_v1.h` is an intentional generated public
source artifact; the maintainer generator derives it from `src/abi.rs` using
[cbindgen](https://github.com/mozilla/cbindgen). Plugin consumers do not need to
build the generator or access application sources.

Cross-language layout verification:

```powershell
./api/scripts/check-abi.ps1
```

```sh
bash api/scripts/check-abi.sh
```

These compile a C11 consumer and compare every ABI struct size, alignment and
field offset against Rust. Windows uses MSVC Build Tools; Linux uses `cc`.
All C executables/objects and comparison output remain under `_build/api-abi`.

## Draft ABI ownership

- The sole plugin export is `midnight_plugin_entry_v1`, matching `MidnightEntry`.
  Its output capacity bounds all writes. The 16-byte header is checked before
  accessing the required function-table prefix; optional future tails are ignored.
- One process, trusted native libraries. Pointer metadata checks do not validate
  arbitrary native addresses or sandbox plugin code. No Rust allocator, trait,
  Tokio object or Slint object crosses this boundary.
- Entry calls for one instance are serialized on backend dispatch. All callbacks
  are `extern "C"` and must contain panics/exceptions. Long work uses host permits.
- Request payload slices are borrowed for the call. A request's optional host
  content buffer needs a successful retain before use after that call. The host
  keeps immutable bytes valid until the matching releases.
- Event payload and optional raw content are separately owned plugin buffers.
  The host releases each nonzero ID through its producing instance exactly once;
  it never frees plugin memory using its allocator. ID zero only represents an
  empty buffer. Raw content is not a CBOR array of individual byte integers.
- Accepted work owns its context until exactly one plugin drop callback, whether
  it ran or was cancelled before running. Rejected work retains caller ownership.
- Shutdown rejects new user requests, cancels outstanding work and drains terminal
  events. Destruction requires quiescence, including workers, continuations,
  retained host buffers and outstanding plugin buffers. No hot unload is planned.

## Wire and semantic models

The shared codec uses [minicbor](https://github.com/twittner/minicbor), with a
non-allocating validation pass before typed decoding. Version 1 uses definite
CBOR maps with strictly increasing unsigned numeric field tags. Duplicate keys,
indefinite containers, CBOR tags, floats, malformed UTF-8 and trailing bytes are
rejected. Optional unknown fields may be skipped only after the same validation.
The envelope limit is 1 MiB, 65536 total items and nesting depth 16. Encoding has
the same byte limit and checks model validity before writing.

`text-page.v1` currently defines revision (tag 0), source range (1), decoded UTF-8
text (2), ordered disjoint style spans (3), exact source anchors (4), before/after
flags (5/6) and partial-line flag (7). Source offsets/lengths are checked `u64`;
text ranges are bounded UTF-8 byte offsets, never pixels. Copy uses decoded text;
only explicitly listed anchors map exactly back to source bytes. Never interpolate
across UTF-16 or visibly escaped invalid bytes. Limits are 256 KiB decoded text,
4096 spans and 16384 anchors, also subject to the aggregate wire-item limit.

Styles name semantic roles and flags; the host applies theme colors/fonts. Plugin
data does not carry executable UI markup or shell commands. Binary/document
models are also defined: binary pages refer to separate bounded raw buffers;
document pages contain at most 512 semantic blocks with shared text/style/anchor
budgets and bounded link metadata. The host never treats a link target as an
automatic action. Viewer Open/Viewport/Search and ReadAt/ReadReply/CloseReader
schemas preserve explicit revisions, optional lengths, EOF and errors. The
[SDK contract](midnight-plugin-sdk/README.md) describes owned requests, content
leases, host-scheduled continuations, output backpressure and quiescent shutdown.
The content broker and actual viewer plugins remain pending.

The API/SDK packages will be staged as versioned `.crate` artifacts with an actual
local dependency-source workflow. Exported-plugin CI must prove it can build/test
without the application checkout. No package has been published to a registry.
