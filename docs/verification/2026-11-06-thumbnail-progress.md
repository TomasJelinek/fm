# Thumbnail implementation verification ledger

Updated 7 November 2026. Source began in the isolated `midnight-thumbnails`
worktree at `5466f8068f9ce04feaddd0e0dc9d745cfb087f57` and was integrated into
main by the coordinating agent. All generated build outputs and logs named
below are relative to main `_build/media-thumbnails/`.

## Native implementation and test evidence

- Windows and Linux natively built FFmpeg 9.0.2 from the verified source below.
  The independent image and video executables use only pinned public registry
  SDKs. There is no in-process untrusted decoder fallback.
- Image provider: three fixtures cover PNG alpha/aspect ratio, JPEG EXIF
  orientation and malformed/oversized headers. Real restricted-helper BMP
  decoding also checks exact pixels and cancellation. The resize test caught
  unintended upscaling; small images now retain their original dimensions.
- Video provider: two tests cover authored real AVI samples and malformed or
  playlist rejection. Actual Windows AppContainer and Linux seccomp helper
  tests return distinct bounded sample frames through the broker.
- Scheduler: four external tests cover static video, hover priority, fair
  progression of every selected visible video, offscreen/stale rejection and
  two-worker accounting. A unit test verifies 30-second negative-result expiry.
- Cache: four tests cover persistent retention of all six samples, warm reopen
  without source reads, revision/corruption invalidation, bounded LRU, and
  recovery when another cache instance removes persistent records. Missing
  disposable records are forgotten without hiding permission/reparse failures.
- Service tests cover clearing while a decode is in flight, stale-result
  rejection and one pinned source revision across all animation samples.
- Native timeout tests cover an unresponsive helper and a broker queued behind
  the shared I/O gate. The latter explicitly observes a queued request before
  the deadline, ensuring it exercises the former join deadlock. Cancellation
  does not release a worker still inside an actual uncancellable source call.
- Native containment denies unrelated private-file reads and a live localhost
  TCP connection while broker pipes remain available. Private-store tests cover
  atomic replacement, traversal rejection, Unicode and Windows paths exceeding
  300 characters; Linux additionally rejects symlink cache redirects.
- Shared I/O tests cover weighted FIFO admission, exclusive-source fairness and
  queued cancellation. Folder-scan concurrency fixtures use isolated test gates;
  production scans, transfers and thumbnail reads retain the process-wide gate.
- UI tests added to main reject stale/offscreen/oversized pixels, animate all
  selected visible videos while ordinary video stays static, and prevent reads
  during delayed/unknown network classification until explicit opt-in. Preference
  tests preserve early per-field edits through a late settings load. Schema 12
  carries nondefault preview settings; default native/archive settings retain
  schema 10/11 compatibility.

Windows evidence: `windows-final-native.log`, `windows-final-native-clippy.log`,
`windows-review-regressions.log`, `windows-review-clippy.log`,
`windows-final-host.log`. Linux image/native evidence:
`linux-image-checks.log`; the final refreshed runs use `linux-image-final.log`
and `linux-video-final.log`. The public API suite contains three wire and two
broker tests (`final-public-api.log`). Individual logs record exact exit/results;
main UI and application release qualification belongs to the coordinating main
build, not a duplicate Slint build in this worktree.

## Private installation and independent packaging

`build-thumbnailers.ps1` produced `_build/release/thumbnailers/` with both release
executables, their manifests, four FFmpeg DLLs, original notices, source archive
and native build configuration. `deploy-private.ps1 -IncludeFilesystems
-IncludeThumbnailers` then copied viewers and both worker trees into a fresh
private disposable installation, preserving protected ACLs, no-junction checks
and SHA-256 readback. This native fixture necessarily used a trusted user-profile
ancestor; the source/build checkout's broader ancestor permissions were unchanged.

The actual installed service initially exposed a normal-Windows versus verbatim
path-prefix mismatch. Discovery now normalizes namespace syntax without resolving
reparse points before its existing trust/canonical checks; a raw UTF-16 test
preserves even unpaired units. Actual installed discovery and video decoding then
passed (`private-deploy-test.log`). A separately created uppercase `UNTRUSTED.DLL`
with an intentionally weak write ACL was rejected (`private-deploy-weak-library.log`),
after which the fixture file was removed. The worker SID receives ReadAndExecute
on exactly four installed runtime DLLs, never inherited/directory/write access.

Exported image and video projects outside the source worktree pass their 3/2
fixture tests and strict Clippy with `--locked --offline` and only the canonical
registry bundle (`exported-image.log`, `exported-video-final.log`). Their manifests
contain no host path dependencies. New worker and thumbnail APIs are separate
workspaces; published viewer API/SDK 0.1.0 artifacts are unchanged.

The final formatted bundle is `releases/worker-sdk-1794010699238644700`:

| New public package | SHA-256 |
|---|---|
| midnight-worker-sdk 0.1.0 | `4bebb3875212b35f7bb1aa8c3e85b420dfcd62effba1a4879d421598e6a51ff0` |
| midnight-thumbnail-api 0.1.0 | `06ddd129d4b50c83086680d4ea92b3eddfdab63443af8c5698f2f195b09c11e0` |
| midnight-thumbnail-sdk 0.1.0 | `54efedc97d6d31ebeaa5aabf721c20542ed9952e6f8985699d38d0b376b0c21f` |
| midnight-filesystem-api 0.1.0 | `b50788dea55a9f8f63ffccbfba569a5947765b827f6728d98171e85441ad46b7` |

Only the two new thumbnail package checksums changed during pre-release formatting;
plugin lock entries were deliberately refreshed. Worker/filesystem checksums and
all released viewer checksums stayed unchanged.

## Verified native acquisition and notices

- Official FFmpeg 9.0.2 source SHA-256:
  `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`.
  Its detached signature validates against upstream release key
  `FCF986EA15E6E293A5644F10B4322F04D67658D8` from ffmpeg.org.
- Official MSYS2 portable base 20260927 SHA-256:
  `ea2f31a0b6ade63914ce441ffb022f0f6aa96982bfefa2326460a26d5fb01322`.
  Detached signature validates against published master fingerprint
  `0EBF782C5D53F7E5FB02A66746BD761F7A49B0EC`. Build-only signed packages are
  make 4.4.1-3 and pkgconf 3.0.10-1, retained in `.tools/media-native/msys64`.
- libclang 18.1.1 Windows wheel SHA-256:
  `4dd2d3b82fab35e2bf9ca717d7b63ac990a3519c7e312f19fa8e86dcc712f7fb`.
  Linux wheel: `c533091d8a3bbf7460a00cb6c1a71da93bffe148f172c7d03b1c31fbf8aa2a0b`.
  Both match PyPI release metadata; LICENSE.TXT is Apache-2.0 with LLVM exception.
  Linux bindgen also uses installed GCC builtin headers through
  `BINDGEN_EXTRA_CLANG_ARGS=-isystem /usr/lib/gcc/x86_64-linux-gnu/13/include`.

FFmpeg source LICENSE.md confirms LGPL-2.1-or-later with optional GPL/nonfree/
version3 disabled. The recipe also disables networking, protocols, hardware and
external autodetection; it enables VP8/VP9/Theora/FFV1/rawvideo and Matroska/WebM,
AVI/Ogg only. Initial video support excludes H.264/H.265, MPEG-4 and AV1. HDR/wide
color gamuts and unsupported display transforms fail closed. Archive members are
currently excluded from thumbnail targets pending a typed source adapter.

Pinned Rust decoder dependencies include image 0.25.10 (PNG/JPEG/GIF/WebP/BMP/
ICO/TIFF; default features disabled), moxcms 0.8.1, ffmpeg-next 9.0.0,
ffmpeg-sys-next 9.0.1 and BLAKE3 1.8.7. Full native-target graphs and license
inventory are in `docs/THIRD_PARTY.md`. Generated image/video notices retain
35/28 third-party dependency records plus Rust notices. ffmpeg-sys-next's exact
published commit declares WTFPL but has no standalone license file; its override
retains that declaration, the canonical official license text and provenance.
All generated notices stay under `_build/` and must be supplied to staging via
NoticesRoot. Corresponding FFmpeg source and configuration accompany its shared
libraries so recipients can exercise the applicable LGPL rights.

See [worker SDK contract](../developer/WORKER_SDK.md) and the
[thumbnail design](../superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).