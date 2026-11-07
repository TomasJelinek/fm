# Build, test and install a viewer plugin

Use Rust 1.99.0 or newer with the native target toolchain: MSVC Build Tools on
Windows x64, or a native C toolchain on Linux x64. A plugin consumes an exported
SDK bundle and its own dependencies; the application checkout is not a build
requirement. All local intermediate files should remain under `_build/`.
Review the [dependency policy](../DEPENDENCY_POLICY.md) before adding libraries.

## Obtain the public SDK bundle

For API maintainers, from the repository root:

```powershell
cargo fetch --manifest-path api/Cargo.toml --locked
./api/scripts/package.ps1
```

```sh
cargo fetch --manifest-path api/Cargo.toml --locked
bash api/scripts/package.sh
```

Use the printed `_build/plugin-sdk-<timestamp>` directory; do not hardcode a
timestamp from another checkout. The bundle contains versioned API/SDK `.crate`
archives, a local registry, third-party vendor sources, `.cargo/config.toml`, the
public C header and `artifacts.json` with SHA-256 checksums/provenance. Packaging
does not publish to a remote registry. The `.invalid` registry URL is an offline
source identity, not a service to contact. Keep the entire bundle together when
relocating it between machines/operating systems.

An independent plugin declares its own workspace and lockfile, with version-only
public dependencies. For example:

```toml
[workspace]
resolver = "2"

[package]
name = "my-viewer"
version = "0.1.0"
edition = "2024"
rust-version = "1.99"
publish = false

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
midnight-plugin-sdk = { version = "=0.1.0", registry = "midnight" }

[profile.release]
overflow-checks = true
panic = "unwind"
```

The SDK reexports the API as `midnight_plugin_sdk::api`; add a direct versioned
`midnight-plugin-api` dependency only when useful. Never use a path to `api/`, a
source patch, application crates or another plugin. Do not confuse the SDK's own
maintenance path dependency with the version-only manifest inside its release.

Pass the bundle configuration explicitly with Cargo `--config`. Its relative
paths are designed for relocation. For a new project, generate `Cargo.lock` using
the bundle, then use `--locked --offline` for repeatable tests/builds. Preserve
that lockfile in the plugin project.

## Build an existing production plugin independently

Export/copy `plugins/midnight-viewer-basic/` to a directory outside the application
checkout and provide the SDK bundle separately. The commands below run from that
exported plugin directory. Replace the paths with your own private destinations;
installation must satisfy the [native trust rules](README.md#discovery-and-trust).

Windows PowerShell:

```powershell
./scripts/ci.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/basic/_build
./scripts/build.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/basic/_build -OutputDirectory C:/app/viewers/midnight-viewer-basic
```

Linux:

```sh
bash scripts/ci.sh /sdk/.cargo/config.toml /qa/basic/_build
bash scripts/build.sh /sdk/.cargo/config.toml /qa/basic/_build /app/viewers/midnight-viewer-basic
```

CI runs formatting, independent plugin tests, strict all-target Clippy and an
optimized native build. Build scripts write the actual native target and DLL/SO
filename into the installed manifest, and copy the plugin's notices. They install
only that package. They do not edit application code or another plugin.

The [Source](../../plugins/midnight-viewer-source/README.md) and
[Markdown](../../plugins/midnight-viewer-markdown/README.md) plugins use the same
CI/build arguments. Their `prepare-sdk` helpers generate a config under BuildRoot
that keeps the public registry and uses the normal Cargo cache for plugin-specific
dependencies. The API-only bundle vendor directory does not contain every parser
dependency. On an uncached machine, run `cargo fetch --locked --config GENERATED_CONFIG`
with permitted network access before the offline build. Linux preparation requires
Python 3.11+ for reading the SDK config.

For application maintainers building all three shipped packages from the shared
repository, the existing orchestration helpers are:

```powershell
./scripts/build-viewers.ps1 -Profile release -SdkConfig C:/sdk/.cargo/config.toml -AppDirectory C:/app -Test -Offline
```

```sh
MIDNIGHT_TEST_VIEWERS=1 MIDNIGHT_VIEWERS_OFFLINE=1 bash scripts/build-viewers.sh release /sdk/.cargo/config.toml /app
```

Those helpers orchestrate independent builds; they do not make plugins members
of the application workspace. Omit the SDK argument to let the helper package
the local public API. `MIDNIGHT_VIEWER_SDK_CONFIG` can supply it via environment.
Prefer an existing released bundle for independent development.

## Manifest and installation

An example installed Windows package is:

```text
app/
  me.exe
  viewers/
    my-viewer/
      plugin.toml
      my_viewer.dll
      LICENSE
      THIRD-PARTY.md
      notices/...
```

Linux uses the native executable and `libmy_viewer.so`. A minimal manifest is:

```toml
schema_version = 1
id = "example.viewer.mine"
kind = "viewer"
version = "0.1.0"
entry_library = "my_viewer.dll"
target = "x86_64-pc-windows-msvc"
abi_major = 1
minimum_host_minor = 0
required_host_services = ["content.read.v1", "work.schedule.v1"]
models = ["text-page.v1"]
modes = ["text"]
extensions = ["mytext"]
priority = 0
description = "My text format viewer"
```

Set Linux `target` to `x86_64-unknown-linux-gnu` and `entry_library` to the actual
`.so` basename. ID and version must exactly equal `Viewer::ID`/`VERSION` in the
loaded descriptor. The entry name is one filename, not an absolute/relative path.
Use a unique stable ID and a dedicated package directory. Supply the licences and
notices required by your distribution; the loader itself validates the manifest
and library rather than checking legal completeness.

| Field | Current discovery constraints |
| --- | --- |
| Whole manifest | UTF-8 TOML, at most 64 KiB; discovery inspects at most 256 directory entries under `viewers/` |
| `schema_version`, `kind` | Exactly `1`, `"viewer"` |
| `id` | Nonempty, at most 128 bytes; ASCII letters/digits, `.`, `-`, `_` |
| `version` / `description` | Version nonempty and at most 64 bytes; description at most 4096 bytes |
| `entry_library` | Nonempty basename, at most 255 bytes; no `/`, `\`, `:`, NUL, `.`/`..`, trailing dot or space |
| `target` | Exact running host target; Windows/Linux x64 are validated platforms |
| `abi_major` / `minimum_host_minor` | `1` / at most current host minor `0` |
| `required_host_services` | At most 16 entries; only `content.read.v1`, `work.schedule.v1` |
| `models` | 1–3 entries; `text-page.v1`, `binary-page.v1`, `document-blocks.v1` |
| `modes` | At most 4; `text`, `source`, `binary`, `document`; each needs its corresponding model |
| `extensions` | At most 512 entries; each at most 64 bytes, ASCII letters/digits or `_`, `-`, `+`; no leading dot |
| `priority` | Signed integer clamped by discovery to -1000 through 1000 |

`required_host_services`, description, modes, extensions and priority have
defaults. If modes are omitted, text/binary/document modes are derived from the
models; Source must be declared explicitly. An empty extension list plus Text mode
declares a generic fallback, so use format-specific extensions for a tutorial or
specialized plugin. Present unknown required services/models are refused.

Stop the application before replacing a package, install into its actual
executable-relative `viewers/` directory and restart. The plugin options dialog
shows discovered packages and refusal diagnostics. Merely copying into a source
checkout's `plugins/` directory does not install a plugin. Libraries remain loaded
until process exit, even after their last window closes.

## Minimal asynchronous example

The complete teaching package is [examples/tiny-viewer](examples/tiny-viewer/Cargo.toml),
with [source](examples/tiny-viewer/src/lib.rs) and
[manifest](examples/tiny-viewer/plugin.toml). Copy that directory to your own
workspace before building; keep build products and the generated lockfile in
your copied project. It depends only on the released SDK.

Its `Viewer` implementation exports `midnight_plugin_entry_v1`, accepts one Text
session, decodes Open/Viewport requests, reads through `Context::read`, emits a
`TextPage` with exact scalar-boundary anchors through `Context::send`, and
completes in the send continuation. It checks session/generation, read errors,
cancellation and logical close, handles output rejection, and owns no host pointer
or private thread. The source illustrates the actual API rather than placeholder
symbols or pseudocode.

From the copied example directory, with the SDK bundle at `C:/sdk`:

```powershell
cargo generate-lockfile --offline --config C:/sdk/.cargo/config.toml
cargo check --locked --offline --config C:/sdk/.cargo/config.toml --target-dir _build/target
cargo clippy --locked --offline --all-targets --config C:/sdk/.cargo/config.toml --target-dir _build/target -- -D warnings
cargo build --release --locked --offline --config C:/sdk/.cargo/config.toml --target-dir _build/target
```

On Linux use the same Cargo arguments with `/sdk/.cargo/config.toml`. Install the
resulting `midnight_viewer_tiny_example.dll` or `libmidnight_viewer_tiny_example.so`
and the example manifest into a dedicated private `viewers/tiny-example/`
directory. Update manifest target/library fields for Linux. After restarting the
application, open a UTF-8 file ending in `.tinytext` to exercise extension routing.

This example deliberately rejects backward viewports, searches, malformed UTF-8,
many control characters and seeks into a UTF-8 scalar. It reads at most 4096 bytes
at a time and respects the requested line limit. It does not provide BOM/UTF-16
decoding, normalized line endings, visible invalid-byte escapes, complete bidi
control handling, syntax styles or a seek index. These limitations are explicit
failures, not production capabilities. The independently tested Basic plugin is
the reference for robust general-purpose text handling. The example was compiled,
Clippy-checked and built as a native Windows DLL against the released SDK; that
is API/build validation, not a claim of a full example integration test suite.

## Test the independent contract

Keep format tests and API/service fixtures inside your plugin. Cover malformed
input, short reads without EOF, errors, cancellation, stale generations, source
offsets above 4 GiB, checked overflow, UTF-8/UTF-16 boundaries, exact anchors,
output pressure and shutdown with retained leases. Test bounded memory and the
combined wire-item budget, not just successful small-file rendering. Run the
same exported source project on Windows and Linux, with its own lockfile and no
application-source dependencies.

The [conformance fixture](../../plugins/viewer-conformance/README.md) demonstrates
loading the actual DLL/SO through C, including ownership, cancellation, late
replies, backpressure and panic containment. Do not install that test fixture as
a user-facing viewer. Its tests belong to its independent project; generic host
tests live in the application and do not own concrete plugin format tests.

For public API maintainers, from the repository root:

```text
cargo test --manifest-path api/Cargo.toml --workspace --locked
cargo clippy --manifest-path api/Cargo.toml --workspace --all-targets --locked -- -D warnings
cargo run --manifest-path api/Cargo.toml -p midnight-api-headergen --locked
```

Use `./api/scripts/check-abi.ps1` or `bash api/scripts/check-abi.sh` to compare C11
and Rust sizes, alignment and every public field offset. The header generator is
a maintainer tool; plugin consumers do not need application sources or cbindgen.
Avoid manually editing the generated header.

## Extending the public surface

A new parser or style mapping usually needs only a new independent viewer
package. A new semantic model, resource service or provider operation needs a
public API design, versioning, bounds, ownership rules and cross-language tests.
Declare only services/models the actual host supports. Preserve existing tags,
units and ABI prefixes; do not import an internal implementation to sidestep a
missing public capability. Follow [compatibility rules](ABI_AND_LIFECYCLE.md#status-and-compatibility)
and keep [provider proposals](README.md#filesystemprovider-api-status) distinct
from today's shipped viewer protocol.
