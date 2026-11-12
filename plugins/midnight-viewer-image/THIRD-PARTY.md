# Third-party material

Both independent image workers use the shared image-decoder library. PNG, JPEG,
GIF, WebP, BMP, ICO and TIFF retain image-rs 0.25.10 and moxcms 0.8.1. Static SVG
adds resvg/usvg 0.48.1 with only the text feature: no system fonts, font memory
mapping, SVGZ or raster-image loader is enabled. Explicit image resolvers also
refuse external/embedded resources. These dependencies do not reference host code.

MIT is selected for image/resvg/usvg; tiny-skia retains BSD-3-Clause notices.
Unchanged embedded Noto Sans fonts retain OFL-1.1, including copyright, license,
source provenance and hashes under notices/Noto-Sans. They are bundled with the
worker, never sold on their own or relicensed. The exact pinned license files were
inspected; no mandatory fee, subscription or royalty is introduced by this SVG
batch. AVIF adds dav1d 0.11.0/dav1d-sys 0.8.3 (MIT), static native
dav1d 1.5.4 (BSD-2-Clause), and mp4parse 0.17.0 (MPL-2.0). The exact
mp4parse source archive is shipped beside its license. The minimal image-rs
constructor patch is recorded in notices/image-0.25.10/MIDNIGHT_PATCH.md.
Only the native decode feature is enabled, not an AVIF encoder. HEIC is not enabled.
API/SDK and application licenses are not changed by the plugin's MIT license.

Original copyright/license files for the pinned cross-platform graph are retained
under notices/. This table includes normal and test dependencies; it is not a
claim that all test libraries are linked into a release executable.

| Crate | Version | Declared license |
| --- | --- | --- |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 |
| ahash | 0.8.12 | MIT OR Apache-2.0 |
| arrayref | 0.3.9 | BSD-2-Clause |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |
| av-data | 0.4.4 | MIT |
| base64 | 0.23.1 | MIT OR Apache-2.0 |
| bitflags | 2.13.2 | MIT OR Apache-2.0 |
| bitreader | 0.3.11 | MIT OR Apache-2.0 |
| byte-slice-cast | 1.2.3 | MIT |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT |
| byteorder | 1.5.0 | Unlicense OR MIT |
| byteorder-lite | 0.1.0 | Unlicense OR MIT |
| bytes | 1.12.1 | MIT |
| cfg-expr | 0.20.10 | MIT OR Apache-2.0 |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 |
| color_quant | 1.1.0 | MIT |
| crc32fast | 1.5.4 | MIT OR Apache-2.0 |
| data-url | 0.3.2 | MIT OR Apache-2.0 |
| dav1d | 0.11.0 | MIT |
| dav1d-sys | 0.8.3 | MIT |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| euclid | 0.22.14 | MIT OR Apache-2.0 |
| fallible_collections | 0.4.9 | MIT/Apache-2.0 |
| fax | 0.2.7 | MIT |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 |
| flate2 | 1.1.11 | MIT OR Apache-2.0 |
| float-cmp | 0.9.0 | MIT |
| font-types | 0.12.7 | MIT OR Apache-2.0 |
| fontdb | 0.24.0 | MIT |
| gif | 0.14.2 | MIT OR Apache-2.0 |
| half | 2.7.1 | MIT OR Apache-2.0 |
| harfrust | 0.12.0 | MIT |
| hashbrown | 0.13.2 | MIT OR Apache-2.0 |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 |
| heck | 0.5.0 | MIT OR Apache-2.0 |
| image | 0.25.10 | MIT OR Apache-2.0 |
| image-webp | 0.2.4 | MIT OR Apache-2.0 |
| imagesize | 0.15.0 | MIT |
| indexmap | 2.14.2 | Apache-2.0 OR MIT |
| kurbo | 0.13.1 | Apache-2.0 OR MIT |
| libc | 0.2.191 | MIT OR Apache-2.0 |
| log | 0.4.35 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 |
| moxcms | 0.8.1 | BSD-3-Clause OR Apache-2.0 |
| mp4parse | 0.17.0 | MPL-2.0 |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 |
| num-derive | 0.4.2 | MIT OR Apache-2.0 |
| num-integer | 0.1.47 | MIT OR Apache-2.0 |
| num-rational | 0.4.2 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| once_cell | 1.21.4 | MIT OR Apache-2.0 |
| pico-args | 0.5.0 | MIT |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 |
| png | 0.18.1 | MIT OR Apache-2.0 |
| polycool | 0.4.0 | MIT OR Apache-2.0 |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 |
| quick-error | 2.0.1 | MIT/Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| read-fonts | 0.41.0 | MIT OR Apache-2.0 |
| resvg | 0.48.1 | Apache-2.0 OR MIT |
| rgb | 0.8.53 | MIT |
| roxmltree | 0.21.2 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_spanned | 1.1.3 | MIT OR Apache-2.0 |
| simd-adler32 | 0.3.10 | MIT |
| simplecss | 0.2.2 | Apache-2.0 OR MIT |
| siphasher | 1.0.4 | MIT OR Apache-2.0 |
| skrifa | 0.44.0 | MIT OR Apache-2.0 |
| slotmap | 1.1.1 | Zlib |
| smallvec | 1.16.5 | MIT OR Apache-2.0 |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 |
| strict-num | 0.1.1 | MIT |
| svgtypes | 0.16.1 | Apache-2.0 OR MIT |
| syn | 2.0.119 | MIT OR Apache-2.0 |
| syn | 3.0.9 | MIT OR Apache-2.0 |
| system-deps | 7.0.8 | MIT OR Apache-2.0 |
| target-lexicon | 0.13.5 | Apache-2.0 WITH LLVM-exception |
| tiff | 0.11.3 | MIT |
| tiny-skia | 0.12.0 | BSD-3-Clause |
| tiny-skia-path | 0.12.0 | BSD-3-Clause |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT |
| toml | 1.2.1+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_datetime | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 |
| toml_writer | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 |
| unicode-bidi | 0.3.18 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| unicode-script | 0.5.9 | MIT OR Apache-2.0 |
| unicode-vo | 0.1.0 | MIT/Apache-2.0 |
| usvg | 0.48.1 | Apache-2.0 OR MIT |
| version-compare | 0.2.1 | MIT |
| version_check | 0.9.5 | MIT/Apache-2.0 |
| weezl | 0.1.12 | MIT OR Apache-2.0 |
| windows-link | 0.2.1 | MIT OR Apache-2.0 |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 |
| winnow | 1.0.5 | MIT |
| xmlwriter | 0.1.0 | MIT |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT |
| zlib-rs | 0.6.8 | Zlib |
| zune-core | 0.5.3 | MIT OR Apache-2.0 OR Zlib |
| zune-jpeg | 0.5.15 | MIT OR Apache-2.0 OR Zlib |
