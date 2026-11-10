# JPEG Encoded Source Streaming Implementation Plan

**Status: reverted at the user's request on 9 November 2026.** The adapter,
direct zune dependencies and JPEG-specific tests were removed. It was never
deployed to the application's private runtime. The implementation and test
evidence below describe the discarded experiment, not the current source.
Do not execute this plan again unless the user explicitly resumes it. Shared
source/cache/spool work for all viewers is also deferred.

> Historical implementation plan follows; its checked items are not active features.

**Goal:** Remove the image worker's complete encoded JPEG buffer while retaining
the current decoded image, metadata, color and sandbox contracts.

**Architecture:** A plugin-local ImageDecoder adapter uses the already pinned
zune-jpeg decoder directly over the buffered SDK source. Existing shared image
postprocessing and viewport rendering remain the consumer of its pixels.

**Tech Stack:** Rust, image 0.25.10, zune-jpeg 0.5.15, zune-core 0.5.3, moxcms 0.8.1.

**Spec:** `docs/superpowers/specs/2026-11-09-archive-streaming-design.md`, JPEG
subsection. Parent reviewed this independent implementation scope on 9 November.

## Global Constraints

- Plugin-only implementation; no public API, host/native ABI or archive changes.
- Keep 256 KiB source buffering, 32768 pixel edge and 128 MiB decoded-data limits.
- Keep permissive JPEG behavior, output colorspace selection, EXIF orientation,
  ICC handling and all common sRGB/RGBA rendering behavior.
- Direct exact zune pins use existing default x86/neon/std features and the MIT
  license branch after pinned license inspection. Update inventories/notices.
- Build/test output uses `_build/archive-jpeg-streaming`; no root lock changes.
- This removes encoded JPEG preloading, not full decoded pixels, solid archive
  decompression, archive cache memory, or unsupported PSD limitations.

## Review Focus

- A large source suffix must not cause allocation or reads proportional to EOF.
- Short broker reads, backward seeks and real I/O errors must preserve correctness.
- Metadata/header reads must not decode entropy or preload the entire image.
- Progressive/grayscale/color/EXIF/ICC output must match the pinned old route.
- Excessive dimensions/profile size and malformed inputs must fail before pixels
  are exposed; cancellation-related I/O failure must not become successful pixels.

---

### Task 1: Streaming adapter and meaningful regression

**Files:** Modify plugin `Cargo.toml`, `Cargo.lock`, `src/lib.rs`,
`tests/read_budget.rs`; create `src/jpeg.rs` and focused JPEG tests.

**Interfaces:** `jpeg::Decoder::new<R: Read + Seek>(reader: R)` returns an
`ImageDecoder` owning its bounded buffer and parsed zune decoder. Existing
`decode(&mut dyn ReadSeek)` and retained image result remain unchanged.

- [x] Add measured valid JPEG + 4 MiB suffix test through public decode; verify
  old code fails by reading all bytes.
- [x] Implement JPEG selection after format sniffing, streaming header parse,
  color mapping, metadata and decode_into; retain common postprocessing.
- [x] Verify suffix regression, header-only read bounds, short reads/seeks, actual
  I/O failures and pinned-reference pixel/metadata equivalence on generated JPEGs.
- [x] Verify existing independent plugin suite and strict Clippy, then formatting.

### Task 2: Packaging and claims

**Files:** plugin README.md/THIRD-PARTY.md; parent owns global inventory and
IMAGE_VIEWER_API.md.

- [x] Inspect exact MIT license files; retain default enabled codec feature set.
- [x] Update plugin explanation and license choice; provide registry/config paths
  to parent for inventory regeneration and package validation.
- [x] Record test evidence and remaining archive/full-pixel memory limitations.

## Verification evidence

Windows and native Linux each pass 23 independent plugin tests, strict Clippy
(`-D warnings`) and formatting. Windows release and Linux debug packages were
built from the frozen plugin sources. Local logs and source hashes are under
`_build/archive-jpeg-streaming/`; the registry is the immutable public SDK bundle
`_build/image-viewer/releases/worker-sdk-1794092647751272500`.

The first suffix regression failed against the original code: a 654-byte JPEG
with a 4 MiB unused suffix caused 4,457,102 source bytes to be read. The new test
allows at most the JPEG length plus two 256 KiB buffers (sniff and decode).
Header/ICC/orientation inspection reads at most one 256 KiB buffer and reuses
metadata without another read.

Review found and tests reproduced two stream-specific mistakes before delivery:
transient Interrupted poisoning and zune metadata integer reads across legal
short reads/buffer boundaries. The adapter retries Interrupted beneath its buffer,
coalesces requested bytes without allocation and preserves sticky real I/O
failures. Both the one-byte-reader and APP1-length-at-262143 regressions pass.
Owned fixtures exercise varied component DC values for CMYK/YCCK and color
progressive JPEG with 2x2 subsampling, plus grayscale, all eight EXIF orientations,
ICC conversion, corrupt headers, late seek failure and decoded-resource limits.

The first Linux Clippy invocation used a global config before the external
subcommand and could not resolve the SDK; placing `--config` after `clippy`
resolved it. Final checks passed without product-source changes. No public API,
archive cache, PSD support or decoded-full-image storage was changed.
