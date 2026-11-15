# Required runtime modules and main integration — 13 November 2026

## Scope

The user requested all application runtime modules in normal builds, Debug-only
local compilation, internal activation of `.tar.gz`, and integration of the
Linux branch into current main. This includes six viewers (basic, source,
Markdown, image, PDF, HTML), the target platform's native/network filesystem
providers, the archive worker, both thumbnailers and the existing elevation
helper. Other-platform implementations and developer probes are not runtime
modules for this installation. Independent modules continue to consume their
public SDKs and retain separate ownership and sandbox boundaries.

The integration base is main `e77eafaa0bd5d2ce75d831c4f4e39eec168b83a0`, with
feature head `e6d0dbd4c4553524ecae7ab0a3f284e28c334e94`. Historical Windows
checkpoint work in that main revision remains separate from the Linux task.

## Settings collision

Both branches had independently used schema 31: main introduced disk concurrency,
while the Linux branch introduced archive previews. Current schema 32 handles
both fields. A legacy schema-31 document must contain exactly one of those two
extensions; its missing choice gets the established default. Schema 32 requires
both extensions and earlier required fields. Ambiguous schema-31 forms, nulls,
wrong types, duplicates and unknown/newer schemas remain errors.

The new migration tests failed before the correction (archive: 4 passed/5 failed;
disk concurrency: 2 passed/11 failed). After the correction, the platform-common
library and both focused integration targets passed **117 tests**, zero failures
or ignored tests. The receipt and runner summary are under
`_build/qa/main-integration-20261113/codec-green*`.

## Archive activation

The failing installation lacked `filesystems/midnight.fs.archive`. UI activation
also depended on the asynchronous discovered-format list, allowing a configured
archive to reach the desktop association when discovery was pending or empty.
The regression fixtures cover ZIP, 7z and the reported
`ovclash-0.2.2-linux-x86_64.tar.gz`, both before discovery and with an empty result,
as well as an exact retained inline-tree root. Availability continues to control
badges; enabled native-file associations control internal activation. Backend
failure must leave the original native rows/path intact and never launch an
external archive application. Explicitly disabled/unknown associations and
symlinks keep their existing native activation policy.

## Build and dependency boundaries

Normal build helpers require every applicable module and propagate the selected
profile. Existing pinned dependencies and features are retained. The CEF Linux
minimal archive is exactly 333,351,579 bytes, SHA-256
`600ac0b8f37cb953f884e7c89998a6c200d24959aa74820ad431b61d5ebcf360`; the downloaded
archive matched that value. Its existing 929-file SDK inventory, complete CEF
LICENSE/CREDITS, Rust notices and runtime-file hashes remain mandatory. No browser
sandbox bypass, proprietary codec, Widevine or paid dependency is introduced.

The isolated Ubuntu 24.04 build image adds CMake/Ninja and system NSS/ALSA to the
previous Rust 1.99.1 thumbnail build environment. Its image ID is
`sha256:192c0f2f3c26d1968640dd7becc13f1b64ebabaedef307fdd71fd030ceaf06b3`.
No host packages or sandbox policy were changed. Updated native setup/bootstrap
checks passed **15 + 20** Python tests. More build/native receipts are recorded
below once executed; source or mock checks do not establish native runtime
qualification.

## Integrated Debug and installed archive checks

The corrected UI passed **112 targeted tests**: 44 archive/Options presenter
checks plus 13 drag, 11 Options transactions, 2 preview options, 9 reflow,
32 keyboard options and 1 compact-control integration checks. Before the fix,
the new activation regression failed because no `ArchiveNavigate` was emitted.
The independent archive worker passed **20 format/streaming/identity tests**.

The merged application and elevation helper built successfully in Debug. A native
owned-folder startup/snapshot/automatic-shutdown smoke passed. Rust formatting
and the 100,000-byte source-size check passed (2,285 handwritten source files at
that check). These are targeted results, not a full workspace suite.

The exact reported `/home/tjelinek/Downloads/ovclash-0.2.2-linux-x86_64.tar.gz`
was read through the production native-capture/engine/archive-worker path. Three
internal opens/listings succeeded (one root entry each, about 456–477 ms per open);
relisting took about 0.4 ms. The archive was neither extracted nor modified.
The first opt-in test invocation from `debug/deps` could not discover adjacent
archive modules; repeating the same artifact as an owned hardlink beside the
actual installation passed. Both logs are retained; this was test placement,
not a loader-policy change.

Packaging validation passed **58 Python tests**, with **4 Windows PowerShell
cases skipped** on Linux. It covers missing required modules, malformed payloads,
exact CEF inventory hashes/file sets and preserved ZIP/TAR executable modes.
The HTML/bootstrap/profile suites separately passed **21 focused tests**. Native
Windows deployment and Chromium runtime qualification are not implied by these
fixture-based checks.

Main's core preference and typed work-budget boundary suites passed **5 + 10**
tests. The Linux open/desktop group initially hit a Unix-socket test fixture path
longer than `sockaddr_un`; the fixture now creates that socket under a short,
owned `_build/tmp` directory. The corrected group passed **34 tests**, including
portable shell prompts, native/script activation and the fixed system Trash URI.
This changes test placement only, not application path handling.

The first real HTML build exposed an existing first-party SDK packaging mismatch:
the Windows helper used .NET/Ustar while Linux used Python/GNU tar. Both now use
the Python canonicalizer with an explicit portable gzip header (OS=255). Every
archived API source byte was checked against the current source; only the
project-owned API checksum in the HTML lockfile changed. No third-party lock
record, version or feature changed. The unified API 0.2.0 archive SHA-256 is
`5c3e958b2663c303c841dd20a7ce6c67518f1c9b894b3637df8f54905cc27768`.
The corrected real HTML Debug build passed checksum validation and completed
successfully, preserving the full staged CEF runtime and notices. The actual
installation passed `package-release.py validate-runtime`: all six viewers,
`native.linux`, `network.linux`, `midnight.fs.archive`, both thumbnailers and the
application/elevation executables were present and validated. This checks the
installed graph and runtime hashes, not native Chromium interaction on this
desktop. The final build/bootstrap suites passed 16 Python cases with one
PowerShell case skipped; an earlier invocation with a nonexistent extra test
module failed only at import and was corrected in the retained final receipt.
