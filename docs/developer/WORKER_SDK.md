# Restricted worker SDK

`midnight-worker-sdk` 0.2.0 is the public startup contract for independent
executable providers. `midnight-worker-platform` is the host-owned launcher;
plugins never depend on it or application internals. Released viewer API/SDK
0.1.0 package bytes and checksums are unchanged.

## Startup and containment

The host launches an exact trusted executable with stdin/stdout pipes and
discarded stderr. Version 0.2.0 supports both startup forms:

| Host entry point | Wire and resource policy | Required ready acknowledgement |
| --- | --- | --- |
| `spawn(executable, Limits { memory_bytes, cpu_seconds })` | Legacy `MWS1`, little-endian u64 memory ceiling and u32 aggregate CPU seconds; memory 64 MiB–1 GiB, CPU 1–60 seconds | `MWR1`, checked by `read_ready` |
| `spawn_with_policy(executable, ResourcePolicy { memory_bytes, cpu })` | Explicit `MWS2`, little-endian u64 memory ceiling and a CPU tag: 0 for `NoAdditionalLimit`, 1 followed by u32 seconds for `Seconds(1..=60)`; same memory bounds | `MWR2`, checked by `read_policy_ready` |

Before reading content, the helper calls
`midnight_worker_sdk::enter(input, output)`. It consumes the selected startup
version and acknowledges that version only after installing/checking restrictions.
The provider-specific protocol then begins. Legacy bytes remain unchanged;
an explicit-policy host cannot accept the legacy acknowledgement in its place.
`NoAdditionalLimit` adds no aggregate CPU ceiling; it does not raise inherited OS
limits or disable memory/process isolation, host deadlines or cancellation. See
the [startup implementation](../../worker-api/midnight-worker-sdk/src/startup.rs)
and [policy types](../../worker-api/midnight-worker-sdk/src/policy.rs).

This is a contract with trusted installed code, not attestation of arbitrary
downloaded programs. Discovery checks installation ownership, write permissions
and redirect-free paths. Unsupported containment fails closed. No untrusted
decoder runs inline in the application.

Both launch forms return a process guard and pipes. Each service enforces its
wall deadline, cancellation and wire
bounds separately. Guards kill and reap unfinished children. CPU limits apply
to a process lifetime; bounded scheduling controls concurrent processes.
`Process::wait` returns `Ok(false)` for a nonzero exit status on every platform
(including a killed worker); `Process::wait_status` returns the `ExitStatus` itself:
the full 32-bit exit code on Windows (1 after `kill`), the exit code or terminating
signal on Linux (SIGKILL after `kill`). `wait_terminated` confirms reaping
regardless of status.

Windows uses the fixed `MidnightExplorer.RestrictedWorker.v1` AppContainer,
without capabilities, and an explicit inherited-handle list. A suspended process
is assigned to a Job Object with memory, the selected aggregate CPU policy,
single-process and kill-on-close
limits before resuming. Only required system loader environment survives.
The SDK verifies AppContainer and job restrictions before ready.
Pipe and `NUL` handles are created non-inheritable. The three child handles become
inheritable only for the worker's `CreateProcessW` call, under a process-wide
creation lock (`midnight_worker_platform::lock_process_creation`), and are closed
before it is released on every exit path; the browser broker launch follows the same
rule. An in-process launcher that inherits every inheritable handle, such as
`std::process::Command`, can capture a worker's pipe ends only if it runs during
that single call. `midnight_worker_platform::spawn_command` spawns such a command
under the lock, which it holds for the creation call only, never while the child
runs or while its output or exit status is awaited. The application starts external
editors, Custom Command programs and the software-rendering restart through it, so
worker and browser creation waits while such a program is being created: about 4 ms
per launch (median 3.5-4.7 ms, 90th percentile 4-6 ms) and about 20-32 ms for a
program's first start in the process, measured on Windows 11 with Microsoft
Defender, and longer for a program on a network share or one that antivirus software
scans. This short wait is accepted instead of starting these programs with a
separate `CreateProcessW` call and an explicit handle list. Tests in
`midnight-platform-windows` observe the editor and
Custom Command launches waiting for the lock; no test observes this for the restart.
Terminal launches call `CreateProcessW` without handle inheritance, and Shell
launches (`ShellExecuteEx`) pass no handles. Third-party code that starts processes
inside the application cannot take the lock and is outside this rule: Shell
extensions run by a context-menu command or a `DelegateExecute` verb. The About
window shows Slint's `AboutSlint` attribution but covers it, so a click does not open
its link, which Slint's winit backend would start with the `webbrowser` crate's
`std::process::Command::spawn` on the UI thread.

The installed FFmpeg workers share ReadAndExecute access for that fixed worker
SID on `avcodec-63.dll`, `avformat-63.dll`, `avutil-61.dll`, `swscale-10.dll`
and `swresample-7.dll`. Deployment includes a file-specific grant for
`midnight-video-playback.exe`. Package file and ancestor trust checks still
apply, and media bytes enter the restricted apps through the captured read broker. Private deployment
does not change checkout or ancestor permissions.

Linux x86-64 and AArch64 startup closes descriptors above stderr, sets
address-space/file-size/core/descriptor limits and the selected additional CPU
limit, enables `no_new_privs`, and
installs a synchronized seccomp allowlist. Computation, allocation and existing
pipe operations remain available. Filesystem opens, networking, new processes,
threads and execution are denied. Libraries must load before SDK lockdown;
`$ORIGIN` runtime lookup locates packaged FFmpeg libraries without relying on
`LD_LIBRARY_PATH`. Unsupported platforms fail closed.

Native containment fixtures attempt an unrelated private file read and a live
localhost TCP connection after ready. A deliberately unresponsive helper tests
deadline termination. Cross-compilation alone does not qualify a platform.

## Thumbnail protocol and backend

The separate `midnight-thumbnail-api` 0.1.0 and `midnight-thumbnail-sdk` 0.2.0 crates
define `MTH1` framed messages and a broker `Read + Seek` adapter. Helpers receive
request ID, source byte length, requested dimensions and sample index, never a
pathname, credentials or URL. Offset/length reads use a backend source lease
whose revision is checked. Native sources use `CapturedFile`; the host's
generic content adapter can support future virtual providers.

Reads are at most 256 KiB and must return the exact checked length. RGBA output
is bounded to 512 by 512 (1 MiB), with six sample indices. Host limits also cover
total traffic, read count, deadline, generation and dimensions. The service asks
for the DPI-scaled resolution bucket (96, 128, 192, 256, 384 or 512 px) that
covers the displayed thumbnail edge and admits two decoders. The shared I/O gate
admits one unknown, HDD or network source or a bounded batch of confirmed SSD
reads, releasing permits before decoding, hashing and waiting.

Videos use one static sample normally. Hover or selection admits five more;
hover has priority while every visible selected video progresses in rotation.
Offscreen jobs cancel. Decode failures retry after 30 seconds, including an
interruption the view did not request (for example a stopped archive mount);
only cancellations the view requested are abandoned without backoff. Every accepted
sample must persist before presentation. Private atomic cache records include
opaque identity, revision, decoder version and dimensions. Clear-cache waits
for cancelled workers, removes old records and starts a new visible generation
so late results cannot resurrect cleared entries. The shared store's generation
record is advanced under an exclusive cross-process transaction before each
mutation; a torn or missing record (for example after a crash between its two
writes) is rewritten with a fresh random generation by the next mutation, so cache
instances synchronize again instead of rescanning the store on every write.

## Independent builds and deployment

The `worker-packages` binary in `api/tools/package` produces a separate registry
bundle containing the worker, thumbnail, filesystem and image-viewer APIs/SDKs; it
fails if any of them is missing. Its
`.cargo/registry.toml` declares the registry without redirecting third-party
dependencies. Independent plugins pin registry versions and have no host path
dependencies. Existing viewer registry artifacts remain unchanged. Since 21 November
the bundle packages `midnight-filesystem-api` 0.5.0 (protocol 4: archive passwords and
member diagnostics, see [the filesystem API](FILESYSTEM_API.md)), which now depends on
crates.io `zeroize`; the archive plugin pins `=0.5.0`, so its lockfile checksum changes
with a rebuilt bundle. The worker SDK itself stays 0.2.0.

`scripts/build-thumbnailers.ps1` and `.sh` use that config with `--locked --offline`.
Supply `-NoticesRoot` on Windows or the fourth positional argument on Linux,
pointing to generated `_build/` notices with children `midnight-thumbnail-image/`
and `midnight-thumbnail-video/`. Each contains pinned dependency declarations and
original license/copyright files. `scripts/thumbnail-notices.py` reads each file
below its package root (the crate's manifest directory, the shared decoder notices
or the plugin directory) and refuses links, reparse points and `..` there; links
above that root, such as a linked cargo home, are accepted. Its output must lie
below the checkout's own `_build`. The build scripts' destination and build root
must lie below the checkout's own `_build` or, outside the checkout, below the first
`_build` of a custom target directory. That `_build` and every folder below it must
not be a link; the checkout and the folders above it may be one (Fedora Silverblue's
`/home`, a linked parent folder or `subst` drive) or hidden, such as `AppData`.
`scripts/prepare-thumbnail-ffmpeg.py` applies the same rule to its build root. A
verified FFmpeg prefix, its source archive and the libclang directory may be named
through links, and the prefix and libclang directory may themselves be links; nothing
inside the prefix may be one, except the versioned runtime library names, which must
resolve inside it. Its download cache must be a plain folder inside the checkout.
On Windows the package swap and the FFmpeg download and receipt renames wait for
about 2 s while endpoint protection holds a file just written.
Missing notices fail staging. The inventory
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
