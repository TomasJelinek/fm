# Third-party libraries and licenses

The independent image viewer and thumbnail workers now share
`plugins/image-decoder`: the same pinned raster/color libraries plus resvg/usvg
0.48.1 with only `text` enabled for bounded static SVG. MIT is selected for
resvg/usvg and the existing image-rs dependency; tiny-skia's BSD-3-Clause notices
remain intact. Source-selected image/font/file/network loading is disabled.
Unmodified Noto Sans Regular/Arabic/Hebrew font bytes are embedded under OFL-1.1;
see [source and hashes](../plugins/image-decoder/fonts/README.md). Both worker
packaging paths copy the complete shared `plugins/image-decoder/notices` set,
including font copyright/OFL/provenance. SVG reuses pinned versions. AVIF adds
MIT dav1d 0.11.0/dav1d-sys 0.8.3 bindings, BSD-2-Clause native dav1d 1.5.4 and
MPL-2.0 mp4parse 0.17.0. Both packages retain the full native license and the exact
unmodified MPL parser source archive with its license/source notice. The narrowly
patched image-rs 0.25.10 source is selected only by the three independent image
workspaces; see [patch provenance](../vendor/image/MIDNIGHT_PATCH.md) and the
[shared notice inventory](../plugins/midnight-viewer-image/THIRD-PARTY.md).
No API/application license change is made. HEIC/HEIF remains unsupported pending a
qualified packaged decoder and royalty-free distribution determination.

Managed command artwork reuses the pinned `image` 0.25.10 (`MIT OR Apache-2.0`)
for PNG encoding and pixel resizing only in `midnight-platform-common`; its new
direct declaration enables only `png` with default features disabled. Arbitrary
binary image imports continue through the existing restricted image thumbnailer.
The Linux native icon selector now directly uses `resvg`/`usvg` 0.48.1
(`Apache-2.0 OR MIT`) and their existing `tiny-skia` 0.12.0 (`BSD-3-Clause`)
renderer. The direct `resvg` declaration disables default features. Other UI
consumers may unify features, so the icon renderer explicitly supplies empty font
state and disables both external and embedded-image resolvers; SVG icon rasterization
uses no font-file, network, or compressed-SVG loading.
The exact cached license texts were inspected; MIT is selected for `image`,
`resvg` and `usvg`, and the full BSD-3-Clause notice remains for `tiny-skia`.
The existing release notice collector retains all their upstream license and
copyright files under `licenses/<crate>-<version>/`. No new package version,
mandatory fee, subscription or royalty is introduced.

The directory authorization helper adds a project-owned crate and reuses the pinned
`getrandom` 0.3.4 (MIT OR Apache-2.0), `windows-sys` 0.61.2 (MIT OR Apache-2.0),
and `libc` 0.2 (MIT OR Apache-2.0) dependencies already present in the inventory.
Its Windows API feature selection adds no new package/version and enables only
native filesystem, process, named-pipe, security and shell declarations. The helper
contains no plugin discovery or loading entry point. Existing notices apply.

Make File List adds no third-party library or enabled feature. Its engine reuses
the workspace's `midnight-worker-platform` private spool and existing transfer,
clipboard, native picker and viewer services.

The additional color themes add no Cargo library or feature. Modern palette
references use Apache-2.0 (Tokyo Night) and MIT (One Dark, Rosé Pine and Catppuccin), with
[pinned provenance, adaptation notes and retained license texts](THEME_PROVENANCE.md).
Release packages carry that record and `licenses/themes/` alongside library notices.

Inventory updated on 12 November 2026 from the checked-in lockfiles and actual
Cargo selections for Windows x64 and Linux x86-64, plus Darwin x86-64/arm64
captures for the independent native macOS filesystem package only:

- [Direct dependencies](dependencies/DIRECT_DEPENDENCIES.md): **66**
  package/version records declared by project crates, including build/test tools.
- [Complete Cargo inventory](dependencies/CARGO_LIBRARIES.md): **646**
  package/version/source records, **528** distinct crate names, including transitives.
- [Machine-readable CSV](dependencies/cargo-licenses.csv): the same complete list,
  with licenses, source, workspace, target and direct dependants.

The twenty-three workspaces cover the application, public viewer/filesystem/thumbnail/image/web
APIs and worker SDK, six independent viewer plugins, archive/image/video plugins,
the independent native Windows/Linux/macOS Browse/Read filesystem packages,
the independent Windows/Linux Network Neighborhood plugin, and the
independent viewer ABI conformance project and vendored renderer tests. Project-owned
crates are excluded. The negative descriptor fixture has no third-party dependencies.
Target graphs include normal, build and test dependencies; this is broader
than the libraries linked into `me.exe`/`me`. Unused lockfile entries and optional
packages present only in Cargo metadata's feature union are excluded using
`cargo tree`. The complete list links to each workspace's maintained lockfile.

The scrolling renderer correction vendors the same FemtoVG **0.27.0** under its
MIT alternative, retaining both upstream license texts and
[exact source provenance and patch details](../vendor/README.md). Application
package/version selections are unchanged. The same local source also guards
feature-specific imports/helpers so the app's `swash,wgpu` feature set compiles
without unused-item warnings; no dependency, license or feature selection changes.
Its lockfile changes only the
renderer source from the registry to the local patch. The `renderer-tests` scope
accounts for the independent vendored test workspace with
`--no-default-features --features wgpu`, including its normal, build and development
dependencies. The standalone lock retains upstream pins except the Windows API
family compatibility correction documented with the vendor source. The broader
test graph adds 95 net inventory records and 17 crate names; these are not new
application runtime dependencies.

Application and renderer-test metadata/tree pairs were captured on Windows and
WSL Ubuntu 24.04 on 11 November 2026; the other forty target pairs retain their
previous capture provenance. The renderer-only `harfrust` 0.5.2 archive declares
MIT in its manifest/README and notes its HarfBuzz Old MIT origin, but omits a root
license file. `resource_list_proc_macro` 0.6.1 declares CC0-1.0 and shares its
recorded upstream commit with `resource` 0.6.1, whose workspace archive includes
the complete CC0 text. These published-source details do not change their
verbatim declarations in the generated inventory.

All entries have a declared license. Alternatives are retained verbatim; they do
not mean all alternatives are selected. The mandatory free-of-charge requirement
and Slint's selected royalty-free branch are in [the policy](DEPENDENCY_POLICY.md).

Normal Linux HTML builds also require CMake and Ninja. The 13 November Ubuntu
24.04 build container used CMake `3.28.3-1build7` (upstream BSD-3-Clause, retaining
its component notices) and Ninja `1.11.1-2` (Apache-2.0, retaining its bundled
component notices). The installed distribution copyright files were inspected;
both tools permit the intended build use without mandatory fees. They are build
tools rather than libraries shipped with ME. The exact version/license capture is
`_build/qa/main-integration-20261113/html-build-tools.txt` and its adjacent package
version receipt. CEF's existing runtime notices remain separate.

The HTML provider, included by normal builds, adds pinned `cef` / `cef-dll-sys`
`156.4.0+156.0.26`, with only `sandbox` enabled and MIT selected for the Rust
bindings. Its separately packaged Chromium Embedded Framework runtime is
`156.0.26+g8ea0c73+chromium-156.0.8078.88`. The main executable does not link CEF.
CEF's BSD license does not replace Chromium's bundled-component licenses;
the package retains its full `LICENSE.txt`, generated `CREDITS.html`, and the
locked Rust packages' copyright/license files. LGPL, MPL, Unicode/data and
other component conditions still apply. See the provider's
[pinned runtime, licenses and source-availability record](../plugins/midnight-viewer-html/THIRD_PARTY.md).
No paid CEF wrapper, Widevine or proprietary codec build option is selected.
The new Windows/Linux target Cargo selections were captured from the Windows
builder, including its build-tool dependencies; this is dependency accounting,
not a claim of native Linux Chromium qualification.

The Linux HTML package prepared on 10 November additionally supports explicitly
bundled Ubuntu 24.04 amd64 runtime libraries when the host does not supply them:

| Native package | Exact package version | License in the pinned package |
| --- | --- | --- |
| NSS (`libnss3`) | `2:3.98-1ubuntu0.3` | MPL-2.0; bundled components retain Zlib, BSD and other notices |
| NSPR (`libnspr4`) | `2:4.35-1.1build1` | MPL-2.0 |
| ALSA (`libasound2t64`) | `1.2.11-1ubuntu0.3` | LGPL-2.1-or-later |

These are separate dynamically loaded native libraries, not additions to the
Cargo inventory or the main executable. The package preserves their exact binary
hashes, original copyright files, complete license texts and matching original
source archives plus distribution patches/build scripts under `sources/native/`.
Retain that bundle when distributing this variant; compatible rebuilt shared
libraries remain replaceable. Source details and the verified CEF archive hashes
are in the [HTML provider record](../plugins/midnight-viewer-html/THIRD_PARTY.md).
No new Rust dependency or feature is needed for Linux's private browser socket
directory or for native tree-watch batching.

The streaming-worker SDK reconciliation on 9 November upgrades only project-owned
packages: worker SDK 0.2.0, image-viewer SDK 0.3.0, thumbnail SDK 0.2.0 and
filesystem API/SDK 0.3.0. Twenty metadata/tree pairs were recaptured on native
Windows and Linux for the application, affected public API workspaces and five
independent archive/media consumers. The external dependency records and enabled
features remain unchanged; regeneration and `--check` retain 490 records, 458
names and 62 direct records, with byte-identical generated inventory tables.
The existing MIT alternatives for windows-sys 0.61.2 and libc 0.2.191 were
rechecked against their pinned license files. Image/PDF notices cover both target
graphs; refreshed thumbnail notice staging now includes the Windows/Linux union
(36 image and 29 video records), including Linux-only libc/cfg-if notices.
FFmpeg's exact `build-options.txt`, corresponding source and configuration survive
release staging unchanged. See the [SDK consumer and notice verification](
verification/2026-11-09-worker-sdk-media-consumers.md).

Custom archive suffixes reuse the already pinned `unicase` 2.10.0 in
`midnight-core` as well as `midnight-engine`. Its exact manifest and MIT notice
were inspected; MIT is the selected branch, and no optional features or additional
packages are enabled. Release notice generation already retains this package's
copyright and license files. The direct-dependant inventory includes both crates.

The native filesystem host adds only project-owned API dependencies and new
dependants of already pinned serde, toml and test-only tempfile. The application
graphs were recaptured on Windows and Linux on 8 November; other workspace
dependency selections are unchanged. Regeneration and `--check` retain the same
490 package/version/source records, 458 names and 60 direct records.

The subsequent Windows renderer correction enables Slint's `unstable-wgpu-30`
configuration API to select DX12 explicitly. WGPU 30.0.1 and rgb 0.8.53 were
already selected; their MIT licenses were checked. Resolution preserves all
610 existing lockfile package pins and adds only optional Android metadata:
`android-build` 0.1.4 (MIT) and `i-slint-backend-android-activity` 1.18.1 (the same
Slint Royalty-free 2.0 alternative). Their pinned license files were inspected.
Neither Android package nor Skia enters the Windows/Linux selected build graphs.
Earlier Browse/Read native Windows captures cover additional modules of pinned
windows-sys 0.61.2 under its MIT alternative, with the full notice retained.
The targeted prepared-Rename capture below updates that source selection.

The integrated prepared-Rename native Windows **0.2 source** adds runtime features
`Wdk_Foundation`, `Wdk_Storage_FileSystem`, `Win32_Globalization`,
`Win32_Security`, `Win32_Security_Cryptography` and `Win32_System_Threading`
to pinned windows-sys **0.61.2**, with defaults disabled. Its normal feature tree
has **19** namespace features; the test/compiled union has **20**, adding only
`Win32_System_LibraryLoader`. windows-link remains **0.2.1**. The selected **MIT**
alternative and complete retained notices are unchanged; no separate cryptographic
library or new third-party package/version is introduced.

The targeted Windows-host/Windows-target metadata/tree captures select windows-sys
and windows-link for `native-windows`, and no third-party package for
`native-linux`. The Linux-target libc **0.2.191** `std` declaration/checksum is
unchanged; no new Linux-host capture or Linux execution is claimed. Both plugins'
own 0.1-to-0.2 versions are project-owned and excluded. The targeted package/row
comparison preserves the generated package/license tables and their recorded
provenance: **646 records / 528 names / 66 direct records**. This is a feature
refresh, not a full 46-pair recapture or generator `--check`.

Windows/Linux native 0.2 source is integrated in D merge `5baee471` from C commit
`42d45def` (25 matching paths). The plugins are not deployed and engine/UI cutover
remains unimplemented; installed-provider/native-runtime qualification is separate. See
[provider status](ACTIVE_WORK.md#provider-increments-from-the-architect-review).

## Main functional libraries

Optional executable-size diagnostics use `cargo-bloat` **0.12.1** (MIT), installed
only into `.tools/cargo-bloat`; its build output stays under `_build/tools/`.
It is not an application dependency or a distributed runtime component. The
[diagnostic tool inventory](dependencies/CARGO_BLOAT.md) records its pinned Windows
dependency graph separately from the application/plugin inventories above.

This table is a guide; the linked inventories above contain every selected crate.

| Library | Purpose | Declared license |
| --- | --- | --- |
| Slint 1.18.1 | UI, compiler and render integration | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0; royalty-free desktop branch selected |
| Tokio 1.54.1 | Async backend orchestration | MIT |
| serde 1.0.229 / serde_json 1.0.154 / toml 0.9.12 | Data/settings/manifest serialization | MIT OR Apache-2.0 |
| sha2 0.10.9 | SHA-256 copy/readback verification, protected Windows plugin runtimes and standalone SHA-256/SHA-512 checksum backend | MIT OR Apache-2.0 |
| md-5 0.10.6 / sha1 0.10.6 | MD5/SHA-1 compatibility calculations in the standalone checksum backend; not authentication or copy verification | MIT OR Apache-2.0; MIT branch selected |
| crc32fast 1.5.4 | SFV/CRC32 compatibility calculations; also an existing PNG/compression dependency | MIT OR Apache-2.0; MIT branch selected |
| notify 8.2.1 | Filesystem event watching | CC0-1.0 |
| image 0.25.10 / png 0.18.1 | Icons, isolated image thumbnails and the independent image viewer | MIT OR Apache-2.0 |
| zip 8.6.0 | Read-only ZIP archive provider | MIT |
| hayro 0.9.0 and its parser/interpreter crates | Isolated PDF rendering | Apache-2.0 OR MIT; pinned notices and embedded font/CMap licenses accompany the PDF plugin |
| similar 2.7.0 | Bounded text comparison | Apache-2.0 |
| sevenz-rust2 0.23.0 / lzma-rust2 0.21.0 | Read-only 7z and XZ decoding | Apache-2.0 |
| bzip2 0.6.1 / libbz2-rs-sys 0.2.6 | Pure Rust BZIP2 stream decoding | MIT OR Apache-2.0 / bzip2-1.0.6 |
| ruzstd 0.9.1 / twox-hash 2.1.5 | Zstandard decoding and frame checksums | MIT |
| tar 0.4.46 / flate2 1.1.10 and 1.1.11 | TAR/GZIP archive decoding, SDK packaging and image codec dependencies | MIT OR Apache-2.0 |
| blake3 1.8.7 | Thumbnail cache identities and integrity records | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| ffmpeg-next 9.0.0 / ffmpeg-sys-next 9.0.1 | Required isolated video thumbnail decoder bindings | WTFPL |
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

The checksum backend pins the digest-0.10-compatible hash releases and enables
only `std`. Optional assembly, OID, compression and force-soft features are not
enabled. Exact released source archives and both license texts were inspected;
MIT copyright/license notices accompany distribution. SHA-1 0.10.7 was also
inspected: its only patch change disables an optional assembly feature that this
project does not enable. The Ctrl+F8 window is implemented and has separate
[UI qualification](verification/2026-11-07-checksum-window.md); native hashing and
manifest evidence is in [backend verification](verification/2026-11-07-checksum-backend.md).

SFV checksum support reuses the already locked `crc32fast` 1.5.4 as a direct
engine dependency with only `std` enabled. Its released manifest and full MIT
license were inspected; MIT is the selected branch. The only runtime dependency
is the already selected `cfg-if` 1.0.5. The release notice collector retains the
existing `crc32fast` copyright and license files under `licenses/crc32fast-1.5.4/`;
no new package, paid alternative or nightly feature is introduced.
The Windows and native Linux application graphs were recaptured and the complete
inventory regenerated and checked: 490 package records and 458 names remain,
with 61 direct package/version records after promoting `crc32fast` to a direct
dependency.

The native Browse/Read API and author SDK use only `std`. Independent Windows
and Linux filesystem packages reuse windows-sys 0.61.2/windows-link 0.2.1 and
libc 0.2.191 under their inspected MIT alternatives. Windows runtime features are
Foundation/Storage_FileSystem; standalone dynamic-loader tests additionally use
System_LibraryLoader. Linux enables only libc `std`. Each package includes the
complete selected MIT notice; no dependency version or mandatory fee was added.

The independent Network Neighborhood plugin adds the `network` inventory scope.
Windows reuses windows-sys 0.61.2 (defaults disabled; Foundation and WNet with their
parent features) and windows-link 0.2.1 under MIT. Linux lazily loads GIO through
libloading 0.8.9 (ISC) and cfg-if 1.0.5 (MIT), without optional features. Their
exact pinned manifests and license texts were inspected; all four already exist
in the inventory. Native-host Windows/Linux metadata and Cargo tree captures
record the separate target graphs. Promoting libloading to a direct dependency
raises the direct count to 62; the complete inventory remains 490 records and 458
names. Full selected license texts and runtime notes
ship inside the package's [THIRD-PARTY.md](../plugins/midnight-filesystem-network/THIRD-PARTY.md).
Optional GLib/GIO and GVfs/backends/FUSE are system-provided and are not bundled.
GVfs Windows discovery also uses the optional system `wsdd` helper. Ubuntu
`wsdd` 2:0.7.1-5 (upstream 0.7.1) and its Debian packaging were inspected under
MIT, with system Python 3 as the only declared dependency and no optional build
features. It is not bundled or added to the Cargo inventory; its full runtime
provenance is recorded in the network package's THIRD-PARTY.md.
The inspected GLib package is LGPL-2.1-or-later with per-file notices; GVfs 1.54.4
COPYING is GNU Library GPL v2, with inspected backend/client headers allowing later
versions. Any future native runtime bundle needs its own full license, backend
dependency and corresponding-source audit. Current fixture checks do not qualify
live SMB/FUSE interoperability.

The independent native macOS package also uses libc 0.2.191 with only `std` and
the public, std-only filesystem API/SDK. Its exact cached manifest/features and
full MIT license were inspected; retain the package's
[THIRD-PARTY.md](../plugins/midnight-filesystem-native-macos/THIRD-PARTY.md) with
every distribution. Its separate Darwin x86-64 and arm64 captures add the
`native-macos` scope and direct dependant to the existing libc record, without
adding a package/version or changing license choices. The existing Windows/Linux
captures are preserved. These target-filtered metadata/tree captures were made
on Windows; this graph has no additional build or procedural-macro dependencies.
They do not establish Darwin linking, runtime behavior or main-application support;
see the package's [qualification limits](../plugins/midnight-filesystem-native-macos/VERIFICATION.md).

The archive provider supports bounded ZIP/7z/TAR and GZIP/XZ/BZIP2/Zstandard streams,
including compressed TAR variants, with read-only browsing and host-verified copy-out;
its concrete limits are in [the filesystem API](developer/FILESYSTEM_API.md).
The source viewer currently uses custom lexical
profiles with `regex`, not a comprehensive grammar engine such as syntect or
Tree-sitter. Media preview choices and implementation status are in the
[thumbnail design](superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).

## Native libraries, runtime and data outside Cargo

Linux settings Browse uses the desktop FileChooser portal through zbus 5.19.0
(MIT), futures-lite 2.6.1 and Tokio (both MIT OR Apache-2.0). The Tokio-backed
portal enables socket2 0.6.5 (MIT OR Apache-2.0) and bytes 1.12.1 (MIT). These
licenses were checked in the pinned Cargo manifests; no paid service is involved.
Windows Browse uses the existing Windows Common Item Dialog bindings. Runtime
distribution notices are generated from the target's selected dependency graph.

These are distinct from the Cargo inventory. Most are supplied by the user's OS;
versions, transitive dependencies and graphics drivers vary by installation.

| Component | Use | Upstream license family / provenance |
| --- | --- | --- |
| FFmpeg 9.0.2 shared libraries | Required isolated video thumbnails; pinned source/build, not linked into the main executable | LGPL-2.1-or-later in the supplied configuration: GPL, nonfree, version3 and network disabled. Matching source, configuration and notices accompany the four runtime libraries |
| LLVM/libclang 18.1.1 wheel or system development package (Ubuntu 18.1.3 in this Debug build) | Build-time video binding generation, not shipped as application runtime | Apache-2.0 WITH LLVM-exception; retained upstream license or distribution copyright files |
| GnuPG system package (2.4.4 in this build container) | Build-time FFmpeg detached-signature verification in an isolated keyring; not bundled with ME | GPL-3.0-or-later; retain distribution package notices when redistributing the build environment |
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
| Darwin/libSystem | Independent native macOS filesystem package prerequisite; not bundled | Apple platform terms; the libc Rust binding's MIT license does not relicense macOS |
| Rust standard library/runtime | Application platforms and independent macOS filesystem package | MIT/Apache-2.0 plus component-specific notices; releases include `licenses/rust/COPYRIGHT-library.html` |
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
`x86_64-pc-windows-msvc` and Linux for `x86_64-unknown-linux-gnu`, except
`native-macos`, which requires only `x86_64-apple-darwin` and
`aarch64-apple-darwin`. Do not add Darwin captures for the application or other
workspaces. Name the outputs
`<scope>-<target>.json` and `<scope>-<target>.tree` in `_build/verification/dependency-inventory/`.

| Scope | Manifest |
| --- | --- |
| app | `Cargo.toml` |
| renderer-tests | `vendor/femtovg/Cargo.toml` |
| api | `api/Cargo.toml` |
| basic | `plugins/midnight-viewer-basic/Cargo.toml` |
| source | `plugins/midnight-viewer-source/Cargo.toml` |
| markdown | `plugins/midnight-viewer-markdown/Cargo.toml` |
| conformance | `plugins/viewer-conformance/Cargo.toml` |
| filesystem-api | `filesystem-api/Cargo.toml` |
| native-windows | `plugins/midnight-filesystem-native-windows/Cargo.toml` |
| native-linux | `plugins/midnight-filesystem-native-linux/Cargo.toml` |
| native-macos | `plugins/midnight-filesystem-native-macos/Cargo.toml` |
| network | `plugins/midnight-filesystem-network/Cargo.toml` |
| worker-api | `worker-api/Cargo.toml` |
| thumbnail-api | `thumbnail-api/Cargo.toml` |
| image-viewer-api | `image-viewer-api/Cargo.toml` |
| web-viewer-api | `web-viewer-api/Cargo.toml` |
| html-viewer | `plugins/midnight-viewer-html/Cargo.toml` |
| image-viewer | `plugins/midnight-viewer-image/Cargo.toml` |
| image-decoder | `plugins/image-decoder/Cargo.toml` |
| pdf-viewer | `plugins/midnight-viewer-pdf/Cargo.toml` |
| archive | `plugins/midnight-fs-archive/Cargo.toml` |
| thumbnail-image | `plugins/midnight-thumbnail-image/Cargo.toml` |
| thumbnail-video | `plugins/midnight-thumbnail-video/Cargo.toml` |

Example PowerShell capture for one scope/target; repeat using the table. Omit
`--config $sdkConfig` for application, API, native and network filesystem workspaces.
These filesystem packages use only the checked-in public API/SDK. Viewer plugins use the
released viewer bundle; archive and thumbnail plugins use the separate public
worker bundle described in [the worker SDK](developer/WORKER_SDK.md). Supply the
matching bundle's absolute config path; plugin lockfiles pin its package checksums.

```powershell
$scope = 'app'
$manifest = 'Cargo.toml'
$target = 'x86_64-pc-windows-msvc'
$captures = '_build/verification/dependency-inventory'
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

For `renderer-tests`, append `--no-default-features --features wgpu` to both
Cargo commands. Its independent lockfile and development edges are part of the
capture.

On Linux use `sh scripts/cargo.sh` and UTF-8 shell redirection instead of
PowerShell's `Set-Content`; use `python3` where Python is installed under that name.
`cargo metadata`/`tree` do not compile native code. Capture each target on its
native host: build dependencies/procedural macros follow the host, so a Windows
cross-target query is not equivalent to the native Linux graph.
The native macOS package's two added captures are an explicit cross-target
exception: its current libc/public-API/SDK graph has no additional build or
procedural-macro dependencies. Recapture on a Mac if that graph gains such
dependencies; do not extrapolate its graph to the application. Metadata/tree
success never establishes native runtime support.
The generator requires forty-six metadata/tree pairs across its twenty-three
scopes, including `image-decoder`; it excludes project-owned
API/SDK crates, and writes deterministic Markdown/CSV. Explicitly marked vendored
third-party workspace members remain included. Recapture after every dependency
or feature change. `--check` verifies tables against the supplied captures; it
does not prove that those captures are current. Current source hashes are not
used as a substitute for capture provenance. The four application/renderer pairs
were refreshed on 11 November 2026; other pairs retain their documented native-
host or macOS cross-target provenance. A complete current input set is required
for regeneration.

The generator emits package/license/scope records, not enabled-feature columns.
Retain the affected package's `resolve.nodes[].features` with each metadata
capture. The older [feature/license CSV](third-party-inventory.csv) belongs to the
[dated main-branch audit](verification/main-audit.md), as the [docs index](README.md)
explains; preserve that historical snapshot rather than inserting later feature changes.

```sh
python3 -B -m unittest discover -s scripts/tests -v
python3 -B scripts/dependency-inventory.py --metadata-dir _build/verification/dependency-inventory --check
```

The 11 November MEDIA-01 follow-up advances only the first-party
`midnight-filesystem-api` package from 0.3.0 to 0.4.0 for negotiated cooperative
archive staging cancellation. The independent archive plugin consumes its new
immutable public SDK package; its lockfile changes only that package/version and
checksum. No third-party dependency, feature or license selection changed. This
note supplements the historical dependency captures above; it is not a new full
inventory capture.
