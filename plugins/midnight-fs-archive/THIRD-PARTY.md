# Archive dependency decisions

All selected dependencies permit intended development, application use and binary
distribution without mandatory subscriptions or license fees. Preserve applicable
copyright, license and notice files in distribution; this file does not assign a
license to Midnight Explorer or its public API.

| Pinned crate | Selected license | Runtime features |
| --- | --- | --- |
| zip 8.6.0 | MIT | defaults off, deflate-flate2 and aes-crypto (WinZip AES decryption) |
| flate2 1.1.10 | MIT (MIT OR Apache-2.0) | defaults off, rust_backend/miniz_oxide |
| tar 0.4.46 | MIT (MIT OR Apache-2.0) | defaults off; no xattr |
| sevenz-rust2 0.23.0 | Apache-2.0 | defaults off, aes256 (7-Zip AES decryption); no BZip2, PPMd, utilities |
| lzma-rust2 0.21.0 | Apache-2.0 | std/xz; SHA-2 trailer validation; encoder for tests only |
| bzip2 0.6.1 | MIT (MIT OR Apache-2.0) | default pure Rust libbz2-rs-sys backend; no C bzip2-sys |
| libbz2-rs-sys 0.2.6 | bzip2-1.0.6 | Rust allocator; retain original and translation copyright notices |
| ruzstd 0.9.1 | MIT | defaults off, std/hash; explicit 64 MiB decoder window bound |
| twox-hash 2.1.5 | MIT | xxhash64 only; Zstandard checksum, not copy integrity |
| aes 0.9.4, cipher 0.5.2, inout 0.2.6, block-padding 0.4.2, cbc 0.2.1 | MIT (MIT OR Apache-2.0) | RustCrypto AES block cipher, CTR (zip's own) and CBC (7z) decryption |
| hmac 0.13.0, pbkdf2 0.13.0, sha1 0.11.0 | MIT (MIT OR Apache-2.0) | RustCrypto WinZip AES key derivation (PBKDF2-HMAC-SHA1) and authentication |
| cmov 0.5.4, ctutils 0.4.8, cpubits 0.1.1 | MIT (Apache-2.0 OR MIT) | RustCrypto constant-time helpers of the crates above |
| constant_time_eq 0.4.2 | Apache-2.0 (CC0-1.0 OR MIT-0 OR Apache-2.0) | zip's constant-time comparison of the AES authentication code |
| getrandom 0.4.3 | MIT (MIT OR Apache-2.0) | required by zip/sevenz-rust2 AES features for writing salts; the read-only worker never calls it |
| zeroize 1.9.1 | MIT (Apache-2.0 OR MIT) | alloc; wipes passwords (through midnight-filesystem-api 0.5.0) |

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

The November 9 streaming change reuses these exact codec pins/features. Only the
project's public filesystem API and worker SDK versions change. Salamander source
was inspected for index/extraction/cache lifecycle behavior; no GPL source or
assets were copied into this plugin or the host.

The 21 November password support enables the `zip` crate's `aes-crypto` and
sevenz-rust2's `aes256` features. They add the RustCrypto packages listed above
(maintained by the RustCrypto organisation: block-ciphers, block-modes, MACs,
password-hashes, hashes, traits and utils repositories), constant_time_eq (Cesar
Eduardo Barros, required by zip's feature) and getrandom (rust-random, required by both
features). Each exact package's manifest and LICENSE files were inspected in Cargo's
downloaded source; every one offers MIT or Apache-2.0 (constant_time_eq: CC0-1.0, MIT-0
or Apache-2.0). `r-efi` 6.0.0 enters the lockfile through getrandom for UEFI targets
only and is not built for Windows or Linux. No paid branch or mandatory fee is selected.
