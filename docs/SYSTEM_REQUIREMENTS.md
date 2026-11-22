# System requirements

Applies to Midnight Explorer **0.3.0**. Build and run commands are in the
[README](../README.md#build-and-run).

## Operating systems and support status

These are the project's compatibility targets. Not every listed OS, desktop, graphics
driver or filesystem has been tested; the pending tests are tracked as PLAT-01 and
PLAT-02 in the [implementation backlog](IMPLEMENTATION_BACKLOG.md).

| Platform | Minimum targeted OS / architecture | Tested so far and limitations |
| --- | --- | --- |
| Windows | Windows 10 22H2, **x64**, MSVC build; Windows 11 recommended | Native adapter implemented. Development checks run on Windows 11 Enterprise 25H2 (build 26200). Windows 10 has not been tested on its own yet. Use an edition that receives security updates. Windows 7/8, 32-bit, native ARM64 and MinGW builds are not targets. |
| Ubuntu | Ubuntu **24.04 LTS**, **x86-64**, GNU/glibc; newer LTS releases are targets too | Native adapter implemented. CI runs on Ubuntu 24.04; local Linux checks use Ubuntu 24.04.4 under WSL2 (kernel 6.6.87.2, glibc 2.39), which does not cover every physical desktop. Ubuntu 26.04 needs its own desktop and renderer tests. |
| Linux Mint | Mint **22.x**, **x86-64**, Ubuntu-based edition | Same Linux adapter and Ubuntu 24.04 library baseline. Not yet tested on a native Cinnamon, MATE or Xfce desktop; LMDE is not tested separately. |
| Fedora | **Fedora 44** and newer maintained releases, **x86-64**, GNU/glibc | Same Linux adapter. Native build, GNOME/KDE, SELinux-enabled operation and graphics tests are pending; Rawhide is not a target. |
| openSUSE | **Leap 16.x** and current **Tumbleweed**, **x86-64**, GNU/glibc | Same Linux adapter. Build, desktop, graphics and filesystem tests are pending; a test records the Tumbleweed snapshot it used, not a lasting guarantee for a rolling release. SUSE Linux Enterprise is not a target. |
| Other Linux | x86-64 GNU/glibc desktop with the dependencies below | Source builds may work; there is no distribution-independent binary guarantee. Older Ubuntu/Mint, musl/Alpine and ARM64 are not targets. |
| macOS | **Planned (PLAT-01); no supported version or architecture** | There is no macOS platform adapter, so the application does not build or run there. Versions, architectures and the RAM minimum will be set with that port. Slint's own macOS support does not change this. |

The Windows baseline also satisfies the Rust toolchain's Windows 10-or-newer
requirement ([Rust MSVC platform support](https://doc.rust-lang.org/rustc/platform-support/windows-msvc.html)).
Mint 22 uses the Ubuntu 24.04 base
([Mint 22 release notes](https://www.linuxmint.com/rel_wilma.php)).
openSUSE Leap 16 requires an **x86-64-v2** CPU; this is the OS baseline, although the
release build uses no host-specific CPU optimization
([Leap release notes](https://doc.opensuse.org/release-notes/x86_64/openSUSE/Leap/16.0/html/release-notes-leap-160/index.html)).

## RAM and hardware for running the application

The following are **provisional provisioning budgets**, not measured minimum-memory
guarantees: no constrained-RAM test has established the application's hard minimum
(PLAT-02). Total RAM includes the OS; application headroom means available memory
after the desktop and other applications are running.

| Platform | Minimum total RAM to provision | Recommended total RAM | Application headroom to reserve |
| --- | --- | --- | --- |
| Windows 10/11 x64 | **4 GB** | **8 GB or more** | **1 GiB** for ordinary browsing; **2 GiB or more** for large listings and concurrent work |
| Ubuntu 24.04 desktop x86-64 | **4 GB** | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; not measured on Linux yet |
| Ubuntu 26.04 desktop x86-64 | **6 GB** | **8 GB or more** | Same allowance, in addition to sufficient desktop headroom |
| Mint 22.x x86-64 | **4 GB** | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; not measured on Mint yet |
| Fedora / openSUSE x86-64 | **4 GB or the desktop's minimum, whichever is higher** | **8 GB or more** | Same 1 GiB / 2 GiB provisional allowance; not measured natively yet |
| Other Linux | At least the desktop's own requirement and the application headroom above | **8 GB** is a practical starting point | Depends on the desktop and workload; untested |
| macOS | **Not established** | To be measured when the adapter exists | Not a supported configuration |

Windows 11 itself requires 4 GB
([Microsoft requirements](https://learn.microsoft.com/en-us/windows/whats-new/windows-11-requirements)).
Canonical specifies 6 GB for Ubuntu Desktop 26.04
([release requirements](https://documentation.ubuntu.com/release-notes/26.04/#requirements-and-compatibility)).
Mint's own minimum is 2 GB, with 4 GB recommended; the 4 GB row above is a more
conservative product target, not Mint's minimum
([Mint FAQ](https://www.linuxmint.com/faq.php)).

Use a 64-bit processor that meets the chosen OS's requirements; two CPU cores are a
practical minimum for responsive background work. Release builds use the portable Rust
target baseline, not `target-cpu=native`, so AVX2 or a particular CPU make is not
required. An SSD is recommended for large trees and development, but browsing
supports HDDs and mounted network storage too.

Reserve **100 MiB** for the application itself and small settings, separate from OS
and runtime dependencies, plugins, thumbnails, copied files and build output; this is
a packaging allowance, not a measured installer size. A desktop of **1280 × 800
logical pixels** is recommended. The main window can shrink to 320 × 200 logical
pixels; font size, scaling and touch density affect how much fits.

Memory use depends on directory entry counts, native icons, cached tabs, searches and
queued jobs. Copy and checksums stream file contents, so copying a 100 GB file does not
need 100 GB of memory; queues, worker pools and caches are bounded. The headroom above
is neither a process-memory cap nor a guarantee for arbitrarily large directories.

One dated measurement: a short Windows 11 release startup on 2026-11-06 peaked at
**399 MiB working set** and **611 MiB sampled private memory**, with a large window,
the repository root and a three-entry Unicode fixture
([record](verification/2026-11-06-themes-and-commander-tools.md)). It shows that
512 MiB is not an established available-memory minimum, but it does not establish
worst-case use or a Linux result.

## Runtime software

Normal builds and distributions include all six viewers (basic, source, Markdown,
image, PDF and the Chromium HTML viewer), the platform's native and network
filesystem providers, the archive worker and both thumbnailers. Keep the complete
module folders (`viewers/`, `filesystems/`, `thumbnailers/`) beside the executable.

**All platforms:** a graphical desktop and readable system fonts that cover the
scripts being displayed. **Hardware GPU acceleration is optional.** The normal
renderer is Slint FemtoVG-WGPU:

- On Windows it allows Vulkan and Direct3D 12 and uses Vulkan when a Vulkan driver is
  present. The application sets no power preference, so wgpu takes the first suitable
  adapter; `WGPU_POWER_PREF=high` prefers a discrete GPU for a run, as does the
  per-application graphics preference in Windows Settings.
- On Linux it uses Vulkan, and all windows share one GPU device;
  `MIDNIGHT_SHARED_WGPU=0` gives each window its own device again, for comparison.
- An explicit `WGPU_BACKEND` (for example `dx12`, or `gl` for OpenGL on Linux) replaces
  the backend choice, for driver diagnosis.

The CPU renderer is available through `--software` and through automatic startup
recovery when the default GPU setup fails. The recovery makes one attempt, before the
first window is shown; it keeps the original paths, arguments and working directory.
A failed hidden attempt cannot save preferences or window geometry, and no visible
session or file operation is repeated. An explicit `SLINT_BACKEND` choice is never
replaced. CPU rendering still needs a graphical display; there is no headless mode.
CPU scrolling uses the same pixel scrolling and bounded loading, with performance that
depends on the hardware.

The Slint software renderer does not draw bitmap (color) emoji even when the font is
installed, so a passing smoke test does not prove complete Unicode rendering. For WGPU
diagnostics, `SLINT_WGPU_CPU=1` allows a CPU adapter while keeping the WGPU renderer;
it is distinct from `--software` and says nothing about GPU performance. See the
[scrolling contract](developer/SCROLLING.md) and the
[native scrolling measurements](verification/2026-11-11-native-wheel-scrolling.md).

**Windows:** install the Microsoft Visual C++ x64 runtime if it is not already
present; the MSVC release executable uses `VCRUNTIME140.dll` and the Windows Universal
CRT. Use Microsoft's
[supported Visual C++ redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist),
not individual DLL downloads. Rust, Cargo, Visual Studio, .NET and a separate Slint
SDK are **not** needed to run a built executable. The command line's `cmd.exe` and
Windows PowerShell are part of Windows; the PowerShell choice uses PowerShell 7
(`pwsh.exe`) instead when it is installed.

**Linux:** an X11 or Wayland desktop with fontconfig/FreeType, libxkbcommon, the X11/XCB
or Wayland client libraries, libudev, the Vulkan loader and the graphics driver's
runtime libraries. Desktop integration also uses `xdg-open` (`xdg-utils`) and an
installed terminal ([which terminals](../README.md#linux--ubuntu--mint)). External file
drag and drop works through X11/XWayland; under native Wayland, files can be dragged
only inside the application (PLAT-03).

In a source checkout, `sh scripts/setup-linux.sh --install` installs the build
packages and then these runtime libraries; it prints the plan without `--install`. On
Ubuntu 24.04 and Mint 22 the runtime packages are:

```text
libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 libx11-6 libxcursor1
libxi6 libxrandr2 libxcb1 libwayland-client0 libudev1 libvulkan1 xdg-utils
libnss3 libnspr4 libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 libatspi2.0-0t64
libcups2t64 libdbus-1-3 libdrm2 libexpat1 libgbm1 libglib2.0-0t64 libpango-1.0-0
libcairo2 libxcomposite1 libxdamage1 libxext6 libxfixes3
```

On Fedora and openSUSE the script installs the same libraries by soname (the README
lists the [package commands](../README.md#linux--ubuntu--mint)). The second half of the
list is what the HTML viewer's Chromium (CEF) loads: NSS/NSPR, ALSA, ATK/AT-SPI, CUPS,
D-Bus, DRM/GBM, Expat, GLib, Pango/Cairo and X11/XCB with its extensions. Desktop
installations normally have them. When one is missing, the dynamic loader stops the
viewer's browser before it starts, and the viewer names the missing libraries in the
page's place, with the command that installs them on Debian/Ubuntu, Fedora or
openSUSE (and, in a source checkout, the setup script). It finds them by reading the
ELF dynamic sections of the viewer's executable and of the libraries it needs, as the
loader looks them up; nothing is executed. CMake and Ninja are build tools, not
runtime requirements.

Local Network Neighborhood browsing additionally needs GIO/GVfs discovery and SMB
backends, and opening shares as native paths needs the GVfs FUSE bridge. On
Ubuntu/Mint, install `gvfs-backends gvfs-fuse wsdd`. The `wsdd` helper finds Windows
computers; an advertised GVfs backend alone does not prove that it is installed. A
known server can be opened as `smb://server/`.

### HTML viewer sandbox on Linux

On Ubuntu 23.10 and later (including 24.04 and Linux Mint 22), AppArmor denies
unprivileged user namespaces to programs without a profile that grants them
(`kernel.apparmor_restrict_unprivileged_userns=1`). Chromium's sandbox needs them
unless its `chrome-sandbox` helper is setuid root, so install the HTML viewer's
AppArmor profile once per installation. Chromium never runs without its sandbox.

The simplest way is the viewer itself. When it cannot show an HTML document for this
reason, it says so in the page's place and offers **Install…** (Tab from the page
reaches it and Enter activates it):

- The notice names the viewer that the permission is for. For an installation that can
  be changed without root (a per-user installation, an extracted package or a build
  folder), it warns that a program placed at the viewer's path there could use user
  namespaces too.
- Install… asks for administrator authorization through polkit (`pkexec`; the desktop
  session's authentication agent shows the prompt), installs the profile and opens the
  document again. Cancelling or refusing the prompt changes nothing, and Install… stays
  available; without an authentication agent in the session the viewer says so and
  withdraws it.
- The button is offered only when `/usr/bin/pkexec` and the folders above it belong to
  root, and never for an installation below a folder that other users can write to
  (such as `/tmp`). Nothing is checked or installed before an HTML document needs it.

Otherwise install the profile as root, but never by running with `sudo` a copy of the
script that you, or any program you run, can change, such as the one in an extracted
package or a source checkout: that copy could have been replaced, and would then run
as root.

- A system-wide installation, copied by root to a folder that only root can change
  (such as `/opt/midnight-explorer`):
  `sudo /opt/midnight-explorer/install-html-viewer-apparmor.sh`.
- An extracted package, a per-user installation or a source checkout: use Install…,
  which runs no file of the installation as root, or install system-wide first.

Where the kernel denies user namespaces to every program without administrator rights
(`kernel.unprivileged_userns_clone=0` on Debian and Ubuntu kernels, or
`user.max_user_namespaces=0`), no AppArmor profile helps: the viewer names the setting
and offers no profile. An administrator can change that setting, or make the
`chrome-sandbox` helper of an installation that only root can change setuid root. Check
these settings too when the viewer reports that the sandbox is still refused after its
profile was installed.

The profile, `midnight-explorer-html-viewer`, confines nothing and only grants `userns`
to that installation's `viewers/midnight-viewer-html/midnight-viewer-html`, like
Ubuntu's own browser profiles. `--print` shows it without installing and `--uninstall`
removes it. The machine has one such profile, so it serves one installation at a time;
installing it for another installation moves the permission there. An installation
that can be changed without root (a per-user installation or a build folder) lets any
program placed at the viewer's path create user namespaces, so prefer a root-owned
installation on shared computers.

Install… writes the same profile as the script (one that covers every build of a
source checkout), but runs no file of the installation as root; unlike the script it
does not remove other files that define the profile by hand. For both, the
installation path may contain only letters, digits, spaces and `._+-@:`, and may pass
through no folder that users other than its owner can write to. Both accept a folder
that its group can write to when that group is the owner's private group: the owner's
primary group, named like the owner, which no other account in `/etc/passwd` has as its
primary group and which lists no other member in `/etc/group` (a umask of 002 leaves
the folders a user creates like this on Debian, Ubuntu or Fedora). Without the profile,
and without Install…, the HTML viewer explains this instead of showing the page. It
names the command for its installation only when the script is no link and it and every
folder above it belong to root and only root can change them; otherwise (a per-user
installation or a build folder) it asks an administrator to install the profile as
described here.

### Video

Standard installations include the image and video thumbnailers in the
executable-adjacent `thumbnailers/` folder, with their runtime libraries and notices.
The pinned FFmpeg 9.0.2 libraries are bundled there, so running Midnight Explorer
needs no system FFmpeg and no libclang; compilers, GPG and libclang are build
prerequisites only. The bundled FFmpeg profile decodes VP8/VP9, Theora, FFV1 and raw
video (with Opus audio).

On Windows, every other video plays and gets thumbnails through Windows' own decoders
(Media Foundation), in platform helpers installed beside the bundled ones: H.264,
MPEG-4 Part 2, VC-1 and Windows Media Video, Motion JPEG and H.263 with AAC, MP3 or
Windows Media Audio, in MP4/M4V/MOV/3GP, ASF/WMV, AVI or Matroska files. Some codecs
need a free or paid Microsoft Store extension: HEVC (HEVC Video Extensions), AV1 (AV1
Video Extension), VP9 in MP4 (VP9 Video Extensions) and MPEG-2 (MPEG-2 Video
Extension); Windows N and KN editions need the Media Feature Pack for any of them.
Without the decoder, the viewer names the codec and what provides it.

On Linux, every other video plays and gets thumbnails through the distribution's own
decoders, which the platform helpers load at run time: first the distribution's FFmpeg
libraries (libavcodec 58 to 63, that is FFmpeg 4 to 9, with libavformat, libavutil,
libswscale and libswresample of the same release), then GStreamer 1.x and its plugins.
Neither is required: without them, only the bundled profile's codecs play, and the
viewer names the codec and the packages that add it. Optional packages, and what they
add for MP4/M4V/MOV/3GP, Matroska, AVI, Ogg and ASF (`.wmv`, `.asf`) files:

| Distribution | Packages | Adds |
| --- | --- | --- |
| Debian, Ubuntu and derivatives | `ffmpeg` (pulls `libavcodecNN`, `libavformatNN`, `libswscaleNN`, `libswresampleNN`) | H.264, HEVC, MPEG-4 Part 2, MPEG-1/2, AV1, VC-1/Windows Media, MJPEG, H.263, ProRes video; AAC, MP3, AC-3, E-AC-3, DTS, Vorbis, FLAC, Windows Media audio |
| | or `gstreamer1.0-libav` with `gstreamer1.0-plugins-base`, `-good` and `-bad` (and `-ugly` for ASF/WMV files) | The same through GStreamer (used when the FFmpeg libraries are absent); `-bad` adds OpenH264 and libde265, `-ugly` the ASF demuxer |
| Fedora, RHEL family | `ffmpeg-libs` from RPM Fusion (the full FFmpeg) | As above; Fedora's own `ffmpeg-free` omits some of these decoders, such as HEVC |
| | `gstreamer1-plugin-openh264` (Fedora's Cisco OpenH264 repository), `gstreamer1-plugin-libav` | H.264 through OpenH264; others through GStreamer's FFmpeg plugin |
| openSUSE | `ffmpeg` and `gstreamer-plugins-libav` from Packman | As for Debian; openSUSE's own FFmpeg omits patent-encumbered decoders |
| Arch and derivatives | `ffmpeg`, or `gst-libav` with `gst-plugins-good` | As for Debian |

The table names what these packages provide. Tested so far on Ubuntu 24.04 (FFmpeg
6.1, GStreamer 1.24): H.264, HEVC and MPEG-4 Part 2 video with AAC and MP3 audio
([Linux system decoders record](verification/2026-11-21-linux-system-video-decoders.md)),
and Windows Media Video 7 and 8 with Windows Media Audio 1 and 2 in ASF (`.wmv`, `.asf`)
through FFmpeg ([record](verification/2026-11-22-linux-html-libraries-and-asf.md));
WMV 9 and VC-1, which FFmpeg decodes but cannot encode, were not tested. GStreamer
also needs `gst-plugins-base` (typefinding and converters), `gst-plugins-good` (the
MP4, Matroska and AVI demuxers) and, for ASF/WMV files, `gst-plugins-ugly` (the ASF
demuxer); its Windows Media decoders come from gst-libav only. Hardware decoders
(VA-API, V4L2 and GPU vendors' decoders) are not used: the helpers decode in software
inside their sandbox. The distribution's packages and their licenses are the user's;
the application installs nothing.

### Linux distributions and filesystems

The Linux release build targets the **Ubuntu 24.04 / glibc 2.39** userspace baseline.
Build distributable Linux binaries on that baseline and inspect their ELF versioned
dependencies before publication: building on a newer distribution can raise the
required glibc and libstdc++ versions, and Rust's own older glibc minimum is no
guarantee for the complete GUI application. Rebuilding from source may permit other
configurations, but does not make them tested targets. An Ubuntu binary is not assumed
to run on Fedora or openSUSE merely because all three use glibc; native builds and
runtime tests there are pending (PLAT-02). Distribution RPM packages are planned, not
produced. Distribution packaging sits above the shared Linux platform adapter; there
are no separate Fedora or SUSE filesystem implementations, and capability detection
handles filesystem differences.

Security-sensitive local operations on Linux use `openat2` (kernel **5.6** or later)
and handle-relative APIs. Verified destination publication additionally needs
filesystem support for `O_TMPFILE` and `linkat`, and no-overwrite rename needs the
matching native rename capability; a new enough kernel alone does not guarantee these
on every mounted filesystem.

- Local ext4, XFS, Btrfs and tmpfs provide them.
- FAT/exFAT/NTFS removable media, NFS/SMB shares, sshfs and GVfs FUSE mounts (including
  shares opened from Network Neighborhood) cannot create `O_TMPFILE` files, so Copy into
  them fails with an explicit unsupported-filesystem error; copying from them works. A
  Move from another drive needs local ext2, ext3 or ext4 on both sides: the Copy/Move
  dialog refuses any other destination before the job starts, and the job refuses any
  other source.
- Rename and permanent Delete on such media need a no-replace rename (the vfat, exfat,
  ntfs3 and CIFS kernel drivers implement it; FUSE mounts such as ntfs-3g only when
  their daemon does) and a staging folder owned by you. The mount must therefore map
  file ownership to your user, as desktop automounting does; on a root-owned mount
  without `uid=` they are refused. The staging folder keeps the mount-provided
  permissions only if they let no other user write to it, or if its permissions, owner
  and group equal those of its parent folder, which other users can already change (for
  example a share mounted with `dir_mode=0777`); otherwise Rename and permanent Delete
  are refused there and the staging folder is removed. This follows the drivers'
  documented behavior and is covered by tests that simulate such mounts; it has not been
  verified on a live vfat, exFAT, NTFS or CIFS mount yet (PLAT-02).
- Trash uses the home Trash, only for items on the same filesystem; elsewhere it is
  refused, never replaced by permanent deletion.

Unsupported operations fail explicitly rather than weakening identity checks or
checksum verification. Newer optional metadata system calls have guarded fallbacks;
they do not raise the minimum kernel to the newest call's version.

## Building and testing

Build machines need much more than the runtime:

| Platform | Required development software | RAM / storage planning allowance |
| --- | --- | --- |
| Windows x64 | Pinned Rust **1.99.1** with rustfmt and Clippy ([rust-toolchain.toml](../rust-toolchain.toml)); Visual Studio's MSVC C++ build tools and a Windows SDK; PowerShell 7 and Python 3.12 or later with the `py` launcher, which `build.bat` checks; for the video thumbnailer a prebuilt FFmpeg 9.0.2 MSVC prefix, its source archive and libclang | **16 GB RAM recommended**; **32 GB** for concurrent builds or large test and link jobs. Reserve **20 GiB** free for one clean release build plus separate toolchain and download caches. |
| Linux x86-64 | Pinned Rust **1.99.1** (`build.sh` can install it), a C/C++ compiler and linker, pkg-config and the development libraries that `scripts/setup-linux.sh` installs, Python 3.11 or later (3.12 for full validation), `gpg`, libclang, CMake, Ninja, Perl, `xz` and `curl`; or Docker, which `build.sh` uses when native prerequisites are missing | Same allowance. A POSIX local filesystem is needed for the raw-filename and anonymous-file integrity tests; WSL's Windows-mounted paths do not exercise all Linux capabilities. |
| macOS | Not established; a future port needs Rust, the Xcode Command Line Tools and a native adapter (PLAT-01) | No verified build or RAM minimum. |

An **8 GB** development machine with `-j 1` and adequate pagefile or swap is a
low-memory configuration to try, **not a tested minimum**. Fat LTO can still need
substantial RAM for a single link; lowering Cargo's parallelism cannot remove that cost.
Cargo runs as many jobs as there are logical CPUs unless `-j N` or `CARGO_BUILD_JOBS`
limits them, and [full validation](../README.md#full-validation) takes `--jobs N`. See
[release settings](RELEASE_BUILD.md).

Several operating systems, profiles and test caches together can use **100 GiB or
more**; the allowance for one release build is not enough for an unlimited development
history. Compiled, generated and helper temporary output belongs under `_build/` (a
Linux build that shares a checkout with Windows uses `_build/linux/`). Optional
downloaded toolchains and dependency caches live under `.tools/`, separate from the
disposable `_build/` output. Neither is committed. Plugins document any additional
runtime or build requirements themselves.

Measured RAM minima are pending (PLAT-02): they need the release binary tested on
memory-constrained machines for cold start, large Unicode listings, scrolling, sorting,
searches and concurrent verified copies, with the OS, driver, fixture sizes, peak
private memory or RSS, responsiveness and swapping recorded. A short startup test alone
cannot establish them.
