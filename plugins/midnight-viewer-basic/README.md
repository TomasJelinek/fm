# Basic viewer

Independent `midnight.viewer.basic` 0.1.0 plugin. It depends only on versioned
`midnight-plugin-sdk` 0.1.0 from the `midnight` registry, which supplies the public
API. Its Cargo workspace and lockfile are separate from the application.

## Build, test and install

Export this entire directory and a released public SDK bundle. Install Rust
1.99 or newer with rustfmt, Clippy and the native platform linker. The SDK bundle
contains its offline registry and dependencies; no application checkout, source
patches or network access are needed for these commands.

```powershell
./scripts/ci.ps1 -SdkConfig ../sdk/.cargo/config.toml -BuildRoot ./_build
./scripts/build.ps1 -SdkConfig ../sdk/.cargo/config.toml -BuildRoot ./_build `
    -OutputDirectory C:/disposable-install/viewers/midnight-viewer-basic
```

```sh
bash scripts/ci.sh ../sdk/.cargo/config.toml ./_build
bash scripts/build.sh ../sdk/.cargo/config.toml ./_build \
    /disposable-install/viewers/midnight-viewer-basic
```

When working in the shared repository pass its `_build/qa/viewer-implementation/basic`
as `BuildRoot`. The scripts change only that build directory and the exact output
package directory. `build` discovers the native Rust target and writes the matching
DLL/SO manifest. The checked-in manifest is the Windows template. The default
output is `BUILD_ROOT/package/midnight-viewer-basic`; copy that directory under
`viewers/` beside the host executable. Remove it and restart to remove capability.
The platform-neutral ABI tests use real SDK dispatch and host service callbacks.
The distribution's host tests own native DLL/SO loading and routing checks.

## Behavior and limits

- Text mode supports UTF-8 (with or without BOM) and BOM-marked UTF-16 LE/BE.
  It does not guess ANSI/code-page encodings. CR, LF and CRLF display as LF.
  Invalid bytes display as `\xFF`, malformed UTF-16 units as `\uD800`, and
  control characters except tab/newline as visible escapes. Escape sequences
  are never interpreted as terminal commands. Valid Unicode and combining/RTL
  content remain intact for the host renderer.
- Binary mode sends exact bounded raw bytes and u64 source offsets to the host's
  hexadecimal/printable-column renderer, with at most 16 bytes per requested row.
  The caller selects Text or Binary explicitly. Alt+F3 remains Text; ordinary F3
  can select Binary after host probing. This plugin does not reinterpret Text.
- Open emits `Opened`; the host then requests a viewport. Seeking reads a bounded
  neighborhood of the requested source anchor, including a four-byte alignment
  margin. It never scans a huge line or file before returning its first viewport.
  Backward viewport requests return the nearest bounded lines before the anchor.
  Every decoded unit has exact start/end source anchors; escaped bytes and UTF-16
  offsets must not be interpolated. Pages remain below 256 KiB and CBOR item limits.
- Each positional read is at most 8192 bytes. Short reads accumulate until the
  requested range is full or explicit EOF arrives. Three consecutive zero-byte
  non-EOF replies produce a terminal error. Small viewports that cannot contain
  one complete decoded unit also report an error, without skipping source bytes.
- Each instance admits at most 32 sessions. A session retains at most 1024 sparse
  source checkpoints, coarsened as needed; it retains no decoded file cache.
  Line count stays provisional (`estimated_lines = None`). Individual requests
  retain bounded read, decoded-page and search state; the SDK separately caps
  active requests, transport buffers, host leases and queued output.
- Search uses streaming KMP with Unicode 17 default full case folding (C+F,
  non-Turkic), including expansions such as `ß`/`ss`. It does not normalize
  canonically equivalent strings. Queries are capped by the API at 4096 UTF-8
  bytes; matches return exact original byte ranges, including across pages.
  Forward searches include starts at the anchor. Backward searches select the
  nearest match ending at or before the anchor by a cancellable bounded scan
  from the BOM; their latency is proportional to the preceding file content.
  Search operates on decoded visible text, including visible escapes in binary
  mode, and is not a hexadecimal-pattern search. No match emits only completion.
- Generation changes and Close invalidate pending session work. The SDK owns
  content lease retention, cancellation, backpressure and terminal events.
  The host owns and closes its reader; the plugin never opens filesystem paths,
  launches programs, accesses the network or creates worker pools.

Salamander's `src/plugins/shared/spl_view.h` lifetime/probe contract and
`src/viewer2.cpp` buffered reads and newline boundaries were inspected as behavior
references. No Salamander code or assets were copied.
