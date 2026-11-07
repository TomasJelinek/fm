# Third-party libraries and licenses

Inventory captured on 7 November 2026 from the checked-in lockfiles and actual
Cargo selections for Windows x64 and Linux x86-64:

- [Direct dependencies](dependencies/DIRECT_DEPENDENCIES.md): **51**
  package/version records declared by project crates, including build/test tools.
- [Complete Cargo inventory](dependencies/CARGO_LIBRARIES.md): **446**
  package/version/source records, **422** distinct crate names, including transitives.
- [Machine-readable CSV](dependencies/cargo-licenses.csv): the same complete list,
  with licenses, source, workspace, target and direct dependants.

The twelve workspaces cover the application, public viewer/filesystem/thumbnail
APIs and worker SDK, three independent viewer plugins, archive/image/video plugins,
and the independent viewer ABI conformance project. Project-owned crates
are excluded. The negative descriptor fixture has no third-party dependencies.
Both target graphs include normal, build and test dependencies; this is broader
than the libraries linked into `me.exe`/`me`. Unused lockfile entries and optional
packages present only in Cargo metadata's feature union are excluded using
`cargo tree`. The complete list links to each workspace's maintained lockfile.

All entries have a declared license. Alternatives are retained verbatim; they do
not mean all alternatives are selected. The mandatory free-of-charge requirement
and Slint's selected royalty-free branch are in [the policy](DEPENDENCY_POLICY.md).

## Main functional libraries

This table is a guide; the linked inventories above contain every selected crate.

| Library | Purpose | Declared license |
| --- | --- | --- |
| Slint 1.18.1 | UI, compiler and render integration | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0; royalty-free desktop branch selected |
| Tokio 1.54.1 | Async backend orchestration | MIT |
| serde 1.0.229 / serde_json 1.0.154 / toml 0.9.12 | Data/settings/manifest serialization | MIT OR Apache-2.0 |
| sha2 0.10.9 | SHA-256 copy/readback verification | MIT OR Apache-2.0 |
| notify 8.2.1 | Filesystem event watching | CC0-1.0 |
| image 0.25.10 / png 0.18.1 | Icons and isolated image thumbnail decoding | MIT OR Apache-2.0 |
| zip 8.6.0 | Read-only ZIP archive provider | MIT |
| sevenz-rust2 0.23.0 / lzma-rust2 0.21.0 | Read-only 7z decoding | Apache-2.0 |
| tar 0.4.46 / flate2 1.1.10 and 1.1.11 | TAR/GZIP archive decoding, SDK packaging and image codec dependencies | MIT OR Apache-2.0 |
| blake3 1.8.7 | Thumbnail cache identities and integrity records | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| ffmpeg-next 9.0.0 / ffmpeg-sys-next 9.0.1 | Optional isolated video decoder bindings | WTFPL |
| bindgen 0.73.2 | Build-time native video bindings | BSD-3-Clause |
| regex 1.13.1 | Matching and the source viewer's custom lexer | MIT OR Apache-2.0 |
| pulldown-cmark 0.13.6 | Markdown parsing | MIT |
| parley 0.11.1 / swash 0.2.10 | Unicode text layout and glyph rendering | Apache-2.0 OR MIT |
| ICU casemap / locale core 2.4.0 | Unicode case mapping and locale data/types | Unicode-3.0 |
| minicbor 2.3.0 | Bounded plugin wire messages | BlueOak-1.0.0 |
| windows 0.62.2 / windows-sys 0.61.2 | Windows API bindings | MIT OR Apache-2.0 |
| rustix 1.1.6 | Unix native calls | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| arboard 3.7.0 | Native clipboard integration | MIT OR Apache-2.0 |
| cbindgen 0.29.4 | API C-header generation tool | MPL-2.0 |

The archive provider supports bounded, read-only ZIP/7z/GZIP/TAR/TAR.GZ browsing;
its concrete limits are in [the filesystem API](developer/FILESYSTEM_API.md).
The source viewer currently uses custom lexical
profiles with `regex`, not a comprehensive grammar engine such as syntect or
Tree-sitter. Media preview choices and implementation status are in the
[thumbnail design](superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).

## Native libraries, runtime and data outside Cargo

These are distinct from the Cargo inventory. Most are supplied by the user's OS;
versions, transitive dependencies and graphics drivers vary by installation.

| Component | Use | Upstream license family / provenance |
| --- | --- | --- |
| FFmpeg 9.0.2 shared libraries | Optional isolated video thumbnails; pinned source/build, not a dependency of the main executable | LGPL-2.1-or-later in the supplied configuration: GPL, nonfree, version3 and network disabled. Matching source, configuration and notices accompany the four runtime libraries |
| LLVM/libclang 18.1.1 | Build-time video binding generation, not shipped as application runtime | Apache-2.0 WITH LLVM-exception; retained upstream license |
| Fontconfig | Linux font discovery; direct ELF import | Permissive MIT/X11-style notices; distribution copyright file |
| FreeType | Linux font stack | FreeType License OR GPL-2.0; retain FTL credit when choosing FTL ([terms](https://freetype.org/license.html)) |
| GNU libc (`libc`, `libm`, loader) | Linux runtime | LGPL-2.1-or-later with component exceptions; distribution copyright files ([upstream](https://sourceware.org/glibc/)) |
| GCC `libgcc_s` | Linux unwind runtime | GPL-3.0-or-later WITH GCC-exception-3.1 ([runtime terms](https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html)) |
| X11/XCB libraries | Linux desktop/window integration | MIT/X11/BSD-family, per component ([X.Org notices](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/License.html)) |
| Wayland client libraries | Native Wayland integration | MIT-family; retain per-source notices ([upstream](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/COPYING)) |
| libxkbcommon | Linux keyboard handling | Combined permissive MIT/ISC/X11-style terms ([license](https://raw.githubusercontent.com/xkbcommon/libxkbcommon/master/LICENSE)) |
| libudev | Linux device discovery | LGPL-2.1-or-later ([systemd source](https://raw.githubusercontent.com/systemd/systemd/main/README)) |
| Vulkan loader | Linux graphics backend | Apache-2.0 with more-permissive exceptions ([license](https://raw.githubusercontent.com/KhronosGroup/Vulkan-Loader/main/LICENSE.txt)) |
| Windows system DLLs, UCRT, VC runtime | Windows shell, storage, UI, DirectWrite and graphics | Microsoft platform/runtime terms; not open source merely because the Rust bindings are MIT/Apache |
| Rust standard library/runtime | Both platforms | MIT/Apache-2.0 plus component-specific notices; releases include `licenses/rust/COPYRIGHT-library.html` |
| Unicode case-fold data in viewer packages | Unicode-insensitive search | Unicode-3.0; each package retains `UNICODE-LICENSE.txt` |

Read-only import inspection of the tested release found Linux `me` directly imports
`libfontconfig.so.1`, `libgcc_s.so.1`, `libm.so.6` and `libc.so.6`; the three viewer
libraries import `libgcc_s`, `libc` and the ELF loader. Window/graphics libraries can
also be loaded dynamically, so `readelf` alone is not a complete runtime inventory.
Windows imports include Kernel32/User32, Shell32, OLE, GDI, DirectWrite, DXGI,
OpenGL32, UCRT API sets and `VCRUNTIME140.dll`. Host fonts/icons are not bundled
third-party font/icon packs. OS drivers and installed external editors are not
pinned Cargo dependencies of this project.

## Distribution notices

Each release includes `THIRD-PARTY.json`/`THIRD-PARTY.txt` and copied dependency
license/notice files from the exact packaged graph. Independent viewers include
their own `LICENSE`, `THIRD-PARTY.md`, Unicode notice and generated dependency notices.
The package generator refuses missing required plugin notices. This development
inventory supplements those per-artifact notices, rather than replacing them.
Midnight Explorer's own application/API distribution license has not been selected;
these dependency reports do not grant a new license to project-owned code.

Video decoding currently enables VP8/VP9, Theora, FFV1 and raw video with
Matroska/WebM, AVI and Ogg containers. This is not a promise of MP4/H.264 support.
The video package retains FFmpeg's corresponding source archive and the exact
configuration needed to rebuild its shared libraries. `ffmpeg-sys-next` 9.0.1
declares WTFPL but its published source has no LICENSE file; distribution retains
the canonical WTFPL text, the pinned declaration and provenance rather than
claiming an upstream bundled license file exists.

## Regenerating the inventory

Use Python 3.11+ and the pinned Rust toolchain. Fetch dependencies and obtain the
released API/SDK bundle following the [plugin quick start](developer/QUICK_START.md).
The independent plugins use that bundle's `.cargo/config.toml`; they do not use
private host crates. Use an existing released bundle matching plugin lockfiles.
All capture and generated build output belongs under `_build/`.

Capture JSON and tree text for each scope below, using Windows for
`x86_64-pc-windows-msvc` and Linux for `x86_64-unknown-linux-gnu`. Name the outputs
`<scope>-<target>.json` and `<scope>-<target>.tree` in `_build/qa/dependency-inventory/`.

| Scope | Manifest |
| --- | --- |
| app | `Cargo.toml` |
| api | `api/Cargo.toml` |
| basic | `plugins/midnight-viewer-basic/Cargo.toml` |
| source | `plugins/midnight-viewer-source/Cargo.toml` |
| markdown | `plugins/midnight-viewer-markdown/Cargo.toml` |
| conformance | `plugins/viewer-conformance/Cargo.toml` |
| filesystem-api | `filesystem-api/Cargo.toml` |
| worker-api | `worker-api/Cargo.toml` |
| thumbnail-api | `thumbnail-api/Cargo.toml` |
| archive | `plugins/midnight-fs-archive/Cargo.toml` |
| thumbnail-image | `plugins/midnight-thumbnail-image/Cargo.toml` |
| thumbnail-video | `plugins/midnight-thumbnail-video/Cargo.toml` |

Example PowerShell capture for one scope/target; repeat using the table. Omit
`--config $sdkConfig` for application and API workspaces. Viewer plugins use the
released viewer bundle; archive and thumbnail plugins use the separate public
worker bundle described in [the worker SDK](developer/WORKER_SDK.md). Supply the
matching bundle's absolute config path; plugin lockfiles pin its package checksums.

```powershell
$scope = 'app'
$manifest = 'Cargo.toml'
$target = 'x86_64-pc-windows-msvc'
$captures = '_build/qa/dependency-inventory'
New-Item -ItemType Directory -Force $captures | Out-Null
./scripts/cargo.ps1 metadata --locked --format-version 1 --manifest-path $manifest `
    --filter-platform $target | Set-Content -Encoding utf8 "$captures/$scope-$target.json"
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
./scripts/cargo.ps1 tree --locked --workspace --manifest-path $manifest `
    --target $target --edges 'normal,build,dev' --prefix none `
    | Set-Content -Encoding utf8 "$captures/$scope-$target.tree"
if ($LASTEXITCODE -ne 0) { throw 'Cargo tree failed' }
python -B scripts/dependency-inventory.py --metadata-dir $captures
```

On Linux use `sh scripts/cargo.sh` and UTF-8 shell redirection instead of
PowerShell's `Set-Content`; use `python3` where Python is installed under that name.
`cargo metadata`/`tree` do not compile native code. Capture each target on its
native host: build dependencies/procedural macros follow the host, so a Windows
cross-target query is not equivalent to the native Linux graph.
The generator reads the twenty-four captures, excludes project-owned API/SDK crates,
and writes deterministic Markdown/CSV. Recapture after every dependency or feature
change. `--check` verifies tables against the supplied captures; it does not prove
those captures are current. Current source hashes are deliberately not labelled
as capture provenance. The captures for this inventory were refreshed on both
native hosts for the versions reported above.

```sh
python3 -B -m unittest discover -s scripts/tests -v
python3 -B scripts/dependency-inventory.py --metadata-dir _build/qa/dependency-inventory --check
```
