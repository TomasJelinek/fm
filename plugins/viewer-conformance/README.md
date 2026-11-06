# Viewer ABI conformance fixture

This independent Cargo workspace tests the released public API/SDK contract. It
is not an application workspace member and depends only on a versioned SDK from
the explicitly configured `midnight` release registry. It has no host path
dependency or source patch. Do not install this fixture in a user's `viewers/`.

Its native C tests load the built DLL/SO through the sole versioned export and
exercise admission, delayed reads, retained buffers, cancellation, backpressure,
panic containment and quiescent shutdown. They belong to this plugin project.
Windows and Linux loader/test scripts are kept in separate files.

Build an API/SDK bundle with `api/scripts/package.ps1` or `api/scripts/package.sh`,
then pass its `.cargo/config.toml` with Cargo's `--config` option. The registry is offline;
the `.invalid` index URL identifies the source and is never a download endpoint.
The bundle contains versioned `.crate` files, SHA-256 checksums and third-party
dependency sources. The exported project must pass without the application source.

From the repository root, using the printed bundle directory:

```text
cargo build --manifest-path plugins/viewer-conformance/Cargo.toml --release --locked --offline --config BUNDLE/.cargo/config.toml
```

On Windows, run the native tests from an x64 Visual Studio Native Tools terminal:

```powershell
./plugins/viewer-conformance/tests/test-windows.ps1 -Library _build/release/midnight_viewer_conformance.dll -HeaderDirectory BUNDLE/include -BuildRoot _build/conformance/windows
```

On Linux:

```sh
bash plugins/viewer-conformance/tests/test-linux.sh _build/release/libmidnight_viewer_conformance.so BUNDLE/include _build/conformance/linux
```

For an exported project, set `CARGO_TARGET_DIR` and temporary/cache directories
below its common `_build` directory. The tests exercise the actual loaded native
library. One scenario deliberately panics inside Rust: its panic hook prints to
stderr, while the SDK returns a failed event and safely drains/destroys the instance.
