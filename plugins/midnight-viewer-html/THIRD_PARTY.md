# Chromium provider dependencies and distribution notices

This independent workspace consumes only the published `midnight-web-viewer-api`
0.1.0 contract. The public API retains the repository's terms; this plugin does
not assign or change them. No private host/application crate is linked.

The exact Rust lock pins `cef` and `cef-dll-sys` to
`156.4.0+156.0.26`. Both select the MIT branch of MIT OR Apache-2.0. CEF's sole
enabled optional feature is `sandbox`; default features are disabled. The native
pair is CEF `156.0.26+g8ea0c73+chromium-156.0.8078.88` (Windows x64 minimal).
The Windows runtime archive has 175046254 bytes, published SHA-1
`15d29eea209d857c22dc163706080295aea69b5c` and independently calculated SHA-256
`2b0aa070793a6bb858b737f5edf9cb01a3ef7b9adcad5efe1e08affaf250f809`.

The matching Linux x64 minimal archive has 333351579 bytes, published SHA-1
`cacfbb7d7261fea682fb67c108de7e4347519ff7` and independently calculated SHA-256
`600ac0b8f37cb953f884e7c89998a6c200d24959aa74820ad431b61d5ebcf360`.
`scripts/cef-linux-pinned.json` records SHA-256 for every flattened SDK input,
generated only after verifying this exact official archive. The Linux helper
validates the full input inventory before Cargo and preserves the actual Linux
LICENSE/CREDITS; Rust bindings do not relicense those components.

CEF's pinned `LICENSE.txt` is BSD-style and must be distributed with its complete
`CREDITS.html`. Chromium includes numerous separately licensed components;
the CEF license alone does not cover them. This provider adds no Widevine,
proprietary codec build options, paid components, browser extensions or plugins.
Official runtime binaries, bundled fonts/data and codec notices retain their
own conditions, including FFmpeg LGPL-2.1-or-later, WebKit LGPL/BSD and MPL
components. Pinned source/build coordinates are the exact CEF/Chromium revision
above and the official minimal archive metadata. Do not strip CREDITS or assume the MIT Rust binding license
relicenses Chromium. Static CRT linkage uses the installed Microsoft's permitted
redistributable runtime; it is not represented as open source.

The locked Rust graph additionally declares MIT, Apache-2.0, BSD-3-Clause, ISC,
Unicode-3.0, Zlib, bzip2-1.0.6, CDLA-Permissive-2.0 and optional Unlicense/0BSD
alternatives. Choose MIT wherever it is an OR alternative, Apache-2.0 otherwise;
retain all AND conditions (notably `ring`: Apache-2.0 AND ISC, and Unicode tables).
These licenses allow development/use/distribution without mandatory fees.
The enabled graph includes `download-cef`, its HTTP/TLS/archive dependencies and
CMake tooling as build dependencies. They are part of the license audit even
though they are not the browser's runtime network implementation.

`scripts/package-windows.ps1` obtains the actual locked metadata and copies all
LICENSE/LICENCE/COPYING/COPYRIGHT/NOTICE files, including nested bundled-code
licenses, beneath `notices/<crate>-<version>/`. It preserves CEF LICENSE/CREDITS
and records SHA-256 for every package file in `runtime-files.json`. Its adjacent
`dependency-metadata.json` records every pinned version, license declaration and
resolved feature list. Distribution must retain these notices. The central
repository inventory is coordinated separately and must include this lockfile.

Linux `scripts/package-linux.py` retains the equivalent notices and hashes and
filters Cargo metadata for the actual Linux target. Optional private system-library
bundling requires an explicit `MIDNIGHT_HTML_NATIVE_MANIFEST`: every input file
has a checked SHA-256 and the package preserves `native-dependencies.json`.
The qualification image needs Ubuntu NSS/NSPR (MPL-2.0, with NSS component notices)
and ALSA (LGPL-2.1-or-later). Their complete matching original source archives,
Debian patches/build scripts, copyright notices and full license texts accompany
the privately bundled shared libraries under `sources/native/` and `notices/`.
Compatible rebuilt shared libraries remain replaceable; no application restriction
on modification or reverse engineering for debugging those modifications is added.
Preserve this complete source/notices bundle when redistributing that variant.
Other Linux systems may supply these libraries themselves instead of bundling them.

No fee-only or trial-only dependency is selected. License metadata is not a
security or codec-patent guarantee; release qualification uses the actual native
archive and locked feature graph rather than unpinned project names.
