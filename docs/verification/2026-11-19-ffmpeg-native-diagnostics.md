# FFmpeg native diagnostics — 19 November 2026

The retained FFmpeg 9.0.2 MSVC profile remains **unqualified**. Its successful
build and compressed VP9/Opus SDK scenario do not resolve its build diagnostics.
The exact 54 recorded entries are one configure threading warning and 53
warnings from compiled library objects, not 54 demonstrated runtime defects.
Separately, build stderr records 15 missing-tool failures. This is a read-only
source/log diagnosis: no new patches, compiler reproductions, native rebuilds or
runtime tests were performed for this review.

## Evidence and compiler identity

Evidence root:
`D:/experiments/dev/FM/_build/qa/media-playback-win-v1/`.
Paths in this record are relative to that root unless stated otherwise.

| Evidence | Scope |
| --- | --- |
| `native-msvc-v4/native-result.json` | Exit 0; 54 `compilerDiagnostics` entries; `qualified=false`; hashes of the five built shared DLLs. SHA-256: `5984a80337e3a9784a5abf86fc70a63124660c50de85f6e1ed9c9c53ae4b22a5`. |
| `native-msvc-v4/native-build.stdout.log` | Configure summary and actual `CC` object compilation, including the 53 MSVC warnings. |
| `native-msvc-v4/native-build.stderr.log` | Fifteen identical missing-`cmp` messages, outside the 54-entry array. |
| `objects/ffbuild/config.mak` | Compiler `19.44.35228 for x64` at line 20; `/std:c17`, `/O2`, `/W3` and the existing FFmpeg warning options at line 54. |
| `objects/config.h` | `ARCH_X86_64=1`; `HAVE_THREADS=0`, `HAVE_PTHREADS=0`, `HAVE_W32THREADS=0`; `CONFIG_SMALL=0`. The summary's `ARCH x86 (generic)` does not mean a 32-bit build. |
| `objects/ffbuild/config.log` | Capability probes and configure command. Expected failed probes must be distinguished from actual object-build diagnostics. |
| `configure-args.txt`, `inputs.json`, `native.sh` | Profile flags, source identity and retained build recipe. Both pthreads and W32 threads are explicitly disabled. |

FFmpeg source paths below are relative to
`source/ffmpeg-9.0.2/ffmpeg-9.0.2/`. Archive SHA-256 recorded by `inputs.json`:
`8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`.
The reviewed `avcodec.c`, `vp9dsp_template.c`, `log.c`, `swscale.c` and `ops.c`
were independently compared byte-for-byte with the retained pinned archive and
matched. This was not a new extraction or a complete source-tree hash audit.

## Exact warning classification

Counts include repeated template instantiations; they are diagnostic
occurrences, not counts of distinct defects. All 53 MSVC occurrences below
appear during library object compilation in the retained stdout log.

| Count | Class and FFmpeg source locations | Finding and proposed treatment |
| ---: | --- | --- |
| 1 | Configure threading warning; stdout line 85 | Native global mutex/once support is disabled. This is a latent concurrency limitation, not a demonstrated race in the current serialized worker. See the bounded-thread distinction below. |
| 3 | C4293; `libavcodec/vp9dsp_template.c:1750`, instantiated for 8/10/12-bit VP9 | `bits=0` produces a negative shift inside a constant-false conditional branch of the transform macro at lines 1156–1192. That expression is not evaluated. No runtime UB follows from these three occurrences. An algebraically equivalent rounding constant, `(1U << bits) >> 1`, would avoid the invalid dead expression while preserving evaluated cases. Verify all three bit depths. |
| 8 | C4113; `libavcodec/vp9dsp_template.c:1146–1149`, four each for 8/10-bit | The printed callback types differ only by the destination pointer's top-level `restrict`; widths and calling convention agree. This is a qualifier/conformance issue, not demonstrated ABI corruption. Reduce it with the pinned compiler, then use a compiler correction or consistent callback declarations/definitions; a narrow wrapper can preserve the restricted implementation. Do not cast incompatible function pointers to hide it. |
| 9 | C4334; `libavformat/matroskadec.c:945,1304`; `libavcodec/vlc.c:75,340,555`; `libswscale/format.c:1217,1218`; `libswscale/swscale.c:585,588` | A 32-bit shift is subsequently widened. Examined EBML/VLC and pixel/range-conversion paths constrain counts and values; no incorrect result was established. Make operands width-correct where 64-bit arithmetic is intended, preserving and checking those bounds rather than changing result widths blindly. |
| 1 | C4334; `libswscale/ops.c:239` | The rational multiplier uses signed `1 << op->shift.amount`. An amount of 31 is undefined for the 32-bit signed operand; larger counts are invalid. `ops_optimizer.c:448` merges shift amounts and line 725 derives them from powers of two. No current media input or runtime reproducer reaching the problematic amount has been established. Prove the producer bound, or use a 64-bit positive factor with a validated representable bound; merely widening the final result is insufficient. |
| 3 | C4333; `libavutil/log.c:201,207,208` | On this Windows configuration, `color` is an 8-bit console palette (`:65`). Shifts by 8/16 after integer promotion are defined but always zero. Forced ANSI-256 fallback can therefore have incorrect colors; these are not media-pixel/sample truncation warnings. Use separate appropriate ANSI and native-console representations rather than a cast that only hides the zero extraction. |
| 17 | C5286×6 and C5287×11; C5286: `libswscale/ops.c:317,423,455,486,493,509`; C5287: `libswscale/ops.c:327,379,401,415,437,447,462,514,527` and `libavcodec/avcodec.c:754,755` | Fifteen occurrences mix anonymous mask constants with `SwsCompFlags`; use consistently typed masks while preserving the complement mask's bits. The other two occur only in compile-time assertions, whose source already casts both enums to `int`. A reduced compiler reproducer is needed before selecting an equivalent assertion/compiler remedy; adding the same casts again is not a fix. No runtime enum conversion defect was established. |
| 2 | C4090; `libswscale/swscale.c:329,1091` | `memcpy` copies pointer arrays into mutable local arrays; it does not modify const pixel storage. Element-by-element pointer assignments avoid the nested-qualification complaint without removing pixel constness. |
| 10 | C4101; `libavutil/ripemd.c:139,196,321,393`; `libswscale/yuv2rgb.c:457,489` | Four unused RIPEMD temporaries and six monochrome-template locals; declarations already use `av_unused`. Scope declarations to their actual conditional/template uses. No runtime defect is established. |

Microsoft's [C4333 documentation](https://learn.microsoft.com/en-us/cpp/error-messages/compiler-warnings/compiler-warning-level-1-c4333?view=msvc-170)
explicitly illustrates an unsigned byte shifted right by eight yielding zero.
Its [C4113 documentation](https://learn.microsoft.com/en-us/cpp/error-messages/compiler-warnings/compiler-warning-level-1-c4113?view=msvc-170)
states that the assignment is compiled without modification. Neither diagnostic
alone establishes corrupted media or an exploitable input.

## Global thread support and bounded worker execution

The profile explicitly uses `--disable-pthreads --disable-w32threads`.
`libavutil/thread.h:186–211` consequently supplies no-op mutex functions and a
plain-byte once-initialization check. Concurrent access to shared library state
would lack those guards. The present one-decoder lane per worker process does
not demonstrate such concurrent library access; no race is claimed here.

The active host checkout's
`plugins/midnight-thumbnail-video/src/native.rs:223–224` deliberately sets
`thread_count=1` and `thread_type=0`. Keep both settings. The pinned scaler's
`libswscale/options.c:100` defaults its separate `threads` option to one;
automatic thread selection is explicit zero at line 101. Enabling native W32
mutex/once support is not permission to enable automatic decode/scaler pools.

The minimal proposed profile change is to enable W32 threads while retaining
pthreads disabled, then verify `HAVE_THREADS=1` and `HAVE_W32THREADS=1` and retain
codec/scaler limits of one in qualification. The profile already uses bundled
`compat/atomics/win32` via `config.mak:53`. Failed native `<stdatomic.h>` probes
in `config.log` select that fallback; they are not evidence that experimental
MSVC atomics, an extra library or a different codec-thread policy is required.

## Missing tool and qualification decision

All 15 stderr failures are `configure:1942: cmp: command not found`.
`cp_if_changed()` falls through to `cp` when its comparison fails, so this does
not demonstrate corrupted generated configuration. It does show an incomplete
shell-tool admission check. Restore `cmp` in the admitted build toolchain and
make preflight verify its presence. Do not count these failures as capability
probe outcomes or fold them silently into the 54 compiler-diagnostic entries.

The existing zero-diagnostic qualification gate still fails. The profile's
thread-safety limitation, missing tool and unresolved shift-domain analysis
must be addressed explicitly; the remaining warnings need proper source or
compiler resolutions for a clean build, not blanket suppressions. The passing
VP9/Opus SDK decode/seek/EOF case remains useful for its recorded DLL hashes but
does not cover every template, pixel format, malformed input or concurrency
boundary described here.

A separate, controlled native rebuild should:

1. Preserve the original receipts and DLLs. Prepare isolated source/objects and
   record compiler, tool admission, original archive and profile identities.
2. Resolve the tooling/global-lock configuration and focused compiler/shift
   reproductions before choosing minimal patches. Retain codec/scaler bounds;
   do not add warning-disabling options or change the component/license policy.
3. Record every patch, preimage/postimage hash and configure change in build
   provenance and the LGPL source/notice bundle. Keep GPL, nonfree, version3 and
   network disabled and verify the same permitted component closure. Update
   affected dependency/distribution records for the modified source/profile.
4. Build from fresh objects and inspect both compiler and shell-tool diagnostics.
   Replay VP9 8/10/12-bit transform cases, relevant VLC/EBML and scaler arithmetic
   boundaries, then compressed SDK decode/seek/EOF/retirement and resource limits.

These are proposed next actions, not applied changes or passing tests. No
production script, FFmpeg source, profile, license choice or runtime package was
changed by this diagnosis; native qualification remains open.
