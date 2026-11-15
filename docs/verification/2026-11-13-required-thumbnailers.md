# Required thumbnailers — 13 November 2026

Scope: `codex/filter-linux-drive-list`, based on `2054ada`, with the following
build/package changes. Earlier Linux builds contained no `thumbnailers/` directory;
the host/preference implementation alone did not install either decoder worker.

## Implemented behavior

- Normal Linux native/Docker and Windows entrypoints now build and stage both
  existing thumbnailers. The new wrapper prepares the canonical public worker SDK,
  locked dependencies, approved native runtime and complete distribution notices.
  Thumbnail build helpers default to Debug and no longer silently build image-only.
- Linux automatically acquires the pinned FFmpeg 9.0.2 archive, checks its SHA-256
  and detached signature against the pinned release fingerprint, and builds the
  existing restricted LGPL recipe. Cached runs revalidate source/signature,
  configuration and artifact hashes. Source extraction rejects links/traversal and
  bounds expansion. Downloads remain in `.tools/`; generated output is in `_build/`.
- Windows requires 64-bit PowerShell 7 and the approved prebuilt MSVC FFmpeg/source/
  libclang prerequisites. Missing prerequisites fail explicitly. Its normal
  deployment requires both helpers, all four libraries and corresponding notices.
- Standard release packaging requires image and video providers. The built-in
  `midnight-video` always requires its four FFmpeg libraries and corresponding
  source/build notices, including when every library is missing. Lower-level
  generic worker-copy and private-deployment tools retain specialist fixture uses.
- Actual resolved dependency notices/features and the shared decoder's native,
  font, Rust and MPL source notices are staged. The existing missing-license-file
  WTFPL supplement is explicit and version-scoped. No dependency pins, codec
  allowlists, plugin protocols or preview preferences changed.

## Actual Debug build and installation

Root held the only Cargo/native build lease. The build container is
`midnight-explorer-thumbnail-build:2054ada`, image
`sha256:61667fe0eec4e91740a258936a3d6c289e51f08023c7d655070e751c595a19e0`.
It extends the existing pinned Rust build environment with GPG and system libclang
18.1.3; no host packages were installed. The pinned image native archives were
reused and reverified from the earlier image-viewer cache.

The actual successful command was:

```sh
bash _build/qa/required-thumbnailers-20261113/run-debug.sh \
  env MIDNIGHT_THUMBNAILERS_OFFLINE=1 \
  bash scripts/build-default-thumbnailers.sh debug
```

Pinned dependencies were fetched once beforehand with the canonical SDK config;
the real wrapper then succeeded offline. Both worker executables/manifests, four
FFmpeg libraries and notices/source are installed under `_build/debug/thumbnailers/`.
The video executable has ELF RPATH `$ORIGIN` and resolves its native libraries
beside itself. The image worker includes the static approved dav1d decoder.
The application/elevation Debug build also completed successfully. No application
or worker Release build ran; the established native decoder recipes retain their
approved optimized configurations.

The 12,040,788-byte source archive SHA-256 is
`8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`;
the detached signature validates against release fingerprint
`FCF986EA15E6E293A5644F10B4322F04D67658D8` in an isolated keyring.
GPG's ordinary unknown-web-of-trust warning does not replace or bypass the explicit
fingerprint check. The native configuration permits only the existing public
VP8/VP9/Theora/FFV1/rawvideo set, approved demuxers/parsers and Theora's internal
VP3 implementation dependency. GPL, nonfree, version3 and networking remain off.

## Verified checks

**151 targeted checks passed; five Windows-only Python cases were skipped.**

| Boundary | Passed | Skipped |
| --- | ---: | ---: |
| Default build hooks/wrappers, prerequisite setup, decoder preparation/cache, notices and Docker behavior | 70 | 5 |
| Release/viewer/filesystem/thumbnail package validation and attribution | 48 | 0 |
| Existing build bootstrap helper tests | 20 | 0 |
| Actual installed catalog plus sandboxed image/video, queued/deadline/cooperative cancellation | 5 | 0 |
| Independent image PNG/JPEG/format rejection, static SVG and AVIF, video sampled-frame/malformed fixtures | 8 | 0 |

Native checks used Cargo-receipt-selected test executables on the host, with
explicit installed helper paths and serial execution. Discovery found both actual
providers; image decoding returned exact red/green pixels; video returned distinct
frames 0/2/4. The package copier additionally validated and copied the actual
Debug thumbnail tree (726 files, both providers, runtime and notices), without
compiling a Release application. Its recorded package hash is
`d0de25162f9ac73c62fdea643fd0b44a1a5f4af696f69591417ee0633812088d`.

The final helper regression rejects duplicate/conflicting disabled-feature
definitions. The final real cached-native check passed afterward without
rebuilding FFmpeg. Review also found and fixed Windows PowerShell 5.1 invocation
and the all-libraries-missing package hole; the latter has an observed failing
regression before correction. A self-closing native Debug app startup/snapshot
and shutdown passed with in-memory settings and owned empty directories. The
snapshot was visually reviewed; existing user instances were untouched.

Receipts under `_build/qa/required-thumbnailers-20261113/` include
`thumbnail-debug-build.log`, `app-debug-build.jsonl`, `compile-tests.sh`,
`*-tests-build.jsonl`, `native-test-results.json`, `build-helper-tests.log`,
`package-tests.log`, `bootstrap-tests.log`, `cached-native-*`,
`thumbnail-package-validation.json`, `native-smoke-result.json` and `build-result.json`.
The post-build source manifest records exact source identity and is checked for
later drift; it is not presented as a pre-build snapshot. Artifact hashes and
commands are retained in `build-result.json`.

This qualifies the named Linux Debug install/discovery/decode/package/startup
boundaries. It does not qualify native Windows builds/deployment, every Linux
distribution, unsupported MP4/H.264 codecs, archive-panel desktop rendering or
full-workspace tests. The existing WGPU discrete-GPU warning remains unchanged.
