# Third-party libraries and licenses

This document records the third-party code and data that Midnight Explorer, its public
API/SDK and its independent plugins use, the license choices made for them, and how the
dependency inventory is produced. The rules are in the
[dependency policy](DEPENDENCY_POLICY.md). These records describe third-party components
only. Midnight Explorer itself is licensed under the GNU General Public License, version
3 or later ([LICENSE](../LICENSE)), and so are its plugins and public API/SDK
workspaces; the API/SDK crates' manifests gain their `license` field with their next
versions ([policy](DEPENDENCY_POLICY.md#current-decisions)). Every component listed
here is distributed under its own license.

## Inventories

- [Direct dependencies](dependencies/DIRECT_DEPENDENCIES.md): **81** package/version
  records declared by project crates, including build and test tools.
- [Complete Cargo inventory](dependencies/CARGO_LIBRARIES.md): **673**
  package/version/source records with **551** distinct crate names, including
  transitive dependencies.
- [Machine-readable CSV](dependencies/cargo-licenses.csv): the complete inventory with
  license, source, workspaces, targets and direct dependants. The About dialog is built
  from it ([distribution notices](#distribution-notices)).
- [Lock file entries outside the inventory](dependencies/LOCKFILE_ONLY.md): **196**
  records that the lock files pin but no captured target selects, such as dependencies
  for Android, Apple platforms or WebAssembly and optional dependencies that are off.
- [Office preview notices](dependencies/OFFICE_PREVIEW_NOTICES.md) and the separate
  inventory of the [cargo-bloat diagnostic tool](dependencies/CARGO_BLOAT.md).

The inventory covers 23 Cargo workspaces: the application, the public API/SDK
workspaces, the independent viewer, filesystem, archive, image and video plugins, the
viewer ABI conformance project and the vendored FemtoVG renderer tests
([list](#regenerating-the-inventory)). For each workspace it lists the third-party
packages that Cargo selects for Windows x64 and Linux x86-64 through normal, build and
development edges; the native macOS filesystem package is captured for macOS x86-64
and arm64 instead. This is broader than the code a release links. Project-owned crates
are excluded, including released API/SDK versions from the project registry. The
vendored FemtoVG, Slint FemtoVG adapter and image-rs sources are third-party code and are
listed as local path dependencies.

Licenses are the upstream Cargo declarations, verbatim. `OR` lists alternatives, not a
selection, and `AND` combines obligations; old slash syntax is kept as declared. Every
record declares a license, and no dependency requires a fee, subscription or royalty.

Last checked on 22 November 2026 against the lock file of every inventory workspace,
without running Cargo. Resolving each lock file from the pinned crates' manifests for
the captured targets and features yields exactly the records, workspaces, targets and
direct dependants of the three tables. Each record's license is the pinned crate's own
declaration, and the SHA-256 of each crate archive in the local Cargo cache equals the
lock file's checksum. A regeneration from Cargo's own captures
([below](#regenerating-the-inventory)) remains the reference. The tables were last
regenerated from the dependency-inventory workflow's Windows and Linux captures of
`0ccc198d` (25 November 2026): they record the vendored `i-slint-renderer-femtovg`
1.18.1 as a local path dependency with `midnight-ui-input` as a direct dependant
([record](verification/2026-11-25-dependency-inventory-workflow.md)).

## License notes

### Update check

The update check (`midnight-update`, [Updates](developer/UPDATES.md)) added on 23
November 2026: `minisign-verify` 0.3.0 (MIT; no dependencies, its own Ed25519 and
BLAKE2b) on both systems; on Linux only, `ureq` 3.5.2 (MIT OR Apache-2.0) with the
features `rustls-no-provider` only, `rustls` 0.23.46 (Apache-2.0 OR ISC OR MIT) with
`ring`, `std`, `tls12` and `logging`, `ring` 0.17.14 (Apache-2.0 AND ISC; its
`LICENSE`, `LICENSE-BoringSSL` and `LICENSE-other-bits` checked: the BoringSSL-derived
code is Apache-2.0 or ISC, with no OpenSSL or SSLeay license text, and its `once_cell`
polyfill is MIT OR Apache-2.0), `rustls-native-certs` 0.8.5 (Apache-2.0 OR ISC OR MIT)
with `openssl-probe` 0.2.2 (MIT OR Apache-2.0, which only locates certificate files and
links no OpenSSL), `rustls-webpki` 0.103.17 and `untrusted` 0.9.0 (ISC), and
`rustls-pki-types`, `http`, `httparse`, `ureq-proto`, `utf8-zero` and `getrandom` 0.2.17
(MIT OR Apache-2.0). No certificate bundle (`webpki-roots`) and no OpenSSL are linked;
Windows uses WinHTTP through the existing `windows-sys`. `semver`, `sha2`, `zip`,
`flate2`, `tar` and `serde_json` were already in the lock file. Tests only:
`ed25519-compact` 2.6.1 (MIT, default features off) and `blake2` 0.10.6 (MIT OR
Apache-2.0) sign throwaway test keys. `schannel`, `security-framework` and
`core-foundation` 0.10.1 are lock-file entries that no captured target selects. The
inventory tables were updated for the application workspace from Cargo trees of both
targets before and after the change; the other workspaces are unchanged.

### Slint

Slint 1.18.1 is used under the **GPL-3.0-only** branch of its
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`
declaration, which matches the application's GPL license. The About dialog keeps Slint's
AboutSlint attribution and release pages the Made with Slint badge. The decision is in
the [dependency policy](DEPENDENCY_POLICY.md#current-decisions).

Slint's FemtoVG renderer crate, i-slint-renderer-femtovg 1.18.1, is vendored with a
present-mode hook that the application sets on native Wayland. The modified copy is
conveyed under the same GPL-3.0-only branch; see the
[vendor notes](../vendor/README.md#slints-femtovg-wgpu-adapter).

### Async runtime (Tokio)

Tokio 1.54.1 (MIT) runs the backend's asynchronous work. The workspace declaration
selects `rt-multi-thread`, `sync`, `time` and `macros`; Tokio's default feature set is
empty. `midnight-work-budget` declares Tokio as an optional dependency behind its
`resident-workers` feature, which the engine, viewer engine, compare engine, compare
UI, viewer UI and application crates enable. Version 1.54.1 contains the fixes for
[RUSTSEC-2026-0384](https://rustsec.org/advisories/RUSTSEC-2026-0384.html) and
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html).

### Crates published without license files

Some crate archives omit their license files. The repository keeps the upstream texts
from the commit each crate records, with source URLs and SHA-256 hashes, in
`packaging/licenses/NAME-VERSION/`
([example](../packaging/licenses/accesskit-0.24.1/SOURCE.md)). Pass that folder as the
release packager's `--license-overrides`; it uses a supplement only for a crate that has
no license file of its own.

- The eight AccessKit packages (`accesskit` 0.24.1 and its adapters): Apache-2.0 is
  selected; the MIT and Chromium BSD texts are kept as well.
- clipboard-win 5.4.3: BSL-1.0.
- gl_generator 0.14.0, khronos_api 3.1.0 and spirv 0.4.0: Apache-2.0. khronos_api also
  keeps the ANGLE BSD license and the Khronos, EGL and WebGL notices it embeds.
- lyon_algorithms, lyon_extra, lyon_geom, lyon_path and profiling: the MIT and
  Apache-2.0 texts.
- rspolib 0.1.2 and taffy 0.10.1: MIT.
- ffmpeg-sys-next 9.0.1: the canonical WTFPL text ([video](#video)).

Two renderer-test packages have no supplement because no release ships them:
harfrust 0.5.2 declares MIT without a license file, and resource_list_proc_macro 0.6.1
declares CC0-1.0, whose text is in the resource 0.6.1 archive of the same upstream commit.

### Vendored sources

- FemtoVG 0.27.0 (`vendor/femtovg`, MIT OR Apache-2.0) replaces the crates.io release in
  the application through `[patch.crates-io]`; see the [vendor notes](../vendor/README.md).
- i-slint-renderer-femtovg 1.18.1 (`vendor/i-slint-renderer-femtovg`, GPL-3.0-only OR
  LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0; GPL-3.0-only branch
  used) replaces the crates.io release in the application through `[patch.crates-io]`;
  see the [vendor notes](../vendor/README.md#slints-femtovg-wgpu-adapter).
- image-rs 0.25.10 (`vendor/image`, MIT OR Apache-2.0) replaces the crates.io release in
  the image decoder, image viewer and image thumbnail workspaces; see the
  [patch provenance](../vendor/image/MIDNIGHT_PATCH.md). The application and the PDF
  viewer use the unpatched crates.io release.

All three keep their upstream license files, and the release packager copies them.

### Images

The image viewer and the image thumbnailer share `plugins/image-decoder`: image-rs with
PNG, JPEG, GIF, WebP, BMP, ICO, TIFF and AVIF, moxcms 0.8.1 (BSD-3-Clause OR Apache-2.0)
for color management, and resvg/usvg 0.48.1 (Apache-2.0 OR MIT) with `text` and
`raster-images` for static SVG. MIT is selected for image-rs, resvg and usvg; tiny-skia
0.12.0 keeps its BSD-3-Clause notice. SVG rendering opens no files and uses no network;
embedded `data:` images are decoded by image-rs and handed to resvg as PNG.
Unmodified Noto Sans Regular/Arabic/Hebrew font bytes are embedded under OFL-1.1
([source and hashes](../plugins/image-decoder/fonts/README.md)). AVIF uses the MIT
dav1d 0.11.0 and dav1d-sys 0.8.3 bindings, BSD-2-Clause native dav1d 1.5.4 and MPL-2.0
mp4parse 0.17.0. Both worker packages copy the shared `plugins/image-decoder/notices`
folder, which holds dav1d's license and the unmodified mp4parse source archive with its
license; see the [image viewer's notices](../plugins/midnight-viewer-image/THIRD-PARTY.md).
HEIC/HEIF is not supported.

The application uses the crates.io image-rs with only PNG enabled (default features
off). On Linux, `midnight-platform-linux` renders SVG icon-theme artwork with resvg
0.48.1 (default features off) and tiny-skia 0.12.0; it loads no fonts, has no resource
folder and disables both image resolvers.

### PDF

The PDF viewer and the image thumbnailer (first-page PDF thumbnails) render with hayro
0.9.0 (Apache-2.0 OR MIT) and its `embed-fonts` and `embed-cmaps` features. hayro-font
0.5.0 embeds the Foxit standard PDF fonts and hayro-cmap 0.1.1 the Adobe CMaps, both
under BSD-3-Clause. Both packages keep those notices, the CC0 color-data and JPEG 2000
profile notices, Hayro's NOTICE and alloc-stdlib 0.2.4's BSD-3-Clause license; see the
[PDF viewer's notices](../plugins/midnight-viewer-pdf/THIRD-PARTY.md).
`scripts/thumbnail-notices.py` adds the thumbnailer's copies from its own `notices`
folder.

### HTML viewer

The HTML viewer uses `cef` and `cef-dll-sys` `156.4.0+156.0.26` with only the `sandbox`
feature; MIT is selected for these bindings. Its separately packaged Chromium Embedded
Framework runtime is `156.0.26+g8ea0c73+chromium-156.0.8078.88`. The main executable
does not link CEF. CEF's BSD license does not cover Chromium's bundled components: the
package keeps CEF's `LICENSE.txt` and Chromium's `CREDITS.html`, whose LGPL, MPL and
other conditions apply. It also carries the HTML viewer's own `LICENSE`; CEF's
`LICENSE.txt` covers only CEF. No Widevine, proprietary codec or paid component is selected.
The Linux package can also carry native runtime libraries named in a manifest
(`MIDNIGHT_HTML_NATIVE_MANIFEST`) for hosts that lack them, with their notices and
sources. See the [HTML viewer record](../plugins/midnight-viewer-html/THIRD_PARTY.md).
Building the CEF wrapper needs CMake (BSD-3-Clause) and Ninja (Apache-2.0); they are
build tools, not shipped.

### Video

The bundled video helpers use FFmpeg 9.0.2 shared libraries under LGPL-2.1-or-later,
configured with GPL, nonfree, version3, networking and autodetection disabled. They
enable the VP8, VP9, Theora, FFV1, raw video and Opus decoders and the Matroska/WebM,
AVI, Ogg and MOV/MP4 demuxers. The video package keeps FFmpeg's source archive, its
LGPL text and the exact build configuration (`build-options.txt`, `config.h`,
`config.mak`) in `notices/ffmpeg-9.0.2/`, so the libraries stay replaceable. The
bindings are ffmpeg-next 9.0.0 and ffmpeg-sys-next 9.0.1 (WTFPL); bindgen 0.73.2
(BSD-3-Clause) and libclang generate them at build time.

The platform helpers (`system` feature) bundle no decoder. On Windows they use Media
Foundation through windows and windows-core 0.62.2 (MIT OR Apache-2.0). On Linux they
load the distribution's FFmpeg or GStreamer at run time with libc 0.2.191 and add no
other crate. The [dependency policy](DEPENDENCY_POLICY.md#current-decisions) records
both decisions. `scripts/thumbnail-notices.py` writes the video package's notices for the
bundled and the platform helpers together.

### Archives

The archive plugin reads ZIP with zip 8.6.0 (`deflate-flate2`, and `aes-crypto` for
WinZip AES), 7z with sevenz-rust2 0.23.0 (`aes256` for 7-Zip AES), XZ with lzma-rust2
0.21.0 (Apache-2.0), BZIP2 with bzip2 0.6.1 and libbz2-rs-sys 0.2.6 (bzip2-1.0.6),
Zstandard with ruzstd 0.9.1 (MIT), and TAR and GZIP with tar 0.4.46 and flate2 1.1.10
(`rust_backend`). The RustCrypto crates that the AES features add (aes, cbc, cipher,
hmac, pbkdf2, sha1 0.11, sha2 0.11 and their helpers) are MIT OR Apache-2.0;
constant_time_eq 0.4.2 is CC0-1.0 OR MIT-0 OR Apache-2.0. zeroize 1.9.1
(Apache-2.0 OR MIT, `alloc` only) clears passwords in `midnight-core` and in the public
`midnight-filesystem-api`. See the plugin's
[THIRD-PARTY.md](../plugins/midnight-fs-archive/THIRD-PARTY.md).

### Office preview

The Office contents preview uses quick-xml 0.41.0 (MIT), zip 8.6.0 (MIT, only
`deflate-flate2`) and flate2 1.1.10 (only `rust_backend`), all with default features
off. MIT is selected where offered; foldhash 0.2.0 keeps its Zlib license. The selected
texts are in the [Office preview notices](dependencies/OFFICE_PREVIEW_NOTICES.md).

### Filesystem plugins

- Native Windows: windows-sys 0.61.2 with default features off; MIT is selected.
- Native Linux and macOS: libc 0.2.191 with only `std`; MIT is selected. The macOS
  package's limits are in its [verification notes](../plugins/midnight-filesystem-native-macos/VERIFICATION.md).
- Network Neighborhood: windows-sys 0.61.2 (`Win32_Foundation`,
  `Win32_NetworkManagement_WNet`) on Windows; on Linux libloading 0.8.9 (ISC) and cfg-if
  1.0.5 (MIT) load the system's GIO at run time. GLib/GIO, GVfs and its backends are
  system components and are not bundled; see the plugin's
  [THIRD-PARTY.md](../plugins/midnight-filesystem-network/THIRD-PARTY.md).
- Network sign-in uses zeroize 1.9.1 (`alloc` only, MIT selected from
  Apache-2.0 OR MIT) in the host, public filesystem plugin API and Linux network
  provider. Native filesystem packages use the same API and carry its notice.

Each plugin package carries its own `THIRD-PARTY.md` with the selected license texts.
Every independent plugin pins each registry dependency exactly to its locked version
(`scripts/tests/test_plugin_dependency_pins.py`).

### Checksums

The checksum service uses sha2 0.10.9, md-5 0.10.6 and sha1 0.10.6 with only `std`, and
crc32fast 1.5.4 with only `std` for SFV/CRC32; MIT is selected. `midnight-worker-platform`
also uses crc32fast to check private spool reads. MD5 and SHA-1 serve compatibility
checksums only, not authentication or copy verification.

### Tools

- cbindgen 0.29.4 (MPL-2.0) generates the public API's C header; it is not linked into
  any product ([policy](DEPENDENCY_POLICY.md#current-decisions)).
- cargo-bloat 0.12.1 (MIT) is an optional size diagnostic installed into
  `.tools/cargo-bloat`; it has its own [inventory](dependencies/CARGO_BLOAT.md).
- CI actions are listed with the [native components](#native-libraries-runtimes-and-data-outside-cargo).

### Known advisories

zip 8.6.0 is affected by
[RUSTSEC-2026-0429](https://rustsec.org/advisories/RUSTSEC-2026-0429.html),
[RUSTSEC-2026-0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html),
[RUSTSEC-2026-0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html) and
[RUSTSEC-2026-0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html), fixed in zip
9.0.0. All four concern `ZipArchive::extract` and `ZipStreamReader::extract`. Neither
the archive plugin nor the Office preview calls them: both read members with
`ZipArchive::by_index`, `by_index_raw` or `by_index_decrypt`. No full RustSec scan of
every lock file is recorded.

## Main functional libraries

This table is a guide; the inventories above list every selected crate.

| Library | Purpose | Declared license |
| --- | --- | --- |
| Slint 1.18.1 | UI toolkit, compiler and renderers | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0; GPL-3.0-only branch selected |
| Tokio 1.54.1 | Asynchronous backend work | MIT |
| serde 1.0.229 / serde_json 1.0.154 / toml 0.9.12 | Settings, manifests and wire data | MIT OR Apache-2.0 |
| sha2 0.10.9 | SHA-256 copy verification, plugin runtime trust, SDK packages and SHA-256/SHA-512 checksums | MIT OR Apache-2.0 |
| md-5 0.10.6 / sha1 0.10.6 | MD5 and SHA-1 compatibility checksums | MIT OR Apache-2.0 |
| crc32fast 1.5.4 | SFV/CRC32 checksums and private spool read checks | MIT OR Apache-2.0 |
| notify 8.2.1 | Linux filesystem change events (Windows uses its own `ReadDirectoryChangesW` service) | CC0-1.0 |
| image 0.25.10 / png 0.18.1 | Icons, image thumbnails and the image viewer | MIT OR Apache-2.0 |
| zip 8.6.0 | ZIP archives and Office packages | MIT |
| quick-xml 0.41.0 | Office XML | MIT |
| hayro 0.9.0 | PDF viewer and PDF thumbnails | Apache-2.0 OR MIT |
| similar 2.7.0 | Text comparison | Apache-2.0 |
| sevenz-rust2 0.23.0 / lzma-rust2 0.21.0 | 7z and XZ archives | Apache-2.0 |
| bzip2 0.6.1 / libbz2-rs-sys 0.2.6 | BZIP2 streams in pure Rust | MIT OR Apache-2.0 / bzip2-1.0.6 |
| ruzstd 0.9.1 / twox-hash 2.1.5 | Zstandard streams and their checksums | MIT |
| tar 0.4.46 / flate2 1.1.10 and 1.1.11 | TAR and GZIP archives, Office DEFLATE, SDK packages and image codecs | MIT OR Apache-2.0 |
| blake3 1.8.7 | Thumbnail cache identities and integrity records | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| ffmpeg-next 9.0.0 / ffmpeg-sys-next 9.0.1 | Bundled video thumbnail and playback decoding | WTFPL |
| bindgen 0.73.2 | Build-time FFmpeg bindings | BSD-3-Clause |
| regex 1.13.1 | Name filters, comparator and editor rules, and the source viewer's lexer | MIT OR Apache-2.0 |
| pulldown-cmark 0.13.6 | Markdown viewer | MIT |
| parley 0.11.1 / swash 0.2.10 | Viewer text layout and glyph rendering | Apache-2.0 OR MIT |
| ICU casemap / locale core 2.4.0 | Unicode case mapping | Unicode-3.0 |
| minicbor 2.3.0 | Viewer plugin wire messages | BlueOak-1.0.0 |
| windows 0.62.2 / windows-sys 0.61.2 | Windows API bindings. Custom Command output uses `Win32_System_JobObjects` and `Win32_System_Pipes`. The renderer and its latency probe use `Win32_Graphics_Gdi` and `Win32_System_LibraryLoader` to check display compatibility. Other workspace crates already enable these features. | MIT OR Apache-2.0 |
| rustix 1.1.6 | Linux system calls | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| arboard 3.7.0 | Clipboard | MIT OR Apache-2.0 |
| cbindgen 0.29.4 | Public API C header generation (tool) | MPL-2.0 |

The source viewer highlights with its own lexical profiles built on `regex`, not with a
grammar engine such as syntect or Tree-sitter.

## Native libraries, runtimes and data outside Cargo

These are not in the Cargo inventory. The operating system supplies most of them, and
their versions vary by installation.

| Component | Use | License and provenance |
| --- | --- | --- |
| FFmpeg 9.0.2 shared libraries | Bundled video thumbnail and playback helpers | LGPL-2.1-or-later in the supplied configuration; source, configuration and notices accompany the libraries |
| Native dav1d 1.5.4 | AVIF decoding in the image viewer and image thumbnails | BSD-2-Clause; the packages keep its license |
| Chromium Embedded Framework 156.0.26 | HTML viewer runtime | BSD-3-Clause, plus the component licenses in `CREDITS.html` |
| Rust standard library 1.99.1 | Every program | MIT OR Apache-2.0 with component notices; releases keep `licenses/rust/COPYRIGHT-library.html` |
| Unicode case-folding data 17.0.0 | Case-insensitive search in the text and binary viewer | Unicode-3.0; the package keeps `UNICODE-LICENSE.txt` |
| libclang | Build-time FFmpeg binding generation; not shipped | Apache-2.0 WITH LLVM-exception; the distribution's package on Linux, the LLVM that `LIBCLANG_PATH` names on Windows |
| GnuPG | Build-time check of FFmpeg's source signature in a temporary keyring; not shipped | GPL-3.0-or-later |
| CMake and Ninja | Build tools of the HTML viewer's CEF wrapper; not shipped | BSD-3-Clause and Apache-2.0 |
| GNU binutils (strip) | Linux release build tool to strip staged CEF binaries; not shipped | GPL-3.0-or-later |
| Windows Installer (`msi.dll`), `makecab.exe` and `expand.exe` | Windows release build tools of the MSI (`scripts/package-msi.py`); part of Windows, not shipped | Microsoft Windows license terms; no fee beyond Windows ([dependency policy](DEPENDENCY_POLICY.md#current-decisions)) |
| Fontconfig | Linux font discovery | MIT/X11-style notices |
| FreeType | Linux font stack | FreeType License OR GPL-2.0; keep the FTL credit when choosing FTL ([terms](https://freetype.org/license.html)) |
| GNU libc (`libc`, `libm`, loader) | Linux runtime | LGPL-2.1-or-later with exceptions ([upstream](https://sourceware.org/glibc/)) |
| GCC `libgcc_s` | Linux unwinding | GPL-3.0-or-later WITH GCC-exception-3.1 ([terms](https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html)) |
| X11/XCB libraries | Linux windows and input | MIT/X11/BSD family, per component ([X.Org notices](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/License.html)) |
| Wayland client libraries | Linux Wayland windows and Wayland drag and drop (opened at run time) | MIT family ([upstream](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/COPYING)) |
| libxkbcommon | Linux keyboard handling | MIT/ISC/X11-style terms ([license](https://raw.githubusercontent.com/xkbcommon/libxkbcommon/master/LICENSE)) |
| libudev | Linux device discovery | LGPL-2.1-or-later ([systemd](https://raw.githubusercontent.com/systemd/systemd/main/README)) |
| Vulkan loader | Linux graphics | Apache-2.0 ([license](https://raw.githubusercontent.com/KhronosGroup/Vulkan-Loader/main/LICENSE.txt)) |
| ALSA (`libasound.so.2`) | Linux audio output of video playback, opened at run time | LGPL-2.1-or-later; system library, not bundled |
| GLib/GIO, GVfs and its backends | Network Neighborhood on Linux, loaded at run time | LGPL-2.1-or-later (GLib/GIO), LGPL-2.0-or-later (GVfs); system components, not bundled |
| Windows system DLLs, UCRT, VC runtime | Windows shell, storage, UI, DirectWrite and graphics | Microsoft platform terms; the MIT/Apache Rust bindings do not change them |
| Windows Media Foundation and its decoders, including Microsoft Store codec extensions the user installed (HEVC, AV1, VP9, MPEG-2, Web Media) | Video thumbnails and playback in the Windows platform helpers, for codecs outside the bundled FFmpeg profile | Licensed by Microsoft with Windows or the extension; not bundled or redistributed, no codec royalty for the project ([dependency policy](DEPENDENCY_POLICY.md#current-decisions)) |
| The distribution's FFmpeg: libavformat, libavcodec, libavutil, libswscale and libswresample of one series, libavcodec 58 to 63 (FFmpeg 4 to 9) | Video thumbnails and playback in the Linux platform helpers, for codecs outside the bundled profile; loaded at run time, never linked | The distribution's build and license: LGPL-2.1-or-later, or GPL-2.0-or-later where GPL components are enabled (Debian's and Ubuntu's builds do); not bundled or redistributed ([dependency policy](DEPENDENCY_POLICY.md#current-decisions)) |
| The distribution's GStreamer 1.x (core, base, good, bad, ugly, libav) and the listed plugins' libraries | Fallback decoders of the Linux platform helpers; loaded at run time, never linked | LGPL-2.1-or-later for GStreamer and most plugins; the ASF demuxer LGPL-2.0-or-later; faad, dtsdec and a52dec GPL-2.0-or-later; OpenH264, AOM and dav1d BSD (OpenH264 binaries may carry Cisco's binary license); not bundled or redistributed |
| Darwin/libSystem | Prerequisite of the independent native macOS filesystem package; not bundled | Apple platform terms; the libc binding's MIT license does not change them |
| GitHub Actions `actions/checkout` v7, `actions/cache` v6, `actions/upload-artifact` v7 and `actions/download-artifact` v8 | CI steps; not shipped | MIT each ([checkout](https://github.com/actions/checkout/blob/v7.0.1/LICENSE)) |
| Swatinem/rust-cache 2.9.3 (commit `d1527c8f`) | CI cache of the root workspace's compiled dependencies; run unmodified, not shipped | LGPL-3.0 ([license](https://github.com/Swatinem/rust-cache/blob/v2.9.3/LICENSE)); see the [dependency policy](DEPENDENCY_POLICY.md#current-decisions) |

The application uses the host's fonts and icon themes besides its own artwork; apart
from the image decoder's Noto Sans fonts, no third-party font or icon pack is bundled.
Graphics drivers and external editors are not dependencies of this project.

## Distribution notices

Each application release contains `THIRD-PARTY.json`, which records the name, version,
declared license, repository, source and authors of every packaged crate, and
`THIRD-PARTY.txt`. The license and notice files of each crate are under
`licenses/NAME-VERSION/`, the Rust standard library's notices under `licenses/rust/` and
the four palette licenses under `licenses/themes/`, with
[THEME_PROVENANCE.md](THEME_PROVENANCE.md). Like the inventory, the notices
conservatively include build dependencies. The packager refuses a crate without license
files unless `--license-overrides` supplies them, and refuses links. Plugin packages
carry their own `THIRD-PARTY.md` (`THIRD_PARTY.md` for the HTML viewer) and notices,
viewer packages also their own `LICENSE`, and the packager refuses a plugin package
without them. This inventory supplements those per-package
notices; it does not replace them.

The About dialog's Licenses tab is compiled from the
[machine-readable inventory](dependencies/cargo-licenses.csv). For the running
platform it lists the records of the components a release ships: the application, the
native, network and archive filesystem plugins, the viewers, the image decoder and the
image and video workers; not the renderer tests, the conformance project, the public
API/SDK workspaces or the macOS filesystem package. It adds the bundled components
outside Cargo: the Rust standard library, FFmpeg, dav1d, CEF, the Noto Sans fonts, the
Unicode case-folding data, the Foxit standard PDF fonts, the Adobe CMaps and the four
palettes. Regenerating the inventory therefore updates the tab. Tests in `midnight-ui`
check that each listed record is still pinned by the lock file of a component that
ships it, and that the bundled versions match their records: phrases such as
`FFmpeg 9.0.2 shared libraries` in this document, the commits in
[THEME_PROVENANCE.md](THEME_PROVENANCE.md) and the Unicode version in the text and
binary viewer's notices. Keep those phrases when editing.

Opening a library reads its notices on a backend worker. Release builds read only the
`licenses` folder and `THIRD-PARTY.json` beside the program; debug builds may also read
the developer's Cargo registries. Links are never followed, every read is capped before
it starts, and one library shows at most 128 KiB of notice text. Authors come from the
package manifest that `THIRD-PARTY.json` records, otherwise from the copyright lines of
the notices.

## Regenerating the inventory

Use Python 3.11 or later and the pinned Rust toolchain. Fetch dependencies and obtain
the released API/SDK bundles as the [plugin quick start](developer/QUICK_START.md)
describes; plugin lock files pin the bundles' package checksums. Captures and generated
output belong under `_build/`.

Capture Cargo metadata and the Cargo tree for each workspace below: on Windows for
`x86_64-pc-windows-msvc` and on Linux for `x86_64-unknown-linux-gnu`, except
`native-macos`, which needs only `x86_64-apple-darwin` and `aarch64-apple-darwin`. Name
the outputs `<scope>-<target>.json` and `<scope>-<target>.tree` and put all of them in
one folder, such as `_build/verification/dependency-inventory/`.

The `dependency-inventory` workflow
([dependency-inventory.yml](../.github/workflows/dependency-inventory.yml)) does this
when a Cargo manifest, lock file or this inventory changes, and on demand. It packages
the SDK bundles, captures each target on its own host and regenerates the tables from
the captures. Tables that differ from the committed ones fail the run, whose
`dependency-inventory` artifact holds the regenerated tables (`tables/`),
their differences (`tables.diff`) and the captures. Copy the tables into
`docs/dependencies/`, then update the [lock file list](dependencies/LOCKFILE_ONLY.md)
as described below.

[capture-dependency-inventory.ps1](../scripts/capture-dependency-inventory.ps1)
(PowerShell 7), which the workflow runs, captures every scope for the host it runs on,
with the configurations and features described below. With `-IncludeMacOS` it also
captures the two `native-macos` pairs, after checking that the package still has no
build or procedural-macro dependency. It first checks that the Cargo manifests, lock
files and configuration have no local changes. It records the commit in
`capture-<target>.txt` and writes `_build/verification/dependency-inventory-<target>/`
and a zip archive of it. Without SDK configurations it reuses a current bundle that
an earlier Windows build recorded, or packages one.

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

By hand, the PowerShell capture of one scope and target follows; repeat it for each row.
The application, API, native and network filesystem workspaces need no `--config`. The
viewer plugins use the released viewer bundle; `source` and `markdown` use the
configuration that their `scripts/prepare-sdk.ps1` or `prepare-sdk.py` derives from it,
as their CI does. The archive, image, PDF and thumbnail plugins use the public worker
bundle described in the [worker SDK](developer/WORKER_SDK.md), and the HTML viewer uses
the web SDK configuration that `web-viewer-api/scripts/package.py` prints. Pass the
matching bundle's absolute configuration path as `--config`.

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

- For `thumbnail-video`, append `--features system,playback-worker` to both Cargo
  commands. Video packages ship the bundled helpers (the default `bundled` feature) and
  the platform helpers together; without these features Cargo selects none of the
  platform helpers' crates.
- For `renderer-tests`, append `--no-default-features --features wgpu` to both Cargo
  commands. Its own lock file and development dependencies are part of the capture.
- On Linux use `sh scripts/cargo.sh` and UTF-8 shell redirection instead of
  `Set-Content`, and `python3` where Python has that name.
- Capture each target on its own host: build dependencies and procedural macros follow
  the host, so a cross-target query is not equivalent. The macOS package is the
  exception, because its graph has no build or procedural-macro dependency; recapture
  it on a Mac if that changes. Metadata and tree captures compile nothing and do not
  establish runtime support.

The generator needs all 46 metadata/tree pairs of the 23 scopes. It excludes
project-owned crates, keeps vendored third-party workspace members, and writes the
three tables deterministically. `--check` compares the tables with the supplied
captures; it does not prove that the captures are current, so recapture after every
dependency or feature change:

```sh
python3 -B -m unittest discover -s scripts/tests -v
python3 -B scripts/dependency-inventory.py --metadata-dir _build/verification/dependency-inventory --check
```

After regenerating, update the [lock file entries outside the inventory](dependencies/LOCKFILE_ONLY.md):
every third-party package of a workspace's lock file must appear either in the complete
inventory for that workspace or there.

The generator records no features. Keep each capture's `resolve.nodes[].features` when
a feature change matters. The older [feature/license CSV](third-party-inventory.csv)
belongs to the dated [main-branch audit](verification/main-audit.md) and is not updated.
