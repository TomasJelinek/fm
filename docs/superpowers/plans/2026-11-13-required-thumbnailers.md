# Required thumbnailers in standard builds

The user requested that thumbnailers become non-optional, that the current Linux
installation be built in Debug, and that the result be committed and pushed.

Normal Linux/Docker and Windows build helpers must build and stage both existing
image and video workers. A missing decoder, source archive, notice or build failure
must fail the build rather than silently producing an installation without previews.
Low-level Cargo and private deployment tools keep their explicit specialist uses.
Runtime discovery remains executable-relative and subject to existing trust checks;
preview preferences and supported codec sets do not change.

Reuse the pinned public worker SDK, image decoder and restricted LGPL FFmpeg recipe.
Linux prepares the existing pinned/signed FFmpeg source automatically; Windows
requires the already approved MSVC prefix/source/libclang prerequisites explicitly.
Do not substitute a generic FFmpeg distribution or enable extra codecs. Include
actual dependency notices and FFmpeg corresponding source beside the workers.

## Ownership and implementation

1. Build integration agent: normal entrypoint hooks, required-thumbnail wrappers,
   strict low-level image/video build behavior, Windows deployment requirement,
   and focused mocked-command tests. No compiler or Git operations.
2. Runtime agent: pinned FFmpeg acquisition/preparation and validation helper with
   focused tests. No actual native/Cargo build or Git operations.
3. Verification/notices agent: dependency notice generator and focused tests.
   Root owns all compiler/native build invocations through that helper.
4. Root: shared contracts, Linux prerequisite recipes, documentation, actual Debug
   compilation/staging, native verification, independent review and Git integration.

Root holds the sole Cargo target-directory build lease. Native desktop checks run
serially. All output belongs under `_build/`, downloads/tool caches under `.tools/`.

## Acceptance and verification

- Exercise real build wrappers with inert command fixtures: default/debug/release
  selection, both workers, required inputs, errors and deployment propagation.
- Verify pinned FFmpeg SHA/signature, restricted configuration, cached/offline paths
  and failure handling. Test notice collection and missing-notice rejection.
- Compile and stage both workers in **Debug** using the released SDK configuration.
  Verify the existing actual installed catalog test and sandboxed image/video decode
  tests, including cancellation and distinct sampled video frames.
- Run applicable independent worker fixture tests and native app startup. Record
  exact receipts and limits; Windows source integration is not Windows qualification.
- Verify formatting, source size and staged diff; commit and push the existing
  `codex/filter-linux-drive-list` branch. Never restart a user ME process.

## Review focus

Prevent silent image-only builds, stale SDK reuse, release-profile fallback,
unapproved FFmpeg configuration, missing corresponding source/notices, and success
after a helper failure. Confirm Linux runtime library lookup beside the video worker.

Implementation and scoped verification are complete. The
[verification report](../../verification/2026-11-13-required-thumbnailers.md)
records actual Debug build/install and 151 passing targeted checks with five
Windows skips; native Windows qualification remains outside this Linux run.
