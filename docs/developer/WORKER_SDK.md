# Restricted worker SDK

`midnight-worker-sdk` 0.1.0 is the public startup contract for independent
executable providers. `midnight-worker-platform` is the host-owned launcher;
plugins never depend on it or application internals. Released viewer API/SDK
0.1.0 package bytes and checksums are unchanged.

## Startup and containment

The host launches an exact trusted executable with stdin/stdout pipes and
discarded stderr. It sends `MWS1`, a little-endian u64 memory ceiling and u32
process CPU ceiling. Accepted limits are 64 MiB through 1 GiB and 1 through
60 seconds. Before reading content, the helper calls
`midnight_worker_sdk::enter(input, output)`. It returns `MWR1` only after
installing or checking every platform restriction. The provider-specific
versioned protocol then begins. Host `read_ready` rejects other replies.

This is a contract with trusted installed code, not attestation of arbitrary
downloaded programs. Discovery checks installation ownership, write permissions
and redirect-free paths. Unsupported containment fails closed. No untrusted
decoder runs inline in the application.

Host `spawn(executable, Limits { memory_bytes, cpu_seconds })` returns a process
guard and pipes. Each service enforces its wall deadline, cancellation and wire
bounds separately. Guards kill and reap unfinished children. CPU limits apply
to a process lifetime; bounded scheduling controls concurrent processes.

Windows uses the fixed `MidnightExplorer.RestrictedWorker.v1` AppContainer,
without capabilities, and an explicit inherited-handle list. A suspended process
is assigned to a Job Object with memory, CPU, single-process and kill-on-close
limits before resuming. Only required system loader environment survives.
The SDK verifies AppContainer and job restrictions before ready.

The optional FFmpeg provider requires ReadAndExecute access for that fixed
worker SID on exactly `avcodec-63.dll`, `avformat-63.dll`, `avutil-61.dll` and
`swscale-10.dll`. Deployment adds only those explicit file ACEs. It grants no
directory-wide, inherited or write access and no networking capability. Code
and all ancestors still pass installation trust checks. Private deployment
does not change checkout or ancestor permissions.

Linux x86-64 and AArch64 startup closes descriptors above stderr, sets
address-space/CPU/file-size/core/descriptor limits, enables `no_new_privs`, and
installs a synchronized seccomp allowlist. Computation, allocation and existing
pipe operations remain available. Filesystem opens, networking, new processes,
threads and execution are denied. Libraries must load before SDK lockdown;
`$ORIGIN` runtime lookup locates packaged FFmpeg libraries without relying on
`LD_LIBRARY_PATH`. Unsupported platforms fail closed.

Native containment fixtures attempt an unrelated private file read and a live
localhost TCP connection after ready. A deliberately unresponsive helper tests
deadline termination. Cross-compilation alone does not qualify a platform.

## Thumbnail protocol and backend

The separate `midnight-thumbnail-api` and `midnight-thumbnail-sdk` 0.1.0 crates
define `MTH1` framed messages and a broker `Read + Seek` adapter. Helpers receive
request ID, source byte length, requested dimensions and sample index, never a
pathname, credentials or URL. Offset/length reads use a backend source lease
whose revision is checked. Native sources use `CapturedFile`; the host's
generic content adapter can support future virtual providers.

Reads are at most 256 KiB and must return the exact checked length. RGBA output
is bounded to 512 by 512 (1 MiB), with six sample indices. Host limits also cover
total traffic, read count, deadline, generation and dimensions. The service asks
for 96 by 96 and admits two decoders. The shared I/O gate admits one unknown,
HDD or network source or a bounded batch of confirmed SSD reads, releasing
permits before decoding, hashing and waiting.

Videos use one static sample normally. Hover or selection admits five more;
hover has priority while every visible selected video progresses in rotation.
Offscreen jobs cancel. Decode failures retry after 30 seconds. Every accepted
sample must persist before presentation. Private atomic cache records include
opaque identity, revision, decoder version and dimensions. Clear-cache waits
for cancelled workers, removes old records and starts a new visible generation
so late results cannot resurrect cleared entries.

## Independent builds and deployment

The `worker-packages` binary in `api/tools/package` produces a separate registry
bundle containing worker, thumbnail, image-viewer and optional filesystem APIs. Its
`.cargo/registry.toml` declares the registry without redirecting third-party
dependencies. Independent plugins pin registry versions and have no host path
dependencies. Existing viewer registry artifacts remain unchanged.

`scripts/build-thumbnailers.ps1` and `.sh` use that config with `--locked --offline`.
Supply `-NoticesRoot` on Windows or the fourth positional argument on Linux,
pointing to generated `_build/` notices with children `midnight-thumbnail-image/`
and `midnight-thumbnail-video/`. Each contains pinned dependency declarations and
original license/copyright files. Missing notices fail staging. The inventory
table alone does not generate these files. Optional video inputs are the verified FFmpeg prefix, matching
source archive and libclang directory. A flat executable-adjacent
`thumbnailers/` directory holds executable/JSON pairs, exact runtime libraries
and `notices/`; it is the only discovery location. Filesystem workers use their
separate `filesystems/` layout.

`scripts/deploy-private.ps1 -IncludeFilesystems -IncludeThumbnailers` copies
requested worker trees alongside viewers into a fresh protected Windows
installation with no-junction checks and SHA-256 readback. Video distribution
includes matching LGPL source, licenses and build configuration. Users remain
able to replace those shared libraries under their license terms.

Initial video support is VP8, VP9, Theora, FFV1 and raw video in Matroska/WebM,
AVI and Ogg. The verified recipe disables GPL, nonfree, networking, protocols,
external autodetection and unrelated components. Support for every file with
these extensions is not implied.

See [filesystem API](FILESYSTEM_API.md), [thumbnail design](../superpowers/specs/2026-11-05-panel-views-thumbnails-design.md)
and [dependency inventory](../THIRD_PARTY.md). The verification ledger records
actual native qualification separately from this contract.
