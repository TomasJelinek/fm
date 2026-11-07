# Archive dependency decisions

All selected dependencies permit intended development, application use and binary
distribution without mandatory subscriptions or license fees. Preserve applicable
copyright, license and notice files in distribution; this file does not assign a
license to Midnight Explorer or its public API.

| Pinned crate | Selected license | Runtime features |
| --- | --- | --- |
| zip 8.6.0 | MIT | defaults off, deflate-flate2 only |
| flate2 1.1.10 | MIT (MIT OR Apache-2.0) | defaults off, rust_backend/miniz_oxide |
| tar 0.4.46 | MIT (MIT OR Apache-2.0) | defaults off; no xattr |
| sevenz-rust2 0.23.0 | Apache-2.0 | defaults off; no AES, BZip2, PPMd, utilities |
| lzma-rust2 0.21.0 | Apache-2.0 | std/xz; SHA-2 trailer validation; encoder for tests only |
| bzip2 0.6.1 | MIT (MIT OR Apache-2.0) | default pure Rust libbz2-rs-sys backend; no C bzip2-sys |
| libbz2-rs-sys 0.2.6 | bzip2-1.0.6 | Rust allocator; retain original and translation copyright notices |
| ruzstd 0.9.1 | MIT | defaults off, std/hash; explicit 64 MiB decoder window bound |
| twox-hash 2.1.5 | MIT | xxhash64 only; Zstandard checksum, not copy integrity |

The 7z encoder feature is a development-only dependency for real solid fixtures.
ZIP writing is used only by tests, although its crate includes writer code.
No external 7-Zip executable, proprietary codec, nonfree FFmpeg component or RAR
implementation is used. The exact pinned upstream LICENSE files were inspected
from Cargo's downloaded source. Full transitive inventory is generated from this
project's Cargo.lock by the repository dependency inventory tooling before release.

Primary upstream sources: [zip](https://github.com/zip-rs/zip),
[flate2](https://github.com/rust-lang/flate2-rs),
[tar](https://github.com/composefs/tar-rs),
[sevenz-rust2](https://github.com/hasenbanck/sevenz-rust2),
[lzma-rust2](https://github.com/hasenbanck/lzma-rust2).

Additional primary sources: [bzip2](https://github.com/trifectatechfoundation/bzip2-rs),
[libbzip2-rs](https://github.com/trifectatechfoundation/libbzip2-rs),
[ruzstd](https://github.com/KillingSpark/zstd-rs).
Exact pinned package LICENSE files were reviewed on 7 November 2026. The
bzip2-1.0.6 conditions require preserving notices, marking altered source versions,
not misrepresenting origin, and not using authors' names for endorsement.
No mandatory fees, royalties, subscription or paid license branch is selected.
