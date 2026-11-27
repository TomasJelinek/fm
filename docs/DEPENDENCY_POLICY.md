# Third-party dependency policy

Confirmed product requirement, 6 November 2026: every third-party library must
allow the intended development, use and distribution without mandatory license
fees, subscriptions or royalties. Prefer maintained open-source libraries.
This covers direct and transitive dependencies in the application, public API/SDK,
independent plugins, tests and build/release tooling.

Prefer compatible MIT, Apache-2.0, BSD, ISC and similarly permissive licenses.
Open-source copyleft is not automatically excluded: its conditions must fit the
way the component is built, modified and distributed. A free download, trial,
source-visible repository or optional paid license alone does not establish this.
For `OR` expressions choose a compatible free branch; `AND` combines obligations.
Check license files and notices in the pinned source, not just a crate's label.

## Dependency changes

1. Establish the needed capability, maintained upstream and smallest useful feature
   set. Reuse a suitable existing dependency before adding another.
2. Inspect the exact version's license, bundled code/data, native libraries and
   enabled optional components, including codec licenses where relevant.
3. Keep independent plugins' dependency locks and tests in their own projects.
   Sharing this Git repository does not permit dependency on private host crates.
   A plugin pins each registry dependency exactly (`=x.y.z`) to the version in its
   own lock file; `scripts/tests/test_plugin_dependency_pins.py` checks this.
4. Regenerate the [direct and complete inventories](THIRD_PARTY.md), update the
   [lock file entries outside the inventory](dependencies/LOCKFILE_ONLY.md), and
   preserve required copyright/license/source notices in each distributable artifact.
5. Check maintenance and RustSec advisories for the affected target/features.
   Record unresolved findings; a clean scan is not proof of safety.
6. Record the selected license branch and any concrete distribution obligations
   when a permissive default does not describe the actual component.

The inventory reports upstream declarations, not an automated compatibility verdict.
Missing metadata requires source inspection; the generator refuses an `UNKNOWN`
license. Do not add paid-only or trial-only dependencies. Do not assign or change
the license of Midnight Explorer or its public API as a side effect of this policy.

## Current decisions

**Midnight Explorer's own license (owner's decision, 22 November 2026):** the
application is licensed under the GNU General Public License, version 3 or (at the
user's option) any later version, `GPL-3.0-or-later`; the unmodified license text is the
repository's [LICENSE](../LICENSE) and ships in every release archive. The same
license covers every plugin under `plugins/` (each carries the GPL text as its
`LICENSE` and declares `license = "GPL-3.0-or-later"`) and the public API/SDK
workspaces (owner's decision, 23 November 2026), whose `LICENSE` files hold the same
text. The API/SDK crates' Cargo manifests still declare no `license` field: their
released archives' checksums are pinned in the plugins' lock files, so each crate gains
the field with its next version. Binary releases are accompanied by their
corresponding source: each release's tagged commit in the public repository, which the
release notes, the archives' `README.txt` and `BUILD-INFO.json` name. Every dependency
shipped with the application or linked into a plugin must allow distribution of the
combined work under GPL-3.0; `GPL-2.0-only`, CDDL, EPL-1.0, MPL-1.1, the original four-clause BSD license,
the OpenSSL/SSLeay licenses and non-commercial or no-derivatives terms do not.

**Slint 1.18.1:** the application uses Slint under the **GPL-3.0-only** branch of its
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`
declaration, which matches the application's own license and needs no paid Slint
subscription. Because that branch is version 3 only, the released binaries, which
combine Slint with the application, are conveyed under GPL version 3; the project's own
source remains GPL-3.0-or-later. The Slint Royalty-free License 2.0 would also permit
this desktop use but is not relied on, and the paid Software License alternative is not
selected. The application keeps the About Slint attribution and public release download
pages carry the Made with Slint badge in any case. See the
[pinned licenses](https://github.com/slint-ui/slint/tree/v1.18.1/LICENSES)
and [upstream licensing terms](https://slint.dev/terms-and-conditions).

**cbindgen 0.29.4:** MPL-2.0, used by the public API header-generation tool. It is
not linked into the file manager runtime. Preserve covered-source obligations
when modifying/distributing this tool; MPL's file-level scope does not automatically
apply its license to all application files. See the
[Mozilla MPL FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/).

**Swatinem/rust-cache 2.9.3 (CI only):** LGPL-3.0. CI's `scripts-and-lint`, `check`
and `release` jobs run this GitHub Action unmodified, pinned by commit, to cache the
compiled third-party dependencies of the root workspace. It is not linked into, built
into or distributed with Midnight Explorer, its plugins or its public API/SDK, so the
LGPL's conditions for conveying the library or a combined work do not arise; modifying
or redistributing the action itself would bring them. It uses the repository's GitHub
Actions cache, which has no fee. GitHub's own actions in the workflow are
MIT. See [CI time](verification/2026-11-21-ci-time.md).

**Windows installer tooling (26 November 2026):** the release MSI
([Windows installer](RELEASE_BUILD.md#windows-installer-msi)) is written by
`scripts/package-msi.py` with the Windows Installer API (`msi.dll`) through Python's
`ctypes`, and its cabinet with `makecab.exe` and checked with `expand.exe`. All three
are part of Windows, licensed with it and used unmodified on the packaging machine; no
installer toolkit is added and nothing of them is shipped. Alternatives considered on
that date:

- **WiX Toolset v6 and v7** (MS-RL source): the project's binary releases are offered
  under the Open Source Maintenance Fee agreement (`OSMFEULA.txt` in the WiX
  repository), whose fee "applies only to Users that use the Software as part of
  revenue-generating activities and have an annual gross revenue greater than or equal
  to US$10,000"; the repository's README says "if you use this project to generate
  revenue, the Maintenance Fee is required", and v7 enforces acceptance of the
  agreement (`-acceptEula`, `wix eula accept`). Self-compiled binaries are exempt, but
  building WiX needs Visual Studio 2026 with several workloads. A fee that becomes
  mandatory for revenue-generating users or forks conflicts with this policy's "no
  mandatory fees" rule for build tooling, so neither the binaries nor a self-build
  were chosen.
- **WiX v3.14 and cargo-wix** (which drives WiX v3): `wixtoolset/wix3` is archived;
  v3.14.1 (March 2024) is the last release, so it is unmaintained.
- **msitools `wixl`** (LGPL-2.1-or-later, maintained, 0.106; Ubuntu 24.04 packages
  0.103): free and suitable, but it needs Linux or MSYS2, while the Windows ZIP is
  built and checked on Windows. It remains the fallback if the built-in route proves
  insufficient.

**MSI validation and installation test (27 November 2026):** scripts/test-msi-install.py
runs the Windows SDK's ICE rules (darice.cub) on the MSI through the same msi.dll
API: it extracts darice.cub with xpand.exe from the SDK's own
MsiVal2-x86_en-us.msi and cabinet in Windows Kits\10\bin\<version>\x86 (installs
nothing, runs no msival2.exe), merges it into a copy of the MSI and runs its ICE
actions. The Windows SDK is free with Windows development (Microsoft Software License
Terms); it is used unmodified where it is already installed (CI's windows-latest
image, a developer's Visual Studio), never downloaded by the script or shipped, and
without it the check is skipped with a warning. The installation test itself uses only
msiexec.exe, Windows PowerShell's UI Automation and the GitHub CLI already on the
hosted runner, and the published MSIs it upgrades are checked against their signed
SHA256SUMS with scripts/release-signing.py.
**Native runtimes:** Windows/system graphics libraries remain governed by their
vendors; their use does not make the operating system open source. Linux libraries
retain their distribution's licenses. Rust bindings do not relicense native
libraries. See the separate native/runtime section of the [inventory](THIRD_PARTY.md).

**Required media workers:** standard builds include the independent image/video
thumbnailers and the installed video playback worker. `image` 0.25.10 is MIT OR Apache-2.0. The pinned FFmpeg 9.0.2 shared
runtime uses LGPL-2.1-or-later with GPL, nonfree, version3, networking and external
autodetection disabled. The profile enables approved VP8, VP9, Theora, FFV1 and rawvideo video decoders,
Opus audio, and the Matroska/WebM, AVI, Ogg and MOV/MP4 demuxers. The two video
workers use the same shared libraries, including libswresample for stereo PCM. Matching source, build configuration and notices
must accompany the replaceable shared libraries. `ffmpeg-next` 9.0.0 and
`ffmpeg-sys-next` 9.0.1 use WTFPL; preserve their pinned declarations and the
missing-upstream-license provenance recorded in the inventory. No paid decoder
or license branch is selected. See the [video worker guide](../plugins/midnight-thumbnail-video/README.md).

**Platform video decoders (Windows):** video in codecs outside the bundled FFmpeg
profile (H.264, HEVC, AV1, MPEG-2 and MPEG-4 video, VC-1/Windows Media, AAC, MP3,
Windows Media Audio) is decoded by Windows itself, through Media Foundation, in
separate platform helpers that link no FFmpeg. These decoders are Windows components
or Microsoft Store extensions (HEVC Video Extensions, AV1 Video Extension, VP9 Video
Extensions, MPEG-2 Video Extension, Web Media Extensions) that Microsoft licenses to
the user with Windows or with the extension; HEVC Video Extensions may be a paid Store
item or come with the PC. Midnight Explorer bundles, redistributes and links none of
them, adds no patent-encumbered codec of its own and pays no codec royalty, so the
project and its packages stay royalty-free; a missing decoder is named to the user
(the extension, or on N/KN editions the Media Feature Pack) and never downloaded or
installed by the application. The helpers bind Media Foundation with the `windows`
0.62.2 crate (MIT OR Apache-2.0), already in the inventory. Linux is decided below.

**Platform video decoders (Linux):** decided 21 November 2026 (user decision: "the
distribution's own FFmpeg, and if not available then GStreamer"). Video in codecs
outside the bundled profile is decoded by libraries the user's distribution installed,
which the separate Linux platform helpers load at run time: the distribution's FFmpeg
(libavcodec 58 to 63), else GStreamer 1.x with a fixed list of its plugins. Midnight
Explorer bundles, redistributes and links none of them and adds no patent-encumbered
codec of its own, so its packages stay royalty-free and carry no FFmpeg or GStreamer
code for this path. They keep their distributions' licenses: FFmpeg LGPL-2.1-or-later,
or GPL-2.0-or-later where the distribution enables GPL components (Debian's and
Ubuntu's builds do); GStreamer core and base/good/bad/libav plugins LGPL-2.1-or-later;
gst-plugins-ugly's ASF demuxer plugin (`asf`, listed for `.wmv` and `.asf` files)
LGPL-2.0-or-later (its sources' GNU Library General Public License,
version 2 or later, checked in `gstasf.c` and `gstasfdemux.c` of GStreamer's main
branch); some listed plugins GPL-2.0-or-later (faad, dtsdec, a52dec) and some BSD
(OpenH264, AOM, dav1d), with OpenH264 binaries under Cisco's binary license where
Cisco's build is installed. They are combined with the helper only in its process on the user's
machine, which those licenses permit without conditions on the application's own
distribution; nothing of them is conveyed with it. Codec patent licensing for these
decoders lies with the distribution and the user, as for any application using system
codecs. A missing decoder is named to the user with the distribution's package names
and is never downloaded or installed by the application. The helpers add no crate:
libc 0.2.191 (MIT OR Apache-2.0), already in the plugin's lockfile, provides `dlopen`.
FFmpeg's per-series struct offsets are generated from the releases' public headers at
development time (`scripts/generate-ffmpeg-system-layouts.py`); the headers are not
committed, only the resulting numbers. GStreamer's stable 1.x ABI is declared by hand.
A separate helper executable linked to GStreamer (gstreamer-rs, MIT/Apache-2.0) was
considered and not chosen: it would need GStreamer's development packages to build,
add the gstreamer and glib crates with their `-sys` and build-time crates, and still
need a run-time fallback when GStreamer is absent.

**GPL-built system FFmpeg at run time (owner's decision, 21 November 2026):** the Linux
platform helper may load a distribution FFmpeg that was built with GPL components (as
Debian's and Ubuntu's are) at run time, in its own sandboxed helper process. Nothing
GPL is shipped: no GPL code is bundled, linked at build time or redistributed with the
application or its packages. The project's own license is GPL-3.0-or-later, so this
stays: a distribution FFmpeg under GPL-2.0-or-later is compatible with it, and the
helper keeps accepting GPL builds.
