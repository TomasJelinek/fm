# Linux native-provider viewer qualification — 8 November 2026

This checks the current Linux app's `viewers::service` composition with the
installed native filesystem provider and all five independent viewer packages.
The protected QA runtime is `/tmp/fm-interaction/_build/native-host-qa` in WSL;
authored fixtures and outputs remain beneath `_build`. No normal preferences or
production trust/revision rules are changed.

## Package provenance and the draft SDK lock correction

The starting runtime contained Basic only. Source/Markdown binaries in the old
Linux cache predated the source-coordinate work. All five packages are rebuilt
from their independent projects using existing Linux targets and locked/offline
resolution. They depend on packaged public SDK registries, without private app
crate dependencies or source patches.

The current text API/SDK bundle `plugin-sdk-1794112915607548500` matches all 21
current API/SDK source/header files. The selected image/worker bundle
`worker-sdk-1794092695373018700` matches all eight relevant public source files.
The archive bytes and exact lock checksums are checked before use.

Basic, Markdown and the standalone conformance fixture initially pinned the
previous draft API/SDK archive checksums.
A locked Basic build against the current bundle failed checksum verification.
The normal viewer helper passes one current SDK to all three text plugins, so
retaining the mismatch would also break that path. Per the public API README,
0.1.0 is still a versioned draft; frozen releases require a new version for content
changes, and the local packager publishes nothing remotely. The correction
regenerates all three independent locks against the current immutable
bundle. Only their two internal API/SDK checksum records may change; every
third-party package record, version and checksum must remain identical. Subsequent
builds and tests restore `--locked --offline`.

The complete source change is two checksum replacements in each of
`plugins/midnight-viewer-basic/Cargo.lock`,
`plugins/midnight-viewer-markdown/Cargo.lock` and
`plugins/viewer-conformance/Cargo.lock`:

| Internal draft package | Previous checksum | Current checksum |
| --- | --- | --- |
| `midnight-plugin-api` 0.1.0 | `51eab0fac11555c4224fe68abf188e9965bd75d5f9d6fed4264d9dd70d2f713e` | `7f1820f3ae94df7f377cc287fa7a7d5fee82b3061cbf4e78df861e70e03fc9e0` |
| `midnight-plugin-sdk` 0.1.0 | `4c576bfd7238e97263ad51eb7e51387774e57b7b149450b1959fb08bf2cb4f9d` | `8df0f3156505a88a84d67dd7e3a38807a284f0f61ee2922e739249a40aed915f` |

The conformance fixture is tested through its actual Linux SO and C loader;
it is not installed as a user viewer. Existing older SDK bundles remain intact.

Packages include their matching Linux manifest, executable/library, project
license, README, third-party notices and bundled license files. Publication into
the protected runtime replaces each completed file atomically, preserving any
old mapped inode; it does not claim a transaction across the whole package.

## Installed app-service assertions

The basic gate reads real content through `InstalledProvider`, advances with
`NextFile`, then verifies an unavailable provider emits explicit filesystem
context and the original cause, with no content frame or native fallback.

The all-format gate authors four files smaller than 16 KiB each and settles their
native revisions before launch. It selects Source automatically, checks exact
Unicode source coordinates and search progress to line 200, advances to rendered
Markdown without markup delimiters, then checks decoded bitmap dimensions and
RGBA pixels. The two-page PDF has distinct page sizes and solid red/blue content;
the test checks actual page raster pixels and next/previous navigation preserving
manual 1:1 zoom. This uses the same app capture closure as native F3.

The fixture's existing bounded age/quiet interval remains intact. It addresses
observed Windows scanner metadata changes and is not viewer startup work. Source
revision validation, package trust checks, worker sandboxing and plugin selection
are unchanged.

## Gates and artifacts

Independent package tests pass: Basic 27, Source 27, Markdown 34, image 10 and
PDF 6 (104 total). The actual C loader passes seven exported conformance ABI
scenarios, including the intentional contained panic, with no retained
jobs/services/buffers. All-target Clippy with `-D warnings` passes for all five
viewer projects and the conformance fixture.

Both installed app-service tests pass against the final rebuilt harness: basic
F3/NextFile/unavailable provider in 0.06s and all four selected formats in 8.23s,
including the eight-second fixture settling interval. The final combined app's
headless smoke reports 15 volumes and four entries in each pane, with empty
stderr. No production viewer or fixture change was needed for Linux.

After shared provider-tree code stabilized, the focused Linux engine gate passed
54 provider tests. Engine and app all-target strict Clippy passed, as did the
combined debug app and test-harness builds. The installed viewer tests and smoke
were then repeated with those final binaries. `final-app.sh` and `complete.sh`
record this combined sequence, which exited zero.

Logs, immutable input hashes, SDK checksum audits, complete package files and
installed artifact hashes are under `_build/goal/linux-viewer-cutover/`.
`build.sh`, `refresh-locks.sh`, `stage.py`, `integration.sh` and `strict.sh` record
the actual commands. These reuse the existing Linux Cargo caches and target
directories. The generated SDK configurations point to public packaged registries;
the passing dependency-resolution/build steps use the same current text SDK for
Basic, Source and Markdown. Final metadata confirms that each independent project
is its sole local package; all dependencies resolve from registries, including the
public SDK packages. No private app path dependencies or source patches are used.
A full optimized release-helper run is not claimed.

| Installed binary | SHA-256 |
| --- | --- |
| Native filesystem SO | `955d83786f0542adef8e95133e2864c8317921ce9bab9dd77d81d01fb26968d1` |
| Basic SO | `55586eb25ca33ad74d782317612feaf80571d2d4f95c88502e7ff44e360a1bcc` |
| Source SO | `0e5bf868152692f09d96ca2a962bc0667616807ce36c714b21c6fa6a86b2b767` |
| Markdown SO | `0c6af2d7da2fea277ce95abfa8690d27501ed90bbe00cf7c079887811bcf95d7` |
| Image worker | `5812775205a2fb9b6c04d205a55607afe686508fd9a72e2f4103e00519104e21` |
| PDF worker | `68330f3d3630f0b185249b2256b806f8deadb4037fde1e3292f244d3e96a5a0b` |
| App test harness | `375b1652a87bcf25555aec318024597d1b988e5d7b7c50eb4bfc12ab210dc055` |
| Combined app `me` | `8e40a4f13a31f15b3d6f9e1764cda24ef1435598269af389512699d2d3529183` |

The preceding baseline qualification used app SHA-256
`bd76f0423710760dbf0774b806725c78a2f2fab76e73b5a408d77fd14b2cf54f`.
Its logs and hashes are preserved as `baseline-*`; the installed app now has the
combined hash in the table above. All qualification process handles are terminal;
no test/image/PDF worker remained at the final audit. Viewer package directories
and files have no group/other write permissions, and all staged file hashes match
their build/package inputs. Linux Cargo and native acceptance lanes are released.

The shared cutover report/backlog can now replace the Linux basic-only limitation
with this all-format acceptance. CI/build-helper resolution must include the
three lock changes together with their current public API/SDK source closure;
publishing those locks alone against the older API archives would be inconsistent.
This qualification does not claim native pointer/keyboard gesture coverage,
compositor timing, Linux GPU performance, optimized release performance or a
remote CI run. Those require their own acceptance evidence.
