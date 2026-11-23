# Midnight Explorer user guide

A dual-pane desktop file manager for Windows and Linux, written in Rust with Slint and
Tokio.

Product version **0.5** (Cargo SemVer `0.5.0`); the native window title shows
`Midnight Explorer 0.5 — File Manager`. The displayed version is derived from the
package version.

Prebuilt Windows/Linux previews are published on the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases). Published
preview archives may precede the current source; their bundled feature-status file
describes that particular build. See [feature status](FEATURE_STATUS.md) for what
is implemented and the [implementation backlog](IMPLEMENTATION_BACKLOG.md) for
planned work and known limits.

Lossless Unicode throughout the product, security and backend-only non-UI work
are explicit requirements. See [security and integrity rules](../SECURITY.md).
Third-party libraries must be free of mandatory license fees, with open source
preferred. See the [dependency policy](DEPENDENCY_POLICY.md),
[complete library/license inventory](THIRD_PARTY.md) and
[developer API documentation](developer/README.md).

Midnight Explorer provides independent local directory navigation, mounted-volume
switching, history, sorting, selection, mouse/keyboard navigation, background
metadata/capacity queries, native watches, a command editor and appearance
preferences. It also has tabs, Hot Paths, inline folder trees, panel filters, Find,
directory and file comparison, checksums, archive browsing, Network Neighborhood,
thumbnails, Custom Commands and configurable shortcuts; see [Controls](#controls).
Internal file operations include confirmed rename, recursive directory creation,
empty-file creation (toolbar or Shift+F4), and queued deletion (system Trash by
default, permanent deletion with Shift). Empty-file creation accepts a name; the name
of an existing file is asked about first, and only the Replace file button replaces
it, keeping the original in a recovery folder whose path the completion notice shows.
Explicitly opened documents use host associations; on Linux, executable programs
run directly and executable shell scripts run in a terminal,
and explicitly submitted shell commands run with
the selected user/sudo permissions. F3 opens an installed internal viewer; Alt+F3 forces
plain text, and F4 opens the configured external editor.
[Make File List](MAKE_FILE_LIST.md) (Ctrl+M, also on the top toolbar and in the
F10 Tools menu) exports the captured panel selection with Salamander-compatible
templates to the clipboard, the internal viewer or a UTF-8 file.

F5 and Ctrl+C/Ctrl+V copy files and folders through a background queue with
independent SHA-256 destination readback by default. Existing names prompt for Skip,
a different name, explicit folder Merge or recoverable Replace; replacement retains
the displaced original. F6 and Ctrl+X/Ctrl+V move native files and folders. Copy and
Move take up to 1,000,000 selected items at once; a large selection starts with its
first item at once and totals the rest while it copies. A Move
within one volume uses native rename without reading file contents. A Move from
another drive copies and independently verifies each file, then deletes that exact
source before continuing; recursive folders use the same per-file checks and remove
emptied source folders last. A Move from another drive goes wherever Copy goes, a
network location or a USB drive included: each file is copied and verified as Copy does,
and its original is removed only after that. On Windows a read-only file is moved as
Delete removes it. Moving from another drive works from any folder, from a USB drive
or from a network share: each file is copied and verified, other programs cannot change
it meanwhile, and then exactly that file is deleted. A file is left where it is, with the
reason, only if deleting it would lose data a copy cannot carry (on Windows, alternate
data streams) or cannot be done safely (on Linux, a file owned by another user, or one
on a network filesystem).
There is no source-recovery mode or journal.
Copy and Move (F5, F6) ask how to handle links, alike on Windows and Linux. **Copy the
files links point to** copies a link to a file as that file and a link to a folder as
the folder with everything in it, verified like any copy; a Move then removes only the
link itself, never the files it pointed to. **Keep links as links** copies or moves
each link with exactly the same target (relative targets stay relative). The default
follows the destination: a drive that cannot store links (FAT32, exFAT), a network
location on Windows, and a removable drive that the selection is not already on copy
the files, and the tooltip of the **Links** choice says why; elsewhere links stay
links. Choosing to keep links where none can be stored is explained in a note before
the job starts. A link that cannot be
followed (missing or unreadable target, a loop, a special file, a folder that contains
the link) stays a link, and the job details say why. On Linux a followed link may lead
into any filesystem but the kernel's own (/proc, /sys and the like). On Windows a
junction is recreated without privilege; a symbolic link needs Developer Mode or
administrator rights, and without them a directory symbolic link to a local folder
becomes a junction (the job details say so), while a symbolic link to a file or a
network share is left out with that reason. Moving archive members and full metadata
preservation are not implemented yet; the optional exact modified time accepts at most
64 regular files and no folders or links.
See [the transfer capabilities and implementation limits](developer/TRANSFERS.md).
On Linux, Copy writes only to filesystems with anonymous `O_TMPFILE` files (ext4,
XFS, Btrfs, tmpfs): FAT/exFAT/NTFS media and NFS/SMB, sshfs or GVfs shares are
refused as destinations. Rename and permanent Delete on vfat, exFAT, ntfs3 and CIFS
mounts are designed to work when the mount maps file ownership to your user, as
desktop automounting does; this is not yet verified on a live mount. See
[system requirements](SYSTEM_REQUIREMENTS.md).

Copy/Move pauses for supported Retry/Skip/Cancel decisions on I/O errors, including
an unavailable destination or a full disk. Retry preserves the verification policy
captured when the job started; Skip All applies only to the same error type and
phase in that job. Decide later keeps the pending decision available from Jobs. On
Linux and Windows, a file or folder inside a copied folder that cannot be opened (for
example one you may not read) pauses on its own: Retry opens it again, and Skip leaves
out only that item while the rest of the folder is still copied. Likewise, one of
several selected items that cannot be prepared offers Skip, and the other selected
items are still copied, unless the job requires the exact modified time: that refuses
the whole selection instead. On Linux, a Move within one volume of a folder you own
works even when the folder is write-protected (Linux needs the folder itself writable
to move it into another folder): it gets owner write permission only while it is
renamed and keeps its own permissions afterwards, and a Retry still works after you
changed a source's permissions to fix a failed Move. Delete pauses the same way on
each item it cannot remove: Retry, Skip (that item and the folders that contain it
stay, and the job goes on with everything else), Skip all errors (the job's later
failures are skipped without asking) or Cancel job. On Linux, write-protected folders
that you own inside the tree you confirmed for deletion are made writable as they are
emptied, so they do not stop it; the folder that contains the selection and anything
owned by another user are never changed. Copy, Move and Delete share one
progress-window component, with a separate modeless window for each operation.
The speed chart also shows overall completion; its scale follows the recent speed, so
a brief peak is cut off at the top. The small "i" button above the chart's top right
corner explains the chart: point at it, or Tab to it, click it or press Enter, for
what each line measures; the chart itself shows no tooltip. Right under the chart, a Copy or Move always
shows the file it is working on, by its path (the middle is left out when it is too
long), and that file's own bar and percentage, in the same place for every file: a
small file simply passes through to 100%, a large one fills its bar as it goes.
Discovery precedes known totals (`Files: 12 of
2,345 · Bytes: 1.5 MiB of 7.0 MiB` for Copy and Move, `Items deleted: 400 of 1,000`
for Delete); totals that could only be estimated, because part of the source could
not be read or a skipped file leaves part of the work undone, are marked `~`
(`Bytes: 1.2 GiB of ~3.4 GiB`, `~2 min`) and keep the bar, which holds short of 100%
if the work turns out larger. Pause/Resume waits for safe backend checkpoints. The
time estimate averages the measured byte and item rates; once the totals are known,
Calculating...
stands in for it until a first rate is measured, and a paused job keeps it, as in
`About 3 min (paused)`.
X, Escape and Cancel operation first ask whether to cancel that exact job, with
Continue as the default. Confirming requests cancellation, including cleanup of
unpublished temporary files; Continue leaves the job and its progress window as they
were, so closing never turns into silent background work.
Successful automatically opened progress closes. Manually reopened progress and
manually opened Jobs stay open. Finished jobs stay listed in Jobs for the session
until cleared there; Jobs also opens each job's progress window.
See [operation progress behavior and limits](developer/OPERATION_PROGRESS.md)
and [error recovery](verification/2026-11-08-transfer-errors.md).

Mouse file dragging uses the same verified copy queue for drops into either panel's
displayed directory. In flat panels, ordinary file rows and image tiles target the
panel's current folder; folder rows target that folder. Dragging a selected row keeps
the selected group; dragging an unselected row targets that item. Native exchange
with other applications uses Windows OLE or Linux X11/XWayland XDND. Native Wayland
currently supports internal panel dragging only. Internal drops move on a proven same
volume and otherwise copy; Ctrl forces Copy and Shift forces Move. A drag released
less than 0.4 s after the button press is cancelled wherever it ends, and other
applications receive a drag only once its press is that old. Incoming native drops
copy and retain sources; copying out is performed by the receiving application.
See [drag/drop controls](KEYBOARD_SHORTCUTS.md)
and the [native boundary](../SECURITY.md#file-drag-and-drop).

## Build and run

See [system requirements](SYSTEM_REQUIREMENTS.md) for OS versions, runtime
libraries, RAM and build-machine requirements. The compatibility targets are
Windows 10 22H2 and Windows 11 on x64, built with MSVC, and on x86-64 Ubuntu 24.04 LTS
and newer LTS releases, Linux Mint 22.x, Fedora 44 and newer maintained releases, and
openSUSE Leap 16.x and Tumbleweed. Fedora and openSUSE have not been tested on native
machines yet. macOS is planned (PLAT-01 in the
[implementation backlog](IMPLEMENTATION_BACKLOG.md)): there is no macOS platform
adapter, so the application does not build there, whatever Slint itself supports.
Provision 4 GB of total RAM (6 GB for Ubuntu 26.04), preferably 8 GB, with 1 GiB
available for the application. These are provisional budgets, not measured minima;
building the project needs much more memory.

Release builds use full optimization, fat LTO and one code-generation unit, and keep
overflow checks; see [release settings and verification](RELEASE_BUILD.md).

For Windows, or to run Cargo directly, install
[Rust with rustup](https://rust-lang.org/tools/install/) and reopen the terminal so
that `cargo` and `rustup` are on `PATH`; the Linux helper can install Rust itself, as
described below. The repository pins **Rust 1.99.1** with rustfmt and Clippy in
[rust-toolchain.toml](../rust-toolchain.toml). Install that toolchain once with
`rustup toolchain install 1.99.1 --profile minimal --component rustfmt --component clippy`
(current rustup no longer installs it through `rustup show`); Cargo then selects it in
this repository. The first build downloads Slint and the other locked dependencies and
takes longer than later builds; no separate Slint SDK is needed.

The commands below run from the repository root. The root build helpers, `build.bat`
on Windows and `build.sh` on Linux, find the repository themselves, so they also work
when started by path from another directory. Both:

- accept no profile (debug), `debug` or `release`, and `--validate`, which runs
  [full validation](#full-validation) after the build;
- run a locked Cargo build of the host, `midnight-explorer` (its executable is
  `midnight-explorer.exe` on Windows and `midnight-explorer` on Linux), and of the authorization helper, `midnight-elevation`;
- then build the platform's independent modules beside the host, with the same
  profile: the native and Network Neighborhood filesystem packages
  (`filesystems/native.{windows,linux}/`, `filesystems/network.{windows,linux}/`), the
  six viewers (`viewers/`), the image and video thumbnailers (`thumbnailers/`) and the
  archive worker (`filesystems/midnight.fs.archive/`);
- stop at the first failure and return its exit status. A missing module prerequisite,
  worker, runtime or notice fails the build; nothing is left out silently.

`build.bat` expects its prerequisites to be installed already. `build.sh` also accepts
`clean`, `--dry-run`, `--install-deps` and `--acceptance`, chooses a native or Docker
build and can install Rust; see the [Linux instructions](#linux--ubuntu--mint). The
helpers do not start the application.

Cargo's checked-in configuration writes all build output under `_build/`, and the
helpers keep their temporary files in `_build/tmp/`. Output paths below assume that
default and no `CARGO_TARGET_DIR` override.

`cargo build -p midnight-explorer -p midnight-elevation` builds only the host and the
helper. Native browsing and F3 file reads need the native filesystem package and have
no built-in fallback, so build the modules into the same output folder before starting
the host:

| Module | Windows (PowerShell 7) | Linux |
| --- | --- | --- |
| Native filesystem | `scripts/build-native-filesystem.ps1` | `bash scripts/build-native-filesystem.sh` |
| Network Neighborhood | `scripts/build-network-filesystem.ps1` | `bash scripts/build-network-filesystem.sh` |
| Viewers | `scripts/build-viewers.ps1` | `bash scripts/build-viewers.sh` |
| Image and video thumbnailers | `scripts/build-default-thumbnailers.ps1` | `bash scripts/build-default-thumbnailers.sh` |
| Archive worker | `scripts/build-archive-filesystem.ps1` | `bash scripts/build-archive-filesystem.sh` |

They build debug by default; pass `-Profile release` (Windows) or `release` (Linux) for
a release host. The archive worker reuses the public worker SDK that the viewer or
thumbnailer build prepared. See the
[filesystem API and build reference](developer/FILESYSTEM_API.md), the
[worker SDK guide](developer/WORKER_SDK.md) and the
[native filesystem build instructions](developer/NATIVE_FILESYSTEM_API.md#building-the-native-package)
for package-only builds, test switches and offline options. Release packaging
(`scripts/package-release.py`) requires all six viewers, the native, network and
archive filesystem providers and both thumbnailers, and keeps executable permissions,
notices and the FFmpeg corresponding source.

The default build is debug. To keep it responsive, the main workspace's debug profile
optimizes selected crates: the Slint renderers and WGPU, text shaping, fonts, regular
expressions, file comparison and viewer layout at level 2, hashing at level 3, and the
UI and viewer crates at level 1; other code keeps normal debug settings. The debug
application loads debug viewers, a debug archive worker and debug thumbnailers. The
viewer, archive worker and image thumbnailer workspaces build all their dependencies
(decoders, PDF, text and CEF bindings, SDKs) at level 3 and their own crate at
level 1, or level 2 for the image, PDF and HTML viewers; debug assertions, overflow
checks and symbols remain. Unoptimized decoders had made a 3840×2160 JPEG take 8.5 s
to open instead of 0.8 s ([measurements](verification/2026-11-20-viewer-speed.md)).

The HTML viewer build downloads the pinned CEF archive into `.tools/cef/`, verifies its
SHA-256 and keeps the complete runtime, its licenses and the Chromium credits.
Chromium's renderer sandbox stays enabled. On Linux, `CEF_PATH` can name an already
extracted SDK, which is checked against the pinned hashes. On Windows the verified SDK
stays in the checkout's `_build`: the provider compiles against it in place, the
package copies its runtime files from it and checks them against the hashes recorded
when the SDK was verified, and a rebuild with unchanged inputs keeps the installed
package. MSVC cannot compile CEF's C++ wrapper objects in a deep Windows checkout
(260-character paths): set `MIDNIGHT_HTML_TARGET_DIR` to a short absolute folder for
the provider's Cargo output, for example a junction directly below a drive root that
points to the checkout's `_build\mh` (inside the checkout it must be below `_build`).
The wrapper sources stay in the SDK, which needs a checkout path of at most about 125
characters.

The web SDK archives that the HTML viewer pins are classic-zlib deflate output.
Python 3.14 and later on Windows bundles zlib-ng, and so does the system zlib of some
Linux distributions (Fedora's zlib-ng-compat, which its Python uses). The web SDK
packager then stops with that explanation instead of producing archives that fail the
pinned checksums. On Windows it uses the first of `py -3`, `py -3.13`, `py -3.12` and
`py -3.11` that has classic zlib; on such a Linux host, build with
`FM_BUILD_BACKEND=docker` (the Ubuntu build image has classic zlib).
`MIDNIGHT_HTML_SDK_CONFIG` can name an existing web SDK bundle instead.

Linux thumbnail builds download the pinned FFmpeg 9.0.2 source and its signature,
verify both and build a restricted LGPL configuration; `gpg` and libclang are needed,
and a system FFmpeg is never used instead. Windows thumbnail builds need a prebuilt
FFmpeg 9.0.2 MSVC prefix, its source archive and libclang: set
`MIDNIGHT_FFMPEG_PREFIX`, `MIDNIGHT_FFMPEG_SOURCE_ARCHIVE` and `LIBCLANG_PATH`. The
[video worker guide](../plugins/midnight-thumbnail-video/README.md) describes the recipe
and the codec limits. Both platforms also build two helpers without FFmpeg,
`midnight-thumbnail-video-system` and `midnight-video-playback-system`, which decode a
video with the system's own decoders when the bundled FFmpeg profile has none for it:

- on Windows with Media Foundation: H.264, MPEG-4 Part 2, VC-1 and Windows Media
  Video, Motion JPEG and H.263 video, AAC, MP3 and Windows Media Audio, and with the
  free Microsoft Store extensions HEVC, AV1, VP9 and MPEG-2;
- on Linux with the distribution's own FFmpeg (libavcodec 58 to 63), else GStreamer
  1.x, both loaded at run time (the helpers have no rpath): H.264, HEVC, MPEG-4 Part 2,
  AAC, MP3 and the other codecs those libraries have.

Neither Linux library is required to run Midnight Explorer;
[system requirements](SYSTEM_REQUIREMENTS.md) lists the optional packages per
distribution. `MIDNIGHT_THUMBNAILERS_OFFLINE=1` requires cached downloads, and
`MIDNIGHT_THUMBNAIL_SDK_CONFIG` names an existing worker SDK bundle.

The [network plugin guide](../plugins/midnight-filesystem-network/README.md) describes the
optional Linux GIO/GVfs runtime and the unavailable and authentication states. On
Ubuntu and Mint, network discovery and SMB share browsing use
`gvfs-backends gvfs-fuse wsdd`; install them if missing. `wsdd` finds Windows
computers, and a known server can also be entered as `smb://server/` in a panel's path.

Folders that you may not read offer **Retry with authorization**: a one-time listing
through Windows UAC or Linux polkit (`pkexec` and a desktop authentication agent) by
`midnight-elevated-listing`, which the build helpers build beside the host. The helper
lists the requested folder only, loads no plugins, and does not elevate file
operations, change permissions or store passwords
([authorization boundary](../SECURITY.md#directory-authorization-helper)). Because it runs
as Administrator or root, the host uses it only from beside an installed `midnight-explorer` that
standard users cannot change: on Windows a folder on a local fixed drive whose
permissions only administrators can change, such as one below `C:\Program Files`; on
Linux a root-owned directory such as `/opt/midnight-explorer` (see
[Linux desktop launcher](#linux-desktop-launcher)). Build output, the Windows private
runtime, anything in a user profile and other user-writable locations are refused,
and so is a folder created directly below a drive root such as `C:\`, even by an
administrator, because it inherits Modify permission for all authenticated users. In
those installations the retry is still offered, and the Cannot open folder message
after it states the requirement. On Windows, install a release build from a PowerShell
started with **Run as administrator** in the checkout, then start Midnight Explorer
from that folder:

```powershell
$target = Join-Path $env:ProgramFiles 'Midnight Explorer'
New-Item -ItemType Directory -Force -Path $target | Out-Null
Copy-Item -LiteralPath _build\release\midnight-explorer.exe, _build\release\midnight-elevated-listing.exe -Destination $target
Copy-Item -Recurse -Force -LiteralPath _build\release\viewers, _build\release\filesystems, _build\release\thumbnailers -Destination $target
```

### Windows x64

Install 64-bit PowerShell 7 (`pwsh.exe`) and Python 3.12 or later with its `py`
launcher; the module build helpers run Python through `py -3`, or through a real
`python` when the launcher is absent. `build.bat` checks both before building.

Install the Visual Studio **Desktop development with C++** workload, including
the MSVC x64/x86 tools and a Windows SDK, as described in the
[rustup MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
Use the `x86_64-pc-windows-msvc` Rust host toolchain; the GNU/MinGW toolchain is not
supported. The full Visual Studio IDE is optional. The video thumbnailer also needs the
FFmpeg prefix, source archive and libclang described [above](#build-and-run).

Build from PowerShell or Command Prompt:

```powershell
.\build.bat
.\build.bat release
.\build.bat --validate
.\build.bat release --validate
```

`build.bat [debug|release] [--validate]` accepts its arguments in any order;
`--validate` alone builds debug. It runs `scripts\validate.ps1` after every build
step and before the private deployment described below, so a build that fails
validation is not deployed ([full validation](#full-validation)).

Host-only commands with a normal Rust installation (build the module packages with the
helpers above):

```powershell
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo build -p midnight-explorer -p midnight-elevation --locked --release
```

Run either executable directly, or let Cargo build and run it. After a successful
build, `build.bat` deploys the complete bundle to a new private installation (a
`%USERPROFILE%\MidnightExplorer-*` folder) and registers it for that build output by
its launch path and the host executable's SHA-256
([runtime resolution](developer/RUNTIME_DISCOVERY.md)). The build output's
`midnight-explorer.exe` then loads its plugins from that protected installation, even when the
checkout is on a shared drive. Without a registration, a build output that other
users can change loads its own plugins and reports the folder once
([shared installations](#shared-installations)). Each `build.bat` run removes the earlier installations it deployed
for the same build output, except any that is still running or selected by another
registration. Concurrent `build.bat` runs for the same build output in one logon
session deploy one at a time. After a raw Cargo build, or after adding or removing
plugin packages, deploy and register explicitly:

```powershell
powershell -NoProfile -File scripts/deploy-private.ps1 -SourceDirectory _build/release -IncludeFilesystems -IncludeThumbnailers -RegisterRuntime -CreateDesktopShortcut
```

Installations made this way (without `-ReplaceBuildDeployment`) are never removed
automatically. Use the original build executable, the **Midnight Explorer** desktop
shortcut, or the private installation path that the command prints. Restart after
deployment. `-IncludeFilesystems` deploys the filesystem packages and
`-IncludeThumbnailers` the image and video workers, with the Media Foundation helpers
that deployment requires beside the FFmpeg ones; the build deployment uses both and
rejects incomplete module installations. Private installations omit
`midnight-elevated-listing.exe`: **Retry with authorization** is still offered for a
denied folder, but refused with a message that names the requirement (see the
authorization helper above), and the deployment output names it too. To point the
desktop shortcut at an existing persistent installation without redeploying, run
`scripts/update-desktop-shortcut.ps1 -InstallationDirectory <installation> -ExpectedExecutableSha256 <sha256>`;
it refuses `build.bat` deployments because later builds remove them. Neither helper
launches the application or changes saved settings.

**Upgrading from earlier builds:** earlier versions left
`midnight-elevated-listing.exe` in folders that you can change (private runtimes,
release ZIPs extracted into your user profile, build output), and their executable
(then named `me.exe`) ran it as Administrator after the UAC prompt, so any program
running as you could have replaced it. Current builds refuse such a copy, so deleting
it loses nothing. Rebuild and redeploy (repeat `-CreateDesktopShortcut` if you use the
shortcut), then delete every older runtime that contains the helper, by default a
`%USERPROFILE%\MidnightExplorer-*` folder, and replace an earlier extracted release
with a current one, or at least delete its `midnight-elevated-listing.exe`. No script
removes these for you.

**Renamed executable:** releases up to 0.5.0 named the executable `me.exe`; it is now
`midnight-explorer.exe`. Settings stay in `%LOCALAPPDATA%\Midnight Explorer`. A
redeployment with `-CreateDesktopShortcut` replaces the Desktop shortcut; point other
shortcuts, pinned taskbar items, "Open with" choices and firewall rules that name
`me.exe` at `midnight-explorer.exe`.

Direct launches:

```powershell
.\_build\debug\midnight-explorer.exe
.\_build\release\midnight-explorer.exe --left "C:\Users" --right "C:\Program Files"
cargo run -p midnight-explorer --locked -- --left "C:\Users" --right "C:\Windows"
cargo run -p midnight-explorer --locked --release
```

For a filesystem smoke check that exits without opening the UI, wait for the
process and read its exit code. It needs the native filesystem package, which
`build.bat` installs. `midnight-explorer.exe` is a GUI-subsystem program: a console neither waits
for it nor shows its standard output, so its volume and entry counts and `--help`
text are visible only through `cargo run` (or a redirected stream):

```powershell
$smoke = Start-Process .\_build\debug\midnight-explorer.exe -Wait -PassThru -ArgumentList '--smoke-test', '--left', 'C:\Users', '--right', 'C:\Windows'
$smoke.ExitCode
cargo run -p midnight-explorer --locked -- --smoke-test --left "C:\Users" --right "C:\Windows"
```

`build.bat` uses `cargo` from `PATH`. Only when there is none and a toolchain exists in
the checkout's `.tools/cargo/` does it use [scripts/cargo.ps1](../scripts/cargo.ps1),
which you can also call directly; it prefers that local toolchain and keeps temporary
files in `_build\tmp`:

```powershell
.\scripts\cargo.ps1 run -p midnight-explorer --locked '--' --left "C:\Users" --right "C:\Windows"
```

Quote the literal `'--'` separator when forwarding application arguments through
that PowerShell helper. Regular `cargo` commands use the ordinary `--` separator.

### Linux / Ubuntu / Mint

Run `./build.sh` or `./build.sh release`. When the native C compiler (`cc`),
`pkg-config` or the development libraries it checks for are missing, the helper builds
in Docker instead: it prepares an Ubuntu 24.04 image with the dependencies and the
pinned Rust toolchain.
Docker must already be running and usable by your user without sudo. The first build
needs internet access; later builds reuse the image and the Cargo cache.

The container mounts the source read-only and runs `build.sh` with the native backend,
so it builds the same host, helper and modules as a native build. It writes output as
your user under `_build/`, whatever the host's `CARGO_TARGET_DIR`; Cargo downloads and
the container's home are cached under `.tools/docker/`. The resulting binaries target
Ubuntu 24.04 or a compatible system and need the usual desktop runtime libraries to
run outside the container.

To choose the build environment explicitly:

```sh
FM_BUILD_BACKEND=docker ./build.sh release
FM_BUILD_BACKEND=docker ./build.sh release --dry-run
FM_BUILD_BACKEND=native ./build.sh release
```

The default is `FM_BUILD_BACKEND=auto`, which selects Docker when the native compiler
or the pkg-config libraries are missing. Native mode uses the host libraries and can
install Rust in your user account without sudo; native package installation is
described below. When a native build lacks a module prerequisite (`python3`, `gpg`,
`perl`, `make`, `xz`, `cmake` or libclang), `build.sh` warns before building. If
Docker's default DNS cannot resolve package servers, set a DNS server for both the
image setup and the build, for example `FM_DOCKER_DNS=1.1.1.1 ./build.sh release`.
This does not change the host's DNS settings.

For native builds, `build.sh` uses Cargo from `PATH`, from
`${CARGO_HOME:-$HOME/.cargo}/bin` or from `.tools/cargo/bin`. If there is none, it
installs rustup with curl (CA certificates are needed) and then the pinned toolchain,
which needs `python3` 3.11 or later to read `rust-toolchain.toml`. New toolchain
caches default to `.tools/cargo/` and `.tools/rustup/` unless `CARGO_HOME` or
`RUSTUP_HOME` is set; this Rust setup needs no sudo.

For native builds, install the compiler, linker and development libraries of the
Linux build. The viewer build helpers also need Python 3.11 or later. The
[Linux dependency helper](../scripts/setup-linux.sh) recognizes Ubuntu and Mint, Fedora
and openSUSE, and prints a package plan by default:

```sh
sh scripts/setup-linux.sh
sh scripts/setup-linux.sh --install --dry-run
```

Only `--install` runs the package manager, with `sudo` when needed; `--dry-run` keeps
it a plan even with `--install`. The helper does not install Rust; native `build.sh`
can. After the build packages it installs the runtime libraries: the application's
desktop libraries, Vulkan and `xdg-utils`, and every shared library that the HTML
viewer's Chromium (CEF) needs (NSS/NSPR, ALSA, ATK/AT-SPI, CUPS, D-Bus, DRM/GBM, Expat,
GLib, Pango/Cairo, X11/XCB and its extensions). When one of those is missing, the HTML
viewer names the missing libraries and the command that installs them instead of
showing the page. Alternatively, install the packages manually. For Ubuntu 24.04 /
Mint 22 (the `t64` package names of those releases):

```sh
sudo apt update
sudo apt install build-essential python3 pkg-config curl ca-certificates gnupg xz-utils perl libclang-dev cmake ninja-build \
    libfontconfig1-dev libfreetype6-dev \
    libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libxcb1-dev libwayland-dev libudev-dev
sudo apt install libfontconfig1 libfreetype6 libxkbcommon0 libxkbcommon-x11-0 libx11-6 libxcursor1 \
    libxi6 libxrandr2 libxcb1 libwayland-client0 libudev1 libvulkan1 xdg-utils \
    libnss3 libnspr4 libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 libatspi2.0-0t64 \
    libcups2t64 libdbus-1-3 libdrm2 libexpat1 libgbm1 libglib2.0-0t64 libpango-1.0-0 \
    libcairo2 libxcomposite1 libxdamage1 libxext6 libxfixes3
```

Fedora 44+ (maintained desktop release, x86-64) installs the runtime libraries by
soname, a package capability that names no package:

```sh
sudo dnf install gcc gcc-c++ make python3 pkgconf-pkg-config fontconfig-devel freetype-devel \
    curl ca-certificates gnupg2 xz perl clang-libs clang-devel cmake ninja-build \
    libxkbcommon-devel libxkbcommon-x11-devel libX11-devel libXcursor-devel \
    libxcb-devel wayland-devel systemd-devel xdg-utils
sudo dnf install 'libfontconfig.so.1()(64bit)' 'libfreetype.so.6()(64bit)' \
    'libxkbcommon.so.0()(64bit)' 'libxkbcommon-x11.so.0()(64bit)' 'libX11.so.6()(64bit)' \
    'libXcursor.so.1()(64bit)' 'libXi.so.6()(64bit)' 'libXrandr.so.2()(64bit)' \
    'libxcb.so.1()(64bit)' 'libwayland-client.so.0()(64bit)' 'libudev.so.1()(64bit)' \
    'libvulkan.so.1()(64bit)' 'libnss3.so()(64bit)' 'libnssutil3.so()(64bit)' \
    'libsmime3.so()(64bit)' 'libnspr4.so()(64bit)' 'libplc4.so()(64bit)' 'libplds4.so()(64bit)' \
    'libasound.so.2()(64bit)' 'libatk-1.0.so.0()(64bit)' 'libatk-bridge-2.0.so.0()(64bit)' \
    'libatspi.so.0()(64bit)' 'libcups.so.2()(64bit)' 'libdbus-1.so.3()(64bit)' \
    'libdrm.so.2()(64bit)' 'libexpat.so.1()(64bit)' 'libgbm.so.1()(64bit)' \
    'libglib-2.0.so.0()(64bit)' 'libgobject-2.0.so.0()(64bit)' 'libgio-2.0.so.0()(64bit)' \
    'libpango-1.0.so.0()(64bit)' 'libcairo.so.2()(64bit)' 'libXcomposite.so.1()(64bit)' \
    'libXdamage.so.1()(64bit)' 'libXext.so.6()(64bit)' 'libXfixes.so.3()(64bit)'
```

openSUSE Leap 16.x / Tumbleweed (x86-64); then install the same runtime libraries as
for Fedora, with `sudo zypper install` and the same quoted capabilities:

```sh
sudo zypper install gcc gcc-c++ make python3 pkg-config xdg-utils \
    curl ca-certificates gpg2 xz perl clang-devel cmake ninja \
    'pkgconfig(fontconfig)' 'pkgconfig(freetype2)' 'pkgconfig(xkbcommon)' \
    'pkgconfig(xkbcommon-x11)' 'pkgconfig(x11)' 'pkgconfig(xcursor)' \
    'pkgconfig(xcb)' 'pkgconfig(wayland-client)' 'pkgconfig(libudev)'
```

The openSUSE command asks Zypper for pkg-config capabilities so that it selects the
distribution's development packages. The Fedora and openSUSE instructions have not
been tried on clean native machines yet. Keep the distribution's normal security
policy enabled. See
[Fedora's C++ setup](https://developer.fedoraproject.org/tech/languages/c/cpp_installation.html)
and the [Zypper manual](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

For command-line execution, the launcher uses the first installed terminal of this
list: the Debian/Ubuntu `x-terminal-emulator` alternative (your configured default),
then the current desktop's Konsole, GNOME Terminal or Xfce Terminal, the other two of
those, and `xterm` last. When the alternative points to one of these four, also
through a Debian `.wrapper` script, that terminal is started directly with its own
options; another configured terminal is started through the alternative with `-e`.
Ptyxis (Fedora Workstation's default) and GNOME Console are not recognized; install
one of the listed terminals, for example `sudo dnf install gnome-terminal` or
`sudo zypper install xterm`.

To install Rust manually, use the [official rustup instructions](https://rust-lang.org/tools/install/),
then open a new terminal or load rustup's environment with `. "$HOME/.cargo/env"`.
The Linux helper accepts
`./build.sh [debug|release|clean] [--install-deps] [--dry-run] [--validate] [--acceptance]`:

```sh
./build.sh
./build.sh debug
./build.sh release
./build.sh --dry-run
./build.sh --validate
./build.sh release --validate --dry-run
./build.sh --acceptance
FM_BUILD_BACKEND=native ./build.sh debug --install-deps --dry-run
FM_BUILD_BACKEND=native ./build.sh debug --install-deps
```

- With no profile, debug is selected. The helper requires Linux; on any other system
  it stops with exit status 2.
- `--install-deps` selects the native backend and installs the host packages through
  the dependency helper. It is rejected with `FM_BUILD_BACKEND=docker`.
- `--dry-run` prints the selected setup and build plan without installing packages or
  Rust and without compiling.
- `--validate` runs `scripts/validate.sh` after the last build step; with `--dry-run`
  it prints the validation plan and its commands as well
  ([full validation](#full-validation)). Validation needs the native backend: with
  `FM_BUILD_BACKEND=docker`, or when `auto` would select Docker, `build.sh` refuses
  `--validate` with exit status 2 before building, because the Docker build image has
  no X11 test display (Xvfb), mounts the source read-only and has not been tested for
  this.
- `--acceptance` runs `scripts/acceptance.sh` after the last build step, with either
  backend (the suite renders with Slint's software renderer and needs no display); see
  [the acceptance suite](developer/ACCEPTANCE.md). It never runs as part of
  `cargo test`, CI or `--validate`; CI only checks that it compiles. With
  `--validate --acceptance` both run, and the first failure decides the exit status.
  On Windows, `pwsh -File scripts\acceptance.ps1` builds and runs the suite (`--list`,
  `--only`, `--scale`, `--dry-run`; see the acceptance guide).
- `clean` cannot be combined with `--install-deps`, `--validate` or `--acceptance`.

Build only the host with Cargo (build the modules separately as described above):

```sh
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo build -p midnight-explorer -p midnight-elevation --locked --release
```

Preview or remove repository build output with:

```sh
./build.sh clean --dry-run
./build.sh clean
```

Clean removes only the repository's `_build/`, whatever `CARGO_TARGET_DIR` says, so
native output in a custom `CARGO_TARGET_DIR` must be removed separately. It keeps
`.tools/` (including the `.tools/docker/` caches), the Docker dependency image and the
legacy `_build/docker/` cache. It refuses a symbolic-link `_build` and checks the Linux
mount table before removing anything: a mount or bind mount below `_build/` (outside
the kept `_build/docker/`) stops it, and so does missing, unreadable or malformed
mount information. It needs no Cargo, Docker, network or sudo.

Run in a graphical desktop session with X11 or Wayland and working graphics
drivers. Slint's [Winit backend documentation](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/)
describes the display and runtime libraries. The normal renderer is FemtoVG-WGPU.

```sh
./_build/debug/midnight-explorer
./_build/release/midnight-explorer --left "$HOME" --right /tmp
cargo run -p midnight-explorer --locked -- --left "$HOME" --right /tmp
cargo run -p midnight-explorer --locked --release
```

The filesystem smoke check does not require a display; like browsing, it needs the
native filesystem package:

```sh
./_build/debug/midnight-explorer --smoke-test --left "$HOME" --right /tmp
```

Linux discovers already-mounted filesystems through `/proc/self/mountinfo`;
mounting or safely ejecting devices is not implemented.

For other Cargo commands, [scripts/cargo.sh](../scripts/cargo.sh) keeps temporary
files in `_build/tmp/` too, for example `sh scripts/cargo.sh test --workspace --locked`.
It forwards its arguments to `cargo` on `PATH` and returns Cargo's exit status.
When Windows and WSL use the same checkout, give Linux its own output folder:
`CARGO_TARGET_DIR="$PWD/_build/linux" sh scripts/cargo.sh test --workspace --locked`.

### Linux desktop launcher

Ubuntu GNOME resolves the running application's moon icon through an installed
desktop entry, including in X11 sessions. Native Wayland also uses the matching
application ID and desktop entry. Register a debug build directly from the
repository root:

```sh
python3 -B scripts/install-linux-desktop.py
```

For another existing executable, use
`python3 -B scripts/install-linux-desktop.py --executable _build/custom/debug/midnight-explorer`.
The script installs the desktop entry and the committed icon under `$XDG_DATA_HOME`
(default `$HOME/.local/share`), with an absolute executable path, and invalidates
the icon theme cache. It leaves the application and its plugins in place and needs
no change to the graphical session's `PATH`. Restart Midnight Explorer after
registration to refresh its running icon. Register again after moving the checkout or
selecting another build.

For a separate per-user installation of the debug build:

```sh
install -Dm755 _build/debug/midnight-explorer "$HOME/.local/bin/midnight-explorer"
mkdir -p "$HOME/.local/bin/viewers" "$HOME/.local/bin/filesystems" "$HOME/.local/bin/thumbnailers"
cp -R _build/debug/viewers/. "$HOME/.local/bin/viewers/"
cp -R _build/debug/filesystems/. "$HOME/.local/bin/filesystems/"
cp -R _build/debug/thumbnailers/. "$HOME/.local/bin/thumbnailers/"
python3 -B scripts/install-linux-desktop.py --executable "$HOME/.local/bin/midnight-explorer"
```

In a per-user installation, **Retry with authorization** is still offered but is
refused with an explanation: the root helper would come from a directory that its
user can change. For authorized listings, install system-wide into a directory that
only root can change (root-owned, not group- or world-writable, on a filesystem not
mounted `nosuid`):

```sh
sudo install -d -m 755 /opt/midnight-explorer
sudo install -m 755 _build/debug/midnight-explorer _build/debug/midnight-elevated-listing /opt/midnight-explorer/
sudo cp -R _build/debug/viewers _build/debug/filesystems _build/debug/thumbnailers /opt/midnight-explorer/
sudo install -m 755 scripts/install-html-viewer-apparmor.sh /opt/midnight-explorer/
python3 -B scripts/install-linux-desktop.py --executable /opt/midnight-explorer/midnight-explorer
```

Where AppArmor restricts user namespaces (Ubuntu 23.10 and later), the HTML viewer of
this installation then needs its profile once:
`sudo /opt/midnight-explorer/install-html-viewer-apparmor.sh`. The machine has one
such profile, so installing it for another installation moves the permission there.

**Upgrading from earlier builds:** earlier versions left `midnight-elevated-listing`
in directories that you can change, and their executable (then named `me`) ran it as
root through polkit even there: these instructions installed
`$HOME/.local/bin/midnight-elevated-listing`, release archives contain it beside the
executable (so an archive extracted into a folder you own has it there), and build
output holds it beside the executable. Any program running as you could have replaced
it. Current builds refuse such a copy, so deleting it loses nothing: delete
`$HOME/.local/bin/midnight-elevated-listing`, rebuild, and replace an earlier extracted
release with a current one, or at least delete its `midnight-elevated-listing`.

**Renamed executable:** releases up to 0.5.0 named the executable `me`; it is now
`midnight-explorer`. Settings stay in `$XDG_CONFIG_HOME/midnight-explorer`. Run
`scripts/install-linux-desktop.py` again: it replaces the desktop entry, whose `Exec`
named the old executable. Delete an earlier per-user copy such as
`$HOME/.local/bin/me`.

Registration uses only the dedicated `io.github.TomasJelinek.MidnightExplorer`
desktop-entry and icon paths. It refuses symbolic links in those paths and an
unrelated existing desktop entry. These commands do not make Midnight Explorer the
default file manager.

### Rendering diagnostics

GPU acceleration is optional. Normal rendering uses Slint FemtoVG-WGPU with
system-font fallback. On Windows it allows Vulkan and Direct3D 12 and uses Vulkan
when a Vulkan driver is present. The application sets no power preference, so wgpu
takes the first suitable adapter the drivers list (on a laptop with two GPUs usually
the integrated one); `WGPU_POWER_PREF=high` prefers a discrete GPU for a run, as
does the per-application graphics preference in Windows Settings. An explicit
`WGPU_BACKEND`, such as `dx12`, replaces the backend choice. On Linux the renderer uses
Vulkan (`WGPU_BACKEND=gl` selects OpenGL), and all windows share one GPU device, which
saves time when a viewer window opens; `MIDNIGHT_SHARED_WGPU=0` gives each window its
own device again, for comparison.

When default GPU selection or initial window/device setup fails before the first
window is shown, the app retries once with Slint's CPU renderer. The first process
waits for that software child and exits with the child's own status (codes that do
not fit an exit byte become 1). Existing file operations and already visible windows
are never restarted by this startup recovery. An explicit `SLINT_BACKEND` choice is
respected and is never replaced by the retry; `--software` selects CPU rendering:

```sh
cargo run -p midnight-explorer --locked -- --software
```

The UI smoke check and `--snapshot FILE.png` need a working display; see
[Checks](#checks) below. The software renderer does not draw bitmap (color) emoji.
For WGPU diagnostics only, `SLINT_WGPU_CPU=1 ./_build/debug/midnight-explorer` permits a CPU adapter.
This still needs the graphics runtime and fonts, and says nothing about GPU
performance.

### Full validation

`build.bat --validate` and `./build.sh --validate` run, after the build,
[scripts/validate.ps1](../scripts/validate.ps1) (Windows) or
[scripts/validate.sh](../scripts/validate.sh) (Linux). Each also runs on its own, for
example `pwsh -NoProfile -File scripts\validate.ps1` or `bash scripts/validate.sh release`.
They run the tests that CI ([ci.yml](../.github/workflows/ci.yml)) runs for the host
platform, with CI's own test commands and in CI's job order, and build only what those
tests need:

- the public API/SDK and protocol workspace tests, the C/Rust ABI layout test and
  the plugin, worker and web SDK release bundles (CI's `public-sdk` and
  `public-worker-sdk` jobs run on Linux only; Windows validation runs them as well,
  since its plugin steps need the bundles);
- the tests of the independent viewers, the viewer ABI conformance fixture, the image
  and PDF viewers, the image thumbnailer, the archive worker (with its release package
  and fixture export) and the filesystem API with the native and network filesystem
  plugins, with CI's test commands (for the viewers those of their `scripts/ci.*`)
  and the package builds that the private test installation needs;
- the tests of CI's `scripts-and-lint` and `check` jobs: the script and native helper
  tests, the PowerShell deployment tests under PowerShell 7 and 5.1 (Windows), every
  workspace test target including doc tests (CI's own
  [gates](developer/TEST_EXECUTION.md#full-validation), retiring nothing and
  running every gate), and the ignored tests that need installed packages, against a
  private test installation of the packages built above; on Linux also the Trash and
  isolated X11 tests;
- CI's `release` job for the selected profile (default debug): the host build and
  its installed filesystem smoke test.

Two steps that CI does not run are included when their inputs exist: the HTML
viewer's tests without native CEF, and the video thumbnailer's tests with an FFmpeg
9.0.2 prefix (on Windows from `MIDNIGHT_FFMPEG_PREFIX`,
`MIDNIGHT_FFMPEG_SOURCE_ARCHIVE` and `LIBCLANG_PATH`; on Linux the one a normal build
prepared). That video step also runs the tests of the platform decoder helpers, which
CI runs in its `thumbnail-video-platform` job; validation has no separate step for that
job, so without an FFmpeg prefix those tests do not run either. Tests always use the
debug test profile; the independent modules use the profiles their CI jobs use. A step
whose prerequisite is missing is skipped with the reason, never silently left out: for
example PowerShell 7 or the MSVC tools for the AV1 decoder bootstrap, a classic-zlib
Python for the web SDK, cached pinned downloads in an `--offline` run, Xvfb, or the
other platform's steps. A step whose required step failed or was skipped is skipped
too. Validation continues after a failure and ends with a table of every step (PASS,
FAIL or SKIPPED, duration and passed/failed/ignored test counts); step logs, the
summary and, on Windows, the generated step scripts stay in
`_build/validate/logs/<run>/`.

Exit status: 0 when no step failed, 1 when one failed (or, with `--strict`, when a
step was skipped for a missing prerequisite or a step it needs; other-platform steps
never count), 2 for invalid arguments and 127 when cargo, rustc (it must be on `PATH`)
or Python 3.12+ is missing. Without `--strict`, a run in which a CI step was skipped
that way still passes, but a `WARNING: N CI step(s) SKIPPED ...` line before the result
names those steps. `build.bat` returns that status, and `build.sh` exits with it. The
scripts also take:

- `--offline` (as does `CARGO_NET_OFFLINE=true`), which forbids downloads;
- `--dry-run`, which prints every step with its commands and the reason for any skip;
- `--jobs N`, which limits every build and test to N parallel jobs and test threads
  (default: all logical CPUs);
- `--only ID[,ID...]`, which runs the named steps and the steps they need;
- `--plugin-sdk`, `--worker-sdk` and `--web-sdk`, which test against an already
  packaged bundle, as CI's plugin jobs do.

A cold first run compiles every independent module and the workspace tests; see the
[test execution guide](developer/TEST_EXECUTION.md#full-validation) for the
steps, the environment and how validation differs from CI.

## Controls

See the [complete keyboard map](KEYBOARD_SHORTCUTS.md) for every default key,
where it applies and which keys stay with text editing. Options → Keyboard can
change, remove or restore each binding; menus, tooltips and the footer show the
current keys.

- Ctrl+A selects top-level files only; Ctrl+Shift+A includes top-level folders.
  Both replace selection and clear nested marks. Ctrl+D clears all selected files
  and folders, including marked inline descendants; Ctrl+Shift+D has no default binding.
  Ctrl+I inverts files only; Ctrl+Shift+I inverts top-level files and folders.
  This works inside archives too. The parent `..` is excluded; text editors retain
  normal editing. Alt+Up / Alt+Down move the cursor to the previous / next selected
  item, without wrapping.
- Ctrl+= or keypad + opens **Select files**, and Ctrl+− or keypad − **Deselect
  files** (no numeric keypad needed): type
  masks with the panel filter's syntax (`*.txt;*.md`, `?`, `|` before exclusions,
  letter case ignored) and press Enter. The dialog starts with the last mask (`*` at
  first) and its **Include folders** choice, which is saved with the settings. Only
  shown rows change, and marks on rows that do not match stay. In inline trees the
  subfolders of expanded branches take part too when Include folders is chosen.
  Keypad * inverts files, and folders as well when Include folders is chosen
  (without a keypad, Ctrl+I or Ctrl+Shift+I). The number-row + − * keys type into
  filename search; F10 → Selection offers the same three commands.
- Ctrl+H (**Show hidden files**) hides or shows dot-names on Linux and macOS and
  entries with the hidden attribute on Windows, in both panels, archives and inline
  trees. It is also in F10 → Panels and navigation, each panel's View menu and
  Options → General, and is saved; hidden entries are shown by default. The panel
  re-reads in place: the cursor moves to the nearest shown row, marks on hidden
  entries are dropped and the status line counts the hidden entries. Operations, Ctrl+A, folder sizes
  without a selection and Compare directories work on the shown entries; a selected
  folder is still copied, moved, deleted or measured with all its contents. Find
  searches hidden entries too.
- Type in the file pane to search filename prefixes, always Unicode
  case-insensitively. The matching prefix is colored; Up/Down visits matches and
  Escape exits. **Show only filename search matches** in Options → General hides the
  other names while typing. The search only moves the cursor: F3, F5, F8 and the
  other commands close it and act on the selection or the focused row, even one
  that does not match. Enter opens only a matching row; otherwise it opens nothing
  and the status line says that no name matches.

- Alt+2 / Alt+3 / Alt+5: Brief / Detailed / Thumbnails view in the active pane, also
  offered in each panel's View menu and in F10 → Panels and navigation.
- Tab / Shift+Tab: switch the active file pane.
- Ctrl+T / Ctrl+W: add / close a folder tab in the focused pane; the last tab stays.
- Ctrl+Tab: focus the command editor; press again to return to the active pane.
  Ctrl+Shift+Tab selects the previous folder tab. Captions follow the folder name;
  full paths are in tooltips. Tabs restore their order, locations and active position
  after restart; explicit `--left`/`--right` arguments override that pane. Such a
  start path is saved with the next change that saves preferences (a setting, a tab
  or a panel's folder), not at startup: a start that changes nothing writes no
  preferences, also not at exit. Middle-click also closes a tab, and a double-click
  on the empty space after the tabs opens a new one. Tabs shrink uniformly before
  revealing scroll arrows.
- Ctrl+U: swap the complete panels while keeping their operations and active content.
  Drag the divider for unequal widths; double-click it to center the panels.
- Ctrl+Shift+1–9: select a numbered tab. Tabs sit below the drive toolbar.
- Hot Paths save folders, folders inside archives and Network locations, each
  location once and at most 64, shown by their location without separate names. A
  new one takes the first free number of **1–9, 0**, which the menus show; pressing
  that digit in a focused panel opens it. Editors and filename search keep normal
  digit input. Ctrl+number and Shift+number are aliases; Ctrl+Alt+number and
  Alt+Shift+number open the path in the other panel. Ctrl+Shift+Plus or
  Ctrl+Shift+= adds the current folder, and Ctrl+Shift+0 assigns it number 0. The
  manager keeps a clicked path selected while its number is edited. Saved paths also
  appear in the full drive dropdown, without extra buttons on the drive toolbar.
  See the [archive](ARCHIVE_FORMATS.md) and
  [Network](developer/NETWORK_FILESYSTEM.md) details.
- Left click: focus the row and its pane; Ctrl+click toggles selection, Shift+click
  selects an inclusive range, Ctrl+Shift+click adds that range. The range starts at
  the row last clicked or Ctrl+clicked, the same in Detailed, Brief and Thumbnails;
  once the cursor moved without Shift (the cursor keys, Space, Insert, a filename
  search or a jump to a selected row) or Deselect all, Select all or Invert changed
  the selection, it starts at the cursor's row instead. Accepted gestures
  appear immediately while backend confirmations arrive. Panel status shows
  selected file/folder counts.
- Enter / unmodified left double click: enter a directory or open a file in its
  default system application. On Linux, executable ELF programs run directly, and
  executable files that start with `#!` and executable `.sh` files run in a
  terminal; other files, including those without execute permission, open in their
  default application. Directory links stay inside the file manager.
- Each panel's details strip shows the focused name, exact byte size, date/time and
  attributes: attribute letters on Windows, explained in the tooltip, or permissions
  such as `-rwxr-xr-x (0755)` on Linux. Click a field to copy its value; Ctrl+click
  the name to copy the full native path. The free space at the right copies too.
- Backspace, Ctrl+PageUp or the toolbar's Parent folder: parent directory.
  Shift+Backspace, Ctrl+Backspace or Ctrl+\ goes to the root: of the drive or
  filesystem, of the archive inside an archive, or of Network in a Network location.
  Ctrl+PageDown enters the focused folder and never opens a file.
- Alt+Left / Alt+Right: back / forward history; adjacent dropdowns choose a
  particular history entry. Click a highlighted path prefix to visit its ancestor.
  Right-click the prefix to copy it to the clipboard without navigating.
- Arrow keys, Page Up/Down, Home/End: move the cursor.
- Space / Insert: toggle selection and advance, clamped at the last row. Space
  also measures the focused folder's size; links inside it are not followed.
- Ctrl+R, Ctrl+F9 or toolbar Refresh: reload the current directory.
- F10 opens the main menu with file operations, selection, panel navigation/view
  controls, Find, comparisons, checksums, Hot Paths, Options and Jobs. Choose a
  category to narrow the list; each entry shows every key assigned to its command,
  as Options › Keyboard lists them (under the label when the row is too narrow for
  both), and unavailable operations remain visible but disabled.
- F1 shows the keyboard help and Alt+F4 exits. F9 opens the User menu: the command
  line, Hot Paths, Properties, folder sizes, Checksums and each configured Custom
  Command; without Custom Commands it offers **Set up…**, which opens Options →
  Custom Commands.
- Ctrl+Shift+S or Ctrl+Shift+F10: calculate selected folder sizes, or all folders when
  none are selected; press again to cancel. Partial/running values are marked; links
  inside the folders are not followed.
- Ctrl+[ / Ctrl+]: insert the left/right path at the caret or replace the selection.
  The physical bracket keys also work with Shift and with non-US layouts.
  Ctrl+P inserts the active path. Directory insertions end with a native path
  separator, without doubling an existing one.
- Ctrl+Enter: append the focused filename at the end of the command draft, and
  Ctrl+Shift+P its full native path. Insertion adds no quotes, escaping or spaces
  and never submits the command.
- Alt+F1 / Alt+F2: open the left / right panel's drive choices. Ctrl+F3 / F4 / F5 / F6
  sort by name / extension / modified time / size; repeat to reverse the order.
- Ctrl+F1 or Ctrl+L describes the volume that holds the active panel's folder (inside
  an archive, the volume of the archive file): its name, filesystem, capacity and
  storage type. See [drive information](DRIVE_INFORMATION.md).
- Enter in the command editor: open the system terminal in the active directory
  and run the submitted command. Insertion alone never executes a command.
- The full command directory ends with `>`. Its history dropdown restores the
  selected draft without running it. The latest 100 unique exact strings are
  persisted (up to 1MiB aggregate text); recalling never enables elevated mode.
- The terminal icon opens a new interactive terminal in the active folder and
  preserves the command draft. Its adjacent arrow chooses PowerShell/cmd on Windows
  or Terminal/Terminal (sudo) on Linux. A nonempty draft stays bound to its grammar;
  clear it before switching PowerShell/cmd. Sudo selection is session-only.
  Linux uses the executable shell in `$SHELL`, with Bash/sh fallbacks, and reads
  its normal interactive configuration. A missing/basic prompt gets a dynamic
  user/host/current-directory default without replacing custom shell prompts.
  Ctrl+click on the icon (or Ctrl+Enter / Ctrl+Space while it has the keyboard focus)
  opens the selected shell elevated: as administrator after Windows asks for
  consent, or as a root shell through sudo on Linux. Windows refuses, before asking,
  a folder on a network or `subst` drive of your own sign-in, which programs run as
  administrator do not see. This command has no default key and is in F10 → Tools.
- The compact terminal icon and navigation controls explain themselves through
  tooltips. Drive tooltips show free space and Windows volume names; expanded
  drive choices include those details. Quick buttons and their dropdown share
  discovered drives and special locations, with full-path tooltips. The location
  menu button shows a neutral icon instead of repeating the current drive letter;
  its tooltip names the current drive (for a special folder, the drive that holds
  it; for a detected native network path, Network). Its dropdown uses the available
  window height; named OneDrive accounts remain in the menu while the toolbar shows
  one primary OneDrive shortcut. Drives and actions use system icons where
  available and built-in icons otherwise.
- The top toolbar's Options gear groups settings into pages, listed in the order users
  look for them: General, Appearance, Colors, Previews, Archives, Editors, Comparators,
  Custom Commands, Keyboard and Plugins ([why this order](developer/DIALOG_ERGONOMICS.md#options-page-order)).
  General holds the filename search filter, hidden files, readable or exact-byte
  sizes, full SHA-256 or quick copy verification, the number of simultaneous disk
  operations (Automatic, up to 4, or 1–32) and command history persistence/Clear.
  Appearance offers 26 themes including Slate Blue, Slate Teal and Slate Violet,
  Desktop/Touch spacing, an installed font family and independent Desktop/Touch
  base sizes (8–32px). Appearance → **Text rendering**
  chooses how glyphs meet the pixel grid: Sharp (fitted, the default), Balanced
  (snapped) or Smooth (subpixel). It applies after a restart; the
  `MIDNIGHT_GLYPH_FITTING` environment variable (`fitted`, `snapped` or `subpixel`)
  overrides it for one run. **Scrolling glide** (Appearance) sets how long scrolling
  glides to its target after a wheel notch or a scroll key, and how long a touchpad
  flick keeps moving after your fingers lift, in the panels, lists, dialogs and every
  viewer: Off, Very short, Short, Default, Long or Very long, chosen with the mouse or
  with the arrow keys, Home, End, Page Up and Page Down. Off moves every scroll at once.
  The change applies immediately, also in open viewer windows. When Windows' Animation
  effects are off, scrolling never glides, whatever this setting says; a Windows
  precision touchpad's own coasting is controlled by Windows. A mouse wheel notch
  scrolls as far as it does in Chrome and in Electron applications: on Windows 100
  pixels at the default of three lines per notch (proportionally more or less for other
  settings, and seven eighths of the visible height when Windows scrolls one screen at a
  time), on Linux 120 pixels; at Default each notch glides for a fifth of a second, as in
  Chrome, and quick successive notches keep their speed. **Scrolling speed**
  (Appearance) scales that distance for a mouse wheel in the panels, lists, dialogs and
  every viewer: Very slow, Slow, Medium, Fast or Very fast, from half to twice the
  system's distance; Medium, the default, is the system's own speed. Touchpad gestures,
  keyboard scrolling and Windows' one-screen-at-a-time setting are not affected. The
  [palette provenance](THEME_PROVENANCE.md) describes the theme catalog. Preview
  is immediate and reversible. **OK** validates all pages, saves asynchronously
  and closes after success. **Cancel**, Escape and X discard all edits and restore
  previewed theme, spacing and fonts. Validation or save errors keep Options open.
  **Reset to defaults** resets all Options pages while preserving tabs, command
  history, hot paths and custom commands. It is a preview until OK; Cancel undoes
  the reset too. A line above the footer says what it did, also when every page
  already showed its defaults, until the next change. It waits while a page's file
  chooser, an export or an import is open.
  **Export configuration…** saves all settings, including tabs, hot paths and
  window placement, to a versioned JSON file. **Import configuration…** checks a
  whole file first and previews it in Options; OK saves it and then applies its
  tabs, hot paths and window placement, and Cancel keeps the current settings.
  A file from the other operating system keeps its hot paths, editors, custom
  commands and comparators inactive, exports them unchanged and skips its tabs.
  See [Export and import configuration](SETTINGS_FILES.md).
  See the [Options transaction design](superpowers/specs/2026-11-08-options-transaction.md).
  Colors offers a visible priority list of up to 32 wildcard/regex filename rules,
  starting with presets for common file kinds; the first matching enabled rule
  wins. Set a base text color and
  optional background; focused/selected variants are derived automatically and
  previewed together. Matching runs on backend workers in every panel view.
  Color swatches open a movable, resizable picker popup. Its controls scroll in
  small windows while Use color / Cancel stay visible; Escape cancels just the
  picker, and the outer Options OK / Cancel still controls saving. Its preset
  swatches name their color and hex value in a tooltip and show hover and keyboard
  focus.
  Touch spacing makes controls at least 40px high; Desktop is denser. The active
  folder appears beside the command input in the same single row. A folder path too
  long for its share of the row shows its end after its start and an ellipsis, and
  scrolls or drags horizontally, so the input keeps room for typing.
  The command row uses the same height as the function-button toolbar. Alt+Up
  and Alt+Down rotate stored commands without changing the chosen terminal;
  Enter submits the displayed draft.
  The window minimum stays 320×200 logical pixels at every font/density. Compact
  layout keeps the active panel and essential controls accessible; wide controls scroll.
- One shared toolbar above both panels operates on the active panel: Back and
  Forward with their history lists, Parent folder, Refresh, New empty file,
  Attributes, Find, Compare files, Compare directories, Checksums, Make File List,
  Folder sizes, Properties and Hot Paths. Drives, the filter, view and sort choices,
  tabs and paths remain per panel. Narrow windows scroll the shared toolbar.
  The Options gear and, at the right end, the moon button stay fixed; the moon
  opens About with the application version and copyright. Its night sky
  holds still while Windows animation effects are off. The bottom bar keeps F1–F12
  and Jobs.
  Switching panels or locations invalidates an open context-dependent popup.
- Custom Commands has a dedicated toolbar above the command editor whenever at
  least one command is configured. At compact window sizes, the shared toolbar's
  **Custom commands** button opens the same commands in a popup to preserve file
  browsing space. Icons may have optional text, separators group them, and
  overflow scrolls. A command's right-click menu offers **Custom Commands
  settings…**, which opens that page with the command selected.
  Drag rows in Options → Custom Commands to change their order. A marker shows
  the insertion point; the list scrolls at its edges. Escape cancels the drag.
  Order is staged without changing command IDs or shortcuts; Up/Down remain
  available. **Save** validates and stages an individual form so another command
  can be edited. Options **OK** persists staged changes; **Cancel** discards the
  session.
  Program mode passes native arguments separately; terminal mode runs literal
  user-authored PowerShell, POSIX shell or Command Prompt script with explicitly
  declared context variables.
  A Run directly command's **Output** says where its output goes: **Hidden** (no
  window, as before; commands created in earlier versions keep it), **Output window**
  (the default for new commands) or **Console window**. The output window shows the
  command line, folder and start time, the output as it arrives (errors marked) and
  the exit code and duration at the end, with **Stop**, **Copy all**, **Save as…**,
  **Clear**, **Find** (Ctrl+F) and **Close when finished successfully**. A per-item
  command runs one item after another in one window. The program's input is empty,
  so a program that asks a question gets no answer: use Console window for such
  programs. Closing the window while the program runs stops it and the processes it
  started. Console window opens a system console (Linux: your terminal emulator)
  that runs the program with the same arguments and stays open until you press a key.
  Preview does not execute. Jobs reports started/not-started processes; Cancel
  stops future launches without terminating applications already started.
  Assign `user-command.<id>` shortcuts in Options → Keyboard. The
  [Custom Commands contract](developer/USER_COMMANDS.md) records platform
  and workload limits. See the [settings verification](verification/2026-11-07-custom-command-settings.md)
  for Windows/Linux input, scrolling and persistence checks.
- Panel scrolling moves in pixels and draws the rows cut by the panel's edges, so
  it never leaves an empty band below the column header. A wheel notch glides
  the whole rows nearest to the text viewer's notch (which follows the system's
  lines per notch) and stops on a row; touchpad scrolling and its momentum glide
  like the text viewer's, after which the Details and Brief lists settle gently
  on the nearest row (the thumbnail grid keeps its position). With Windows
  animation effects turned off, wheel scrolling and the settling happen at once.
  Windows keeps its own touchpad momentum; on Linux, which reports
  none after the lift, a touchpad flick keeps
  scrolling panels, viewers and the comparator with momentum that slows down and
  stops at once on a click, key or new wheel or touchpad movement (see
  [kinetic scrolling](developer/SCROLLING.md#touchpad-momentum-kinetic-scrolling)).

- Alt+Enter or Ctrl+Alt+Enter: properties of selected items or the focused
  file/folder. Windows
  opens its Properties dialog; for several items, one dialog for all of them with
  their combined count, size and attributes, as in Explorer (items from different
  folders, such as search results, show "Various Folders"). Linux shows a read-only
  summary. For one item: its
  type, size, owner and group (names, or numeric ids for accounts that
  `/etc/passwd` and `/etc/group` do not list), permissions, created time (when the
  filesystem records it, otherwise "not available"), modified and accessed times; a
  folder's files, folders and total size are then counted in the background
  ("Calculating…" until they arrive; closing the summary stops the count; links in
  it are not followed and other filesystems are not entered). For a selection: the
  number of files, folders, links and other items with the total size of its files.
- `.` in the drive dropdown, or click **. As Other Panel**: open the opposite
  panel's current folder.
- Shift+drive letter: switch the focused file pane to that drive on Windows, at its
  remembered folder. Where no drive has the letter (always on Linux), the uppercase
  letter goes to the filename search instead.
- F2: rename the focused native file or folder. A regular-file name collision uses the shared
  overwrite prompt; replacement requires confirmation of that exact destination.
  Self, hardlink, directory and link replacement remain protected. While the rename
  runs and until the new name is listed (at most 3 seconds after it completes) the
  panel holds its input, then replays the keys typed meanwhile; a rename that takes
  longer than half a second says `Renaming “a” to “b”…` in the status line, and
  Escape releases the panel at once
  ([operation progress](developer/OPERATION_PROGRESS.md)). Creating a file or
  folder holds the panel the same way.
- F7: create an absolute or relative folder path,
  including `..` and missing intermediate folders. Paths may leave the active panel.
- Shift+F4 or the shared toolbar's **New empty file** button: enter a filename and press
  Enter to create a zero-byte file. Escape cancels. For the name of a file the panel
  lists, the dialog asks first: only its **Replace file** button (not another Enter)
  replaces that file with an empty one, and the original is kept in a recovery folder
  whose path the completion notice shows. Folders and links are never replaced. On
  Windows the empty file keeps the original's permissions and its read-only, hidden and
  system attributes, is compressed or encrypted when the original was, and replaces a
  compressed, encrypted or sparse file like any other; a cloud or offline file whose
  contents may not be on this computer is not replaced, and the notice says so. The
  new file is focused when creation finishes.
- Delete/F8: confirm moving selected items to Trash. Shift+Delete/Shift+F8:
  explicitly confirm permanent deletion. A Trash failure never permanently deletes.
  The confirmation asks its question first ("Permanently delete “report.pdf”?",
  "Move 12 items to the Trash?"), says right under it that a permanent deletion cannot
  be undone, and lists the details below: for one item its name, type, size (readable,
  with the exact bytes), modified date/time and folder; for several the files and
  folders counted, the first names, the files' total size and their folder. Folder
  contents are not recursively counted just to open the confirmation: a folder's size
  is shown only when Folder sizes has measured it.
- F11/F12: Windows system dialogs for connecting/disconnecting network drives.
  Their footer buttons are hidden when these dialogs are unavailable, including
  on Linux. Supported modifier actions such as Ctrl+F11 and Ctrl+F12 remain visible.
- **Jobs** at the right end of the bottom bar shows the operation queue and session
  history; **Open** displays the selected operation's progress.
- Hold Ctrl, Alt, Shift or Meta, or a combination of them, to show that layer's
  effective F1–F12 captions and actions in the footer; unassigned keys are disabled.
  Remapped or disabled shortcuts are reflected in this layer. Ctrl+F11 expands
  the active panel across the entire panels area, completely hiding the other
  panel and splitter. Tab switches the visible panel; another Ctrl+F11 restores
  the previous split. Outside zoom, double-clicking the splitter centers it.
  The normal panel width ratio is saved automatically after adjustment and restored
  on the next startup, including when the window size changes.
- Ctrl+F10 or **Compare directories** in the shared top toolbar compares the
  two displayed native directories on the backend as Open Salamander does, and
  selects the differences in both panels: files present in one panel only, and of
  same-named files the larger (**Size**) or newer (**Date and time**) one, or both
  when **Attributes** or **Content** differ. Size and time are on by default; time
  differences up to 2 seconds are ignored (the tolerance is the small field beside
  the time option), and differences of exactly one or two hours (daylight saving)
  can be ignored too. Matching metadata does **not** verify file contents: **Content**
  reads same-sized files byte by byte. **Compare subfolders** compares same-named
  folders with everything inside them and selects a folder that differs anywhere
  inside in both panels; every compared row then shows a badge at the end of its
  name: **=** matches, **≠** differs, **+** exists in this panel only, **?** could not
  be compared; expand a folder to see which subfolders and files differ. Folders in
  one panel only can be selected without comparing subfolders, and files or folders
  can be ignored by masks. While contents or subfolders are compared, the dialog shows
  "Comparing … with …", the counts and a progress bar; Cancel, Escape or Alt+C stops
  it and leaves the panels unchanged. The result is a line below the active panel.
  See [Compare directories](developer/COMPARE_DIRECTORIES.md).
- Ctrl+F or Alt+F7 **Find** searches names (masks, regular expressions or metadata
  expressions), sizes, dates and contents; the content query can be a regular
  expression matched per line. **Identical files** finds groups of files with the
  same contents. **Search inside archives** also matches archive
  members, in every archive however large, until the search ends or you press Stop
  (Go to file opens the archive at the member; archives inside archives are not
  opened), and **Feed to panel** shows
  the results as a list in the panel, where View, Edit, Copy, Move, Delete and
  marks act on the real files and Backspace returns to the folder. The result
  list selects several results (Ctrl+click, Shift+click, Shift with the cursor
  keys, Space, Insert, Ctrl+A, Ctrl+D); Ctrl+C and the list's context menu copy
  their full paths, names or row details, the status line counts them and their
  size, and Feed to panel brings them marked into the panel. See
  [Find](developer/FIND.md).
- Ctrl+F12 opens a saved per-panel file filter. Choose wildcard masks, a Rust
  regular expression, or the supported metadata expression language, with explicit
  case sensitivity. **Help** (F1) explains each mode with examples. Masks and regular
  expressions match names and keep folders visible; an expression tests metadata and
  can leave folders out too. The filter combines with typing-based filename search.
  An invalid filter does not replace the active one. Down or Alt+Down in the filter
  line lists the recently applied filters. See
  [metadata expressions](developer/METADATA_FILTERS.md).
- Ctrl+Shift+C or **Compare files** in the shared top toolbar compares two
  selected regular files in the active panel, or one focused/selected file with
  its same-name counterpart in the other panel. An explicit selection in the other
  panel takes priority. Automatic matching uses the exact native name among visible
  files; select both files explicitly to compare different names or letter case.
  When the pair is unclear, a small dialog asks for File A and File B (type a path
  or Browse).
  Text pairs show line differences (files up to 32 MiB and 1,000,000 lines of at
  most 16 KiB each), and **Find…** (Ctrl+F) searches both files; image pairs provide
  **Side by side**, **Swipe** and **Onion skin** controls. Binary files, larger text
  files and other pairs are compared byte by byte in a side-by-side hex view
  (offsets, hex bytes and characters, differing bytes marked) with a summary of the
  sizes, the first difference and the number of differing bytes and regions; the
  **Hex** button shows the bytes of a text pair too. F7, N, Alt+Down or Alt+Right
  and Shift+F7, P, Alt+Up or Alt+Left (or the difference buttons) move between
  differences, Alt+Home and Alt+End to the first and last one. A long byte
  comparison runs in the background and shows its progress; Escape closes the
  window and cancels it. **Options → Comparators** chooses how pairs are compared:
  ordered rules (wildcards or a regular expression that both names must match; the
  first enabled match wins, up to 64 rules) pick **Automatic**, **Built-in text**,
  **Built-in image** or an **External program**, which gets one literal argument per
  row with complete `{left}` and `{right}` rows for the two files; other pairs use
  Automatic. See [file comparison](developer/FILE_COMPARISON.md).
- F3 opens PDFs with the installed independent PDF viewer. Ctrl+PageUp/Down or
  the page buttons switch pages; Fit, zoom and pan are shared with image viewing.
  Each new PDF opens fitted to the window. Page changes retain manual zoom/100%;
  Fit adapts to each page's dimensions. **Paged** shows one page, **Continuous** all
  pages in one column; the layout chosen last is remembered for every PDF opened
  later, also after a restart. Pages are rendered at the exact resolution of the
  screen, in tiles kept for scrolling, so the wheel, the keyboard and the scrollbar
  move the pages smoothly and text stays sharp at every zoom; a zoom step shows the
  pages at the new size at once and sharpens them when their tiles arrive. Click the
  zoom percentage or press Ctrl+Y to type a zoom (`150`, `150%`, `62,5` in a
  decimal-comma locale); Enter applies it, Escape keeps the current zoom.
- F5 inside an archive copies the captured members to a native destination using
  the background queue and SHA-256 destination readback. As for files on disk, the
  progress window and Jobs show the path of the member being copied (the archive
  file's path, then the folders inside it and the member's name, such as
  `backup.zip\docs\readme.txt`), the number of files and bytes to copy, the share
  done and the time remaining. Archive contents stay read-only. XZ, Bzip2 and
  Zstandard streams and their TAR variants are supported by the archive plugin
  alongside ZIP, 7z, GZIP and TAR.
- Password-protected ZIP (ZipCrypto, WinZip AES) and 7z (AES-256) archives open with
  their password. It is asked once, when F3, Enter or F5 first needs it, and kept in
  memory only until the archive is closed: when no panel tab, viewer or copy uses the
  archive any more, or when the application exits. An archive inside an archive asks
  for its own password. Cancel or Escape closes the dialog without a message and
  leaves the archive locked; a copy then skips the protected members and names them
  in its results. After OK, the action that asked goes on with its own target (the
  member F3 was pressed on, the archive Enter opened, the selection F5 copies); if the
  panel has left the archive meanwhile, nothing opens and the status line says so. A
  padlock marks encrypted members, and the status line names a member's encryption or
  why it cannot be read ([details](ARCHIVE_FORMATS.md#password-protected-archives)).
- Ctrl+F2 or the shared toolbar's attributes button opens **Attributes, times and
  case**. Ctrl+F7 opens the same editor with case conversion enabled. Other edits
  start as Leave unchanged. **Review changes** shows the captured selection and the
  exact draft, and **Apply** queues it. Windows attributes, compression and
  timestamps and Linux permissions and timestamps follow reported capabilities.
  Linux permissions have an Owner/Group/Other read/write/execute grid,
  SUID/SGID/Sticky bits, and synchronized octal and symbolic text. Mixed bits
  remain unchanged until edited; three octal digits preserve special bits, while
  four explicitly replace them. Unicode case conversion supports whole names,
  basenames and extensions. **Include subfolders** processes children first
  without following links or crossing mount points. Each folder's
  names are captured before its entries change; a folder whose names do not fit in
  what one edit holds (100,000 names and 4 MiB of names not processed yet, shared
  with the folders above it) is reported and left unchanged with its contents. Name
  conflicts offer Skip, Skip all or Cancel; a name that stays is never a conflict,
  and existing entries are never overwritten. The header names a single selected
  item, or counts the items, and mentions taken names or unsupported items only
  when there are any. Each timestamp on the Times page has separate Date
  (year-month-day, or the system's numeric date order such as 24.11.2026),
  Time (24-hour, or the system's clock such as 02:30:00 PM) and UTC offset
  fields that show the item's current time in the native timezone (or the
  current time when the items differ), ticked or not. Only ticked rows are
  applied: a click into a row's field or on its calendar or clock button ticks
  the row, and so does any change of a value; moving there with Tab does not.
  Unticking keeps what the row shows for later. Type freely, press Up/Down to
  change the part under the cursor, or use the calendar and time pickers
  (Alt+Down from a field; on a 12-hour clock the time picker has an AM/PM
  column). The time picker's ▲/▼ step when pressed and repeat while held, the
  wheel steps the column under the pointer, and digits type the chosen column;
  values wrap within their column (23 → 00 keeps the date). The field follows at
  once: Enter or a click outside keeps the time, Escape restores the row as the
  picker found it. The calendar also turns months with the wheel, and in a window
  too small for it, it scrolls. A date or time keeps the form it was typed in.
  The offset is used exactly as shown, and the review says when it is not local
  time on that date (for example a summer date moved into winter); a ticked time
  left as it started keeps its exact value. Now
  fills a row with the current local date, time and offset and ticks it; Apply
  sets exactly that moment. A change that completes without problems closes
  without a summary; only failures, skips or cancellation report.
- Click anywhere in the blank area after a panel's path breadcrumb, up to the end of
  the path bar, or press Ctrl+G, to type or paste a path; a pasted path stays exactly
  as copied. Press Enter or click **Go** at the right end of the editor to open it;
  Escape cancels. Breadcrumb navigation/copy gestures remain available.
  The path may name a file, also pasted in double quotes as Windows "Copy as path"
  writes it: Enter opens the file's folder with the cursor on the file (an archive is
  not opened), and the panel's status line says why when the file is hidden or the
  panel filter leaves it out.
- Panel view picker: independent Brief, Detailed and large-icon Thumbnails modes
  (Alt+2, Alt+3 and Alt+5 in the active panel).
  Brief fills columns downward; Left/Right changes columns. Thumbnails uses
  Left/Right between cells and Up/Down between grid rows. Page Up/Down moves a
  visible page. Active prefix search retains its existing match navigation.
  Installed thumbnailer plugins provide cached previews for supported image/video
  formats (also H.264, HEVC and the other formats the system decodes: Media Foundation
  on Windows, the distribution's FFmpeg or GStreamer on Linux) and the
  first page of PDFs (password-protected or damaged PDFs keep the
  placeholder); see the dated
  [thumbnail design](superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).
  **Options → Previews → Show image and video previews** turns them on or off (on by
  default). **Allow previews inside archives** is enabled by default,
  including for older saved settings. Turning it off hides archive-member previews
  and removes them from background scheduling. Network archives also require
  **Allow previews on network locations**. Options OK saves these choices;
  Cancel restores their previous values. Previews read local disks, including a
  virtual or cloud machine's disk and USB disks, without that permission; network
  folders and folders whose storage cannot be shown to be local need it, and the
  Thumbnails view then says so in the panel's status line, with the option to turn
  on. Problems of the preview service (a cache that cannot be written, a thumbnailer
  that cannot start) are shown in that line too, never in a window that takes the
  keyboard. A thumbnail that failed for a passing reason is tried again after 2 and
  8 seconds; a file no thumbnailer can read keeps its type icon.
  **Options → Previews → Thumbnail size** offers 48, 64, 72, 96 (default), 128,
  192 and 256 logical pixels: the edge of folder and file-type icons in a tile.
  An image or video preview fills its tile above the name instead, as wide as the
  tile less a small margin and 6 pixels taller than the thumbnail size, keeping its
  aspect ratio, and is prepared sharp for the display's scale. Larger sizes retain
  the current preview while a sharper image is prepared for that file; smaller
  sizes reuse larger cached images. The setting previews immediately, Cancel restores it and OK saves it.
  A selected or hovered video plays a short cycle of six moments from 10% to 85% of
  its length, in order and each for about 0.8 seconds, starting from its still picture;
  moments that are still being prepared join the cycle in their turn.
  The persistent cache (`%LOCALAPPDATA%\Midnight Explorer\cache\thumbnails\v1` on
  Windows, `$XDG_CACHE_HOME/midnight-explorer/thumbnails/v1` or
  `~/.cache/midnight-explorer/thumbnails/v1` on Linux) holds at most 512 MiB and
  8,192 files, and the backend keeps a 4 MiB memory cache; **Clear thumbnail cache**
  empties the cache in the background. Preview decoders and the cache inventory are
  prepared only when previews are first shown.
- Click column headers: change sorting. Cursor and selection remain attached to
  the original raw filesystem paths when the listing order changes.
  Repeated clicks toggle ascending/descending order. Computed folder sizes are
  kept and sorted on the backend; an explicit Refresh discards them.

Application hotkeys can be changed, disabled, or restored in **Options → Keyboard**.
An occupied shortcut offers **Replace** or **Cancel**, identifying the affected
commands. Replacement preserves their other aliases and nonoverlapping contexts;
Options **OK** saves the change, and Options **Cancel** rolls it back. Native text
editing and IME behavior are protected. The [shortcut reference](KEYBOARD_SHORTCUTS.md)
lists defaults. Numbered tab and Hot Path bindings use physical digits across layouts.

Options, tabs, Hot Paths, panel filters, enabled command history, viewer preferences
and shortcut overrides are stored as versioned UTF-8 JSON (supported schemas through 36) in the native
`%LOCALAPPDATA%\Midnight Explorer\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/midnight-explorer/preferences.json` /
`~/.config/midnight-explorer/preferences.json` on Linux. When the new file is absent,
the former `FM` (Windows) / `fm` (Linux) location is read safely; saves use only the
new location and preserve the old file. If settings take longer than 10 seconds to
load, the window opens with defaults and the status line says so; the saved settings
apply when they arrive, except the window's position and size for that session.
Viewer preferences include wrapping, line numbers, search case sensitivity, fonts,
the Markdown presentation, the image zoom mode and the PDF layout; see
[viewer settings](developer/VIEWER_PREFERENCES.md).
Source mode exposes line numbers and the current logical line/column. Markdown
offers GitHub (default), Compact and Comfortable layouts, plus font and size
controls. Exact distant source positions index in the background without delaying
scrolling. See [viewer presentation](developer/VIEWER_PRESENTATION.md).
Storage and font/icon discovery run on backend workers (dated
[navigation and appearance design](superpowers/specs/2026-11-04-navigation-and-appearance-design.md)).
Exact byte sizes and date/time use system regional settings captured on the backend;
restart after changing the OS locale. Missing native formats use invariant/UTC
fallbacks. Name and extension sorting uses native system-locale case and accent
weights; equal size/date values use that same name order. The locale is captured
until restart. Invalid native names remain lossless and use deterministic raw-name
ordering after collatable names. Linux falls back to the C locale when the configured
locale is unavailable. Cached backend sort keys have a 4 MiB resource budget per key;
names are never truncated (dated [locale sorting design](superpowers/specs/2026-11-06-locale-sorting-design.md)).

Platform limits of file operations: on Linux, file operations, Copy/Move and Find
use `openat2` and need kernel 5.6 or later, and Trash uses the home Trash only for
items on the same filesystem (elsewhere it is refused, never replaced by permanent
deletion). The Windows Shell's Recycle Bin can refuse long paths that native Rename,
F7 and permanent deletion handle. On Windows, removable FAT32/exFAT media usually
have no Recycle Bin and have no stable file identities, so Copy and Move refuse them
with an explicit error. Permanent deletion there uses the classic delete disposition
and refuses read-only entries; that is not yet verified on real FAT media (see the
[Windows mutation notes](../crates/midnight-platform-windows/MUTATION_SAFETY.md#fat32-and-exfat)).
The dated [operation design](superpowers/specs/2026-11-05-rename-delete-scroll-design.md)
describes confirmations and cancellation. The
[Network Neighborhood plugin](developer/NETWORK_FILESYSTEM.md) discovers
computers and shares through separate Windows and Linux adapters; macOS support is
planned.
The [pixel-scrolling contract](developer/SCROLLING.md) and the
[native scrolling measurements](verification/2026-11-11-native-wheel-scrolling.md) cover list and
viewer input, renderer optimizations and the measured performance limits. Selection
and resize performance is measured by the ignored `interaction_performance` test
(`midnight-ui` integration tests), the ignored `interaction_selection_cpu` test
(`midnight-engine` library) and the `native_resize` example of `midnight-ui`; the
dated [interaction and layout record](verification/panel-interaction-and-layout.md)
holds earlier results under the former `fm-ui`/`fm-engine` package names.

A small badge on a file/folder icon identifies compression reported by its
filesystem. File sizes remain logical sizes. An archive extension alone does not
set the badge; unavailable compression metadata remains unknown.

Directory links and junctions have distinct link artwork. Windows uses the
system link overlay; other platforms prefer host-theme icons and fall back to
application artwork. Recursive folder-size scans count links inside the measured
folder as skipped and never follow them, so aliases such as profile junctions are
not counted twice. A scanned row that is itself a directory link (a selected row,
or any row of a full-pane scan without a selection) is measured through its
target. Browsing a folder does not inspect a link target merely to choose its
icon.

Paths and file identities are kept separately from their displayed text. A failed
navigation preserves the last usable directory and reports the failed operation.
Each pane remembers its last visited directory for each volume during this run.
Visible directories are watched on the backend. Changes are batched after
150 ms of quiet, with a 500 ms cap during continuous activity, and a flat folder
that keeps changing is listed again at most once a second (less often when its
listing takes longer). A 15-second check
re-lists each watched directory (in tree mode every eligible loaded branch) to
catch missing native/network notifications. It compares the same enumeration
metadata the listing shows and reads metadata separately only for entries whose
enumeration lacks it; an entry whose metadata cannot be read stays unknown, as in
the listing, instead of causing a refresh on every check. A flat panel's directory
stays watched while it is refreshed or re-sorted and while the panel lists another
native folder, and stays watched for the rows still shown when that refresh or
navigation fails. Notifications that arrive after its new listing started may
describe changes that listing missed, so the directory is checked against the new
listing at once when it is shown. A folder the panel navigates to is subscribed
once its listing has read its first entries and is checked the same way when it is
shown; a change to entries already read before the subscription took effect waits
for the 15-second check. A refresh during which no notification arrives is not
followed by another listing, and re-sorting rows that a check found current does
not check them again. On Linux all watches share one inotify instance. If native
notifications cannot be registered for a panel's folder (for example when the
system's watch limit is reached), the folder is checked about every second (less
often for large folders) while the registration is retried after 1, 2, 4 and 8
seconds and then every 15 seconds; a retry that succeeds switches back to
notifications silently. Only when registrations keep failing for a minute does a
line above the panel's details bar say that live updates are unavailable and the
folder is checked regularly, with the reason in its tooltip. Expanded branches
report a failed registration at once and retry with a later refresh, expansion or
collapse, at most once per check interval. If a panel's watcher fails, it restarts
at once, and afterwards at most once a minute; until then that line says that
automatic refresh is paused. Neither is a panel error or a dialog. In tree mode, a
changed set of watched branches registers its native watches before the old ones
are released, so directories watched before and after stay covered; a directory
that becomes natively watched is checked at once, and a collapse alone requests
no refresh. Neither does expanding an unchanged directory: the expansion's listing
records an observation of the whole directory, files included, and the branch's
first check compares with it. A branch listed before its tree was watched,
re-listed by a refresh before its first check, or whose listing kept no
observation (one with more than 4,096 files whose metadata the enumeration lacks)
still requests one refresh of the loaded branches.
Automatic refresh (change notifications and finished file operations) preserves the
viewport, selection, command editing, an open Panel Filter dialog and the computed
sizes of folders that are still listed; folders it adds show as not calculated.
Explicit Refresh discards computed sizes unless a size scan continues across it. A
flat panel's automatic re-read waits while a drag from that panel or a directory
comparison is in progress, and a change notification also waits while a folder the
user expanded is opening its tree, then refreshes that tree; maintenance of
expanded branches waits for a running comparison. When the focused item
disappears, the cursor moves to its nearest remaining neighbor.
Folder sizing uses a bounded fair batch and a shared I/O budget: up to the
[Disk I/O concurrency](DISK_OPERATIONS.md) limit of Options → General
(Automatic: four) of parallel reads only for proven local SSD,
one stream for HDD/network/unknown storage and local disks of unknown media type
(a virtual machine's or USB disk). Detection errors remain conservative;
storage is classified once per device on each scan lane. File lengths come from
complete directory enumeration data without opening each file. Other navigation
work has independent capacity.
An active size scan continues across sorting and same-directory refresh; moving
the cursor, changing selection or working in the other pane does not cancel it.
A running total shows `≈` in a contrasting color, a folder still waiting `…`, and
one that a stopped calculation never reached `—`. A finished total is exact only
when every entry could be read: a total that met errors keeps `≈` beside a warning
icon that opens the details, and a cancelled or unfinished total keeps `≈` too.
Skipped links and special files do not make a total partial.
A file row has a tooltip only when its name is shortened (the whole name); a native
folder link's row shows its target, read when the pointer rests on it. Size-only
UI updates are combined for both panes and shown
at most four times per second; input and navigation remain immediate.
Progress and scan completion update values without reordering rows; explicitly
sort again to apply newly completed sizes to the order.
Quick drive/location buttons sit beside the drive dropdown.

## Updates

The toolbar's update button, between the actions and Options, shows whether a newer
version exists: its tooltip says "Check for updates", "Checking for updates…",
"Version X is available" (with an accent dot), the download progress, "Restart to
update to version X" (with an accent dot) or why the last step failed (with a red dot).
Click it, or **Check for updates** in About, to open the update dialog:

- **Check now** asks GitHub for the newest release.
- **Download and install** downloads the offered version in the background, checks
  its signature and checksums, and prepares it; nothing changes yet, and you can keep
  working or close the dialog with **Later**.
- **Restart now** closes Midnight Explorer the usual way (running jobs are offered
  first), installs the new version into the same folder and starts it again with the
  same panels and tabs. If the new version does not start correctly, the previous
  version is restored and started, and the dialog says why.
- **Skip this version** stops the startup check from mentioning that version; Check
  now still shows it.
- **Open release page** shows the release on GitHub in your browser, with its full
  notes and downloads.

Options › General › Updates has two choices, both on by default:

- **Check for updates at startup:** about 5 seconds after the window opens, at most
  once a day, ask GitHub whether a newer version exists. It only shows the button's
  dot; nothing is downloaded or installed until you choose to.
- **Include preview releases:** offer releases that GitHub marks as previews. Every
  release so far is a preview, so turning this off waits for the first stable release.

**What is sent:** the check is an HTTPS request to `api.github.com`; downloads come
from `github.com` and GitHub's download servers (`objects.githubusercontent.com`,
`release-assets.githubusercontent.com`). The only header that identifies anything is
`User-Agent: Midnight-Explorer/<version>`; nothing about you, your files or your
settings is sent, but GitHub sees your IP address, as for any visit. With the startup
check off, Midnight Explorer contacts no server unless you open the update dialog and
check, download or open the release page.

**Which copies update themselves:** an extracted release folder that your account can
change. A copy installed by an administrator (for example below `C:\Program Files` or
`/opt`), a development build and a copy from another package manager show the reason
and offer the release page instead; Midnight Explorer never asks for administrator
rights to update. Builds made without the project's release key install no updates.
The details, including the signature checks, are in
[Updates](developer/UPDATES.md).

## Build output and version control

Build products and diagnostic output share the ignored `_build/` root. Cargo's
`target-dir` configuration applies to build, test, Clippy, generated Slint Rust,
dependency outputs, incremental caches and rustdoc output:

| Directory | Contents | Git |
| --- | --- | --- |
| `_build/` | Cargo output, including the debug and release executables, and the module, validation and acceptance build folders | Ignored |
| `_build/linux/` | Linux output when a Linux or WSL build shares the checkout with Windows (by setting `CARGO_TARGET_DIR`) | Ignored |
| `_build/tmp/` | Temporary compiler and test files of the build, Cargo and validation helpers | Ignored |
| `_build/qa/` | Test logs, screenshots, diagnostics and disposable experiments | Ignored |
| `.tools/` | Optional toolchains and download caches: a local Rust toolchain, CEF, FFmpeg, the Docker build caches | Ignored |
| `crates/`, `api/`, the other `*-api/` workspaces, `plugins/`, `scripts/`, `packaging/`, `assets/`, `vendor/`, `.cargo/`, `.github/`, root build helpers | Handwritten code, independent plugin and API projects, tests, packaging, assets, patched dependencies, build configuration and CI | Tracked |
| `docs/` | Design, user and developer documentation, dated verification records and documentation images | Tracked |

Cargo writes directly into `_build/`; there is no `target/` directory. Windows
executables are in `_build/debug/` or `_build/release/`; so are Linux executables in
a checkout of their own.

Explicit `CARGO_TARGET_DIR`/`--target-dir` overrides remain supported. Direct Cargo
commands use `_build/` but may use the OS temporary directory; the helpers set
`TEMP`/`TMP` on Windows or `TMPDIR` on Linux to `_build/tmp/`. Global rustup and Cargo
download caches and the application's settings are not build products and keep their
normal locations. Root `build/`, `dist/` and `target/`, compiled binary, object and
library files and logs are ignored as an extra safeguard. `Cargo.lock` is committed
for reproducible dependency resolution.

## Structure

The application workspace (`crates/`):

| Crate | Responsibility |
| --- | --- |
| `midnight-core` | Domain data, commands and events, provider and service contracts, shortcuts, settings formats and sorting |
| `midnight-platform` | Target-selected facade; the implementations live in separate OS crates |
| `midnight-platform-common` | Portable filesystem, transfer, settings and metadata helpers shared by the native adapters |
| `midnight-platform-windows` | Windows-only service implementations and Win32 integration |
| `midnight-platform-linux` | Linux-only service implementations, mount discovery and desktop integration |
| `midnight-engine` | Tokio orchestration, cancellation, bounded workers and batches |
| `midnight-explorer` | The `midnight-explorer` executable: runtime, bounded channels, coalesced UI wakeup, startup and lifecycle |
| `midnight-ui` | Slint presentation: presenters, the virtualized lazy row model and dialogs |
| `midnight-ui-bindings` | Generated Slint types, compiled separately from the handwritten presentation |
| `midnight-ui-input` | Shared keyboard, placement and wheel input policy |
| `midnight-elevation` | The one-shot elevated directory reader and, on Linux, the HTML viewer's AppArmor profile installation |
| `midnight-native-filesystem-host` | Discovery and loading of the independent native and network filesystem packages, and the host side of their browse and read interface |
| `midnight-filesystem-host` | Discovery, validation and supervision of filesystem worker processes, such as the archive worker, and their sessions |
| `midnight-worker-platform` | Process boundary shared by the media and archive worker programs |
| `midnight-work-budget` | Process-wide fair I/O admission |
| `midnight-plugin-host` | Discovery and loading of trusted native viewer plugins and runtime resolution |
| `midnight-viewer-engine` | Viewer sessions; reading, search and file cycling on the backend |
| `midnight-viewer-render` | Viewer text layout and rasterization for backend workers |
| `midnight-viewer-ui` | Viewer windows: thin UI adapters |
| `midnight-image-viewer-host` | Supervision of the restricted image and PDF viewer processes |
| `midnight-web-viewer-host` | Supervision and authority for the optional HTML (Chromium) viewer |
| `midnight-office-preview` | Bounded previews of Office documents' contents |
| `midnight-media-host` | Video playback, trusted media installation and restricted decoder transport |
| `midnight-media-output` | Native audio output for playback |
| `midnight-thumbnail-host` | Scheduling, broker and persistent cache of the thumbnail workers |
| `midnight-compare-engine` | Bounded read-only text, image and byte comparison on workers |
| `midnight-compare-ui` | File comparison windows |
| `midnight-acceptance` | Opt-in end-to-end acceptance suite |

The public APIs and SDKs (`api/`, `filesystem-api/`, `image-viewer-api/`,
`thumbnail-api/`, `web-viewer-api/`, `worker-api/`) and the plugins in `plugins/` are
separate Cargo workspaces.

No filesystem operations or engine implementation are reachable from `midnight-ui`.
Full-list sorting and identity-index construction run off the UI thread. UI
delivery is batched with a short work budget and one outstanding wakeup.

The `midnight-platform` facade only selects the target adapter. `DirectoryService`,
`VolumeService` and `StorageService` are defined in core; their `FileSystem`
aggregate is the engine contract. Each platform implements these interfaces in its
own crate, and the facade's tests run against the selected adapter on each platform.
Windows and Linux implementation bodies and OS-specific tests do not share source
files. Optional `DesktopServices` aggregates `IconService`, `TerminalService`,
`PropertyService`, `LocationService` and `OpenService`; native workers implement these
separately from filesystem navigation. Desktop work and size scans have independent
budgets.

## Checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --features midnight-acceptance/acceptance -- -D warnings
cargo test --workspace --locked
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo run -p midnight-explorer --locked -- --smoke-test
cargo run -p midnight-explorer --locked -- --ui-smoke-test --software
```

These are the main workspace's checks, with CI's formatting and Clippy commands (the
`acceptance` feature makes Clippy check the acceptance suite too). CI runs the
workspace tests through `scripts/ci-workspace-tests.py`, which runs the same test
targets in several Cargo invocations; locally, run it as
`python -B scripts/ci-workspace-tests.py --keep-test-artifacts` (its CI option, which
deletes passing test executables, works only with `CI=true`). Both smoke tests need
the native filesystem package beside the executable (a helper build installs it), and
the UI smoke test needs a display. [Full validation](#full-validation) runs every test of CI for the
platform with one command, but not the formatting, Clippy and source size checks.
During implementation, build the affected test targets once and use the
[targeted parallel runner](developer/TEST_EXECUTION.md) for independent
headless suites. It shares a CPU thread budget across up to four processes,
combines the filters for the same test executable and keeps separate logs. Keep
desktop/input and performance checks serial; run full gates after the
implementation batch is coherent.

`--snapshot FILE.png` runs the UI smoke check and saves a screenshot of the native
window after both directories have finished loading. UI smoke checks use fresh
in-memory preferences and never load or save the user's settings. An application
error dialog fails the check instead of producing a passing screenshot. The image is
encoded and written by a backend worker, which never overwrites an existing file.
Linux and Windows use the same source; CI's build and test commands are in
`.github/workflows/ci.yml`.

The [code style](../CODE_STYLE.md) follows the public Rust Style Guide and Rust API
Guidelines: rustfmt at 100 columns, UTF-8/LF, and a 100,000-byte maximum for
handwritten source files. Run `pwsh scripts/check-source-size.ps1` on Windows or
`bash scripts/check-source-size.sh` on Linux; CI runs both.

## Design and scope

The [main architecture](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md)
is extended by later dated designs, such as
[mouse activation](superpowers/specs/2026-11-04-mouse-activation-design.md),
[compact controls and volume tooltips](superpowers/specs/2026-11-04-compact-tooltips-design.md),
[the viewer and virtual filesystems](superpowers/specs/2026-11-04-viewer-vfs-design.md)
and [the dynamic plugin interface](superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md).
Later accepted requirements supersede earlier designs; the
[documentation map](README.md) lists the current contracts.
Salamander sources supplied separately are behavioral reference only; this
implementation does not copy its source, assets or resource strings. The About
dialog shows the copyright line, the license and Slint attribution: the Made with
Slint pill at the right end of the tab row under its sky (in a narrow dialog, at the
top right of the About tab). Midnight
Explorer is licensed under the GNU General Public License, version 3 or later
([LICENSE](../LICENSE)), as are its plugins. A macOS port is planned
(PLAT-01 in the [backlog](IMPLEMENTATION_BACKLOG.md)): the application has no
macOS platform services yet and does not build there; only the independent native
filesystem plugin has macOS source.
The [asset provenance record](ASSET_PROVENANCE.md) distinguishes project-created
artwork from icons supplied at runtime by Windows or installed desktop themes.

[Checksums on Ctrl+F8](developer/CHECKSUMS.md) use a separate modeless window
with SHA-256 by default, optional SHA-512, SHA-1, MD5 and CRC32, background
calculation, per-algorithm copy, safe manifest export and a Verify page for SFV, MD5,
SHA-1, SHA-256 and SHA-512 files. A run has no limit on files, folder depth
or manifest size; its memory follows its rows (about 200 bytes per SHA-256 row). The
results select like other lists (click, Ctrl+click, Shift+click, the cursor keys with
Shift or Ctrl, Space, Insert, Ctrl+A, Ctrl+D); Copy, Ctrl+C, Ctrl+Shift+C and the
context menu take the selected rows, while Save manifest… writes every calculated row.
While a calculation runs and nothing is selected, the list keeps the file being
calculated in view.

Viewers, filesystems and thumbnailers are separately loadable plugin packages under
the executable-adjacent `viewers/`, `filesystems/` and `thumbnailers/` folders. The
three text/source/Markdown viewer packages and the native and Network Neighborhood
filesystem libraries use a versioned C ABI and run as trusted native code. The viewer
plugin API/SDK (`api/`) is version 0.2.0 with ABI 1.0; plugins built on SDK 0.1.0 load
unchanged. The image
and PDF viewers, the thumbnailers, video playback and the archive worker are separate
executables with versioned protocols, run as restricted processes for untrusted
parsing; the HTML viewer runs Chromium in its own process tree with Chromium's
sandbox enabled. Slint rendering and
Tokio orchestration stay in the host. Missing packages remove the corresponding
features. Native panel browsing, metadata, capacity/storage queries and native F3
content use the separate Windows/Linux filesystem libraries. Other native operations
and watches still use host platform services; moving them to the generic provider
interface is in progress.
Plugins share this Git repository but have independent Cargo workspaces under
`plugins/`, with their own builds, dependencies and tests. CI checks the basic,
source, Markdown, image and PDF viewers, the image thumbnailer, the video
thumbnailer's platform-decoder helpers, the viewer ABI conformance fixture and the
native, network and archive filesystems from exported plugin directories, plus all
public API workspaces. The HTML viewer and the video thumbnailer's bundled-FFmpeg
build have no CI job yet (their pinned CEF and FFmpeg inputs need a cached CI setup);
the normal build helpers build them and [full validation](#full-validation) runs
their tests when those inputs exist. Plugins share only the
public API/SDK and must build from an exported plugin directory without host
sources. Plugins cannot depend on private
application crates or modify the main application's sources; API extensions are
separate host-owned changes. Distribution packaging consumes plugin artifacts.

Archives are browsed through installed archive workers: ZIP, 7z, GZIP, XZ, Bzip2,
Zstandard, TAR, and TAR compressed with GZIP, XZ, Bzip2 or Zstandard.
Enter/double-click opens an archive, directories and `..` navigate it, saved tabs
retain typed provider addresses, and F3 reads member content through the broker.
Members have opaque identities; duplicate names remain distinct (ZIP records that
the reader indexes under one name are listed once, unreadable). Missing providers
or changed sources retain unavailable saved tabs with a diagnostic. Archives are
read-only: nothing is written, moved or edited inside them, F4 does not open their
members, and RAR is not supported. F5 copies
members out through the verified background copy queue.
Enter on an archive member that is itself an archive (a ZIP or tar.gz inside a ZIP,
or any supported format inside another) opens it in the same panel, up to 4 archive
levels deep: the path bar shows the whole path (for example
`outer.zip/folder/inner.zip/sub`), Backspace and `..` go up through the levels with
the inner archive under the cursor, and F3, F5 and history work at every level. The
member is staged and verified through
its enclosing archive (in memory when small, otherwise in a private spool file), with
progress in the status bar and cancellation by navigating away; such archives are
opened for the session only, so tabs and Hot Paths keep the outermost archive's folder.
Details and limits are in [archives inside archives](ARCHIVE_FORMATS.md#archives-inside-archives).
At most two archives opened from native files are open at a time; tabs and viewers
that show the same archive share it. A tab that moves on to another archive hands its
own archive over; while two archives are still in use elsewhere, opening a third asks
"Two archives are already open" and offers to close one of them (the tabs that show it
go to its folder and its viewers close), then opens the archive
([details](ARCHIVE_FORMATS.md#current-operation-limits)).
Decoded content has no artificial size ceiling; the free space of the private spool
volume bounds it, and staging stops before that volume falls below a free-space
reserve of max(1 GiB, 2% of its capacity). Full
limits and supported codecs are in the [archive API reference](developer/FILESYSTEM_API.md).
[Archive associations](ARCHIVE_FORMATS.md) include Python wheels and other ZIP
packages. Options → Archives keeps document/media/application containers unchecked
by default; each extension can be enabled explicitly. The entrance-arrow icon marks
enabled native archives only when their provider is installed.
Each OS has a separate native provider implementation. A copy or move name conflict
shows the source and the existing destination and needs an explicit choice before
anything is replaced. Planned work (see the [backlog](IMPLEMENTATION_BACKLOG.md)):
the rest of the generic provider migration, tests on live SMB and FUSE mounts, and
reusable sudo terminal sessions ([design](superpowers/specs/2026-11-04-terminal-sessions-design.md)).

### Internal viewers

F3 opens the installed viewer for the file's type:

- the image viewer for PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, SVG and AVIF;
- the PDF viewer for PDF documents;
- the Markdown viewer for `.md`, `.markdown`, `.mdown` and `.mkd`;
- the source viewer, with highlighting for its
  [supported language catalogue](../plugins/midnight-viewer-source/README.md) (including
  `.csv` and `.tsv`, coloured column by column);
- the HTML viewer for `.html`, `.htm` and `.xhtml` (below);
- the Office preview for the readable contents (text, tables and sheets) of DOCX,
  XLSX, PPTX, ODT, ODS and ODP files, shown through the HTML viewer, which it
  requires; it runs no macros and evaluates no formulas;
- the video viewer, which plays WebM, Matroska, MP4/MOV, AVI, Ogg, WMV/ASF and 3GP
  files with seeking (see the [video worker guide](../plugins/midnight-thumbnail-video/README.md)
  for codecs);
- and the basic text/hex viewer for other files.

Alt+F3 always requests the basic plugin's Text mode,
including for binary input. The mode selector changes between modes supported by
the current plugin. Unknown or budget-limited syntax stays readable as plain text.
[Viewer file types](VIEWER_ASSOCIATIONS.md) can be changed in Options → Plugins.

Each viewer starts at the main window's size. Escape closes it, including from its
search field. Space or Alt+Right in the content area advances through the captured
panel order, wrapping after the last regular file; Backspace or Alt+Left goes back.
In very large folders
(an order beyond 8 MiB of paths, roughly 25,000–65,000 files) navigation cycles
within a contiguous window around the opened file, centered unless it reaches the
start or end of the folder. Space and Backspace load the file they open as it is
at that moment, so a file changed after the viewer started shows its current
content. A single-file view, such as a Find result, reopens the same captured
file instead, so a change to that file is not loaded.
Ctrl+F opens search; F3/Shift+F3 find the next/previous match. Scrollbars, the
wheel, arrows, Page Up/Down and Ctrl+Home/End navigate without loading the
complete file. Drag selects text and Ctrl+C copies the selection. In a text, source or
Markdown viewer, Ctrl+A selects the whole file, and the status line shows "Whole file
selected" with the file's size; Ctrl+C then copies the file's text as the viewer shows
it, read in the background with progress in the status line. Escape stops a long copy,
and nothing is copied unless the copy finishes. Copy takes up to 64 MiB of text; for a
larger file, copy the file itself with F5 instead. In the hex view, Ctrl+A selects the
rows shown, and in an HTML page it selects all in the page. These shortcuts are
configurable in Options → Keyboard.
Text and source modes support word wrap using the Wrap control or Ctrl+W. Binary
files open with byte offsets, hexadecimal bytes and a printable character column;
their columns remain fixed when wrapping is enabled for text.

Images use the same `image` and `moxcms` libraries as image thumbnails, with original
pixels decoded once in a restricted worker. Use Fit, 1:1, +/−, Ctrl+mouse wheel,
Ctrl+Plus/Equal, Ctrl+Minus, Ctrl+0 and Ctrl+1 to zoom, or Ctrl+Y to type a zoom
percentage. Drag to pan. Animated images show their first frame. This viewer does not
require or enlarge a thumbnail.
A damaged or unsupported image or PDF names the decoder's reason in the status, for
example "This image is damaged or incomplete: unexpected end of file".

The [HTML viewer](developer/WEB_VIEWER_API.md) renders HTML with
embedded Chromium inside the normal viewer. Its runtime is a separate plugin
package included by the normal build helpers; building `midnight-explorer.exe` alone does not
install Chromium. Build/package steps are in the
[independent provider README](../plugins/midnight-viewer-html/README.md).
Ctrl+F searches and Escape closes the viewer. It works as a small browser: the row above
the page has **Back** and **Forward**, enabled while there is a page to go to, and
**Reload**, which reads the file again from disk (F5 or Ctrl+R; Alt+Left and Alt+Right go
back and forward too while there is a page in that direction, and otherwise still cycle
files). The status line shows the page's title, or "Loading…" while it loads, and the
pointer changes over links (a hand), text (an I-beam) and anything the page styles.

HTML starts offline with its JavaScript running, so local pages, saved pages and
generated documentation behave as in a browser; offline, a page's scripts reach no
network and cannot read other files. Untick **JavaScript** to turn scripts off for the
page; the next pages of that viewer window keep that choice. **Online** allows network
for that document only and removes access to its local files; online, scripts can read
and transmit the document. A page shows with its own stylesheets, fonts and images,
also from a page saved "complete" (its `_files` folder) and from folders above it, as
generated documentation (rustdoc, MkDocs, Sphinx, Javadoc) keeps them, up to 16 levels.
It has been tested with real Chromium on Windows and, on Linux, on Ubuntu
24.04 under WSL (XWayland) ([Linux record](verification/2026-11-10-html-linux.md));
a test on a native Linux desktop is pending (PLAT-02 in the
[backlog](IMPLEMENTATION_BACKLOG.md)).
On Ubuntu 23.10 and later (24.04, Mint 22), AppArmor denies Chromium's sandbox the
user namespaces it needs. The first HTML document that this stops shows why in the
page's place, with an **Install…** button that installs the viewer's AppArmor profile
after administrator authorization (polkit) and opens the document again; nothing is
installed before that. Alternatively an administrator can install the package
system-wide and run its root-owned copy of `install-html-viewer-apparmor.sh` with
sudo (see [packaging/linux/README.txt](../packaging/linux/README.txt) and the
system-wide installation under [Linux desktop launcher](#linux-desktop-launcher));
never run a copy that your account can change with sudo. Without a trusted
`/usr/bin/pkexec` the viewer only explains this, naming the command only for a
root-owned installation; Chromium never runs without its sandbox. See
[system requirements](SYSTEM_REQUIREMENTS.md).

Links in Markdown and HTML pages and in Office previews follow one policy, decided
before anything opens; a link never runs a program:

- `#name` scrolls within the document;
- a relative path, or a `file:` URL without a host, resolves against the document's
  folder: a file that a viewer shows opens in the same viewer window, a folder opens
  in the active panel, and any other file is shown in the active panel with the
  cursor on it;
- `http`, `https` and `mailto` links go to the system's default handler;
- other schemes are refused, and the status line names the scheme.

Pointing at a link shows its target on the status line, and a click follows it. In a
Markdown page, Tab and Shift+Tab move a focus ring through the links and Enter follows
the focused one; an HTML page keeps these keys. Alt+Shift+Left and Alt+Shift+Right,
or the mouse's Back and Forward buttons, go back to the document a link was followed
from and forward again, in Markdown and HTML alike. Alt+Left and Alt+Right cycle files,
except in an HTML page with a page to go back or forward to, as the **Back** and
**Forward** buttons show ([keyboard map](KEYBOARD_SHORTCUTS.md)). An offline HTML page's
script cannot open a web or mail address in your browser by itself: only the link under
the pointer, which the status line shows, opens.

Plugins are independent Cargo projects in `plugins/`, with their own tests and locks.
Only built packages under `viewers/<package>/plugin.toml` (text/source/Markdown),
`viewers/<package>/image-viewer.json` (isolated image/PDF worker) or
`viewers/<package>/web-viewer.json` (HTML viewer) in the resolved installation are
discovered; video playback uses the video thumbnailer package under `thumbnailers/`.
Options → Plugins lists available/refused packages and lets you disable
or enable them, edit their extension associations and choose precedence for the next
startup. Missing or disabled plugins remove their
capability; the host has no built-in parser fallback. At most four viewer windows
may be open. Reading, parsing, search, layout and rasterization run on backend workers.

Native plugin code is trusted application code, **not sandboxed**. Keep the executable
with its `viewers/`, `filesystems/` and `thumbnailers/` folders together, and never
install DLLs from the directory being browsed. The loader refuses an installation, a
file in it or a folder above it that is a link leading elsewhere, or that belongs to
an account other than you, the system or administrators (Linux: you or root). See the
[resolution and security contract](developer/RUNTIME_DISCOVERY.md).

### Shared installations

A release package runs from any folder you extract it to; no script or installer is
needed. When other accounts can change the installation (a folder created directly
below a Windows drive root such as `C:\Tools`, which every account may modify, or on
Linux group- or world-writable files), plugins still load: anyone who can change those
files could replace Midnight Explorer itself. A notice above the panels says so once
for that folder:

- **Secure this folder** (no administrator rights; only for a folder you own) limits the
  folder and everything in it to you, the system and administrators. On Windows it
  inherits its permissions from its parent again when that is private (this repairs a
  folder you moved there from a shared location, which keeps its old permissions), and
  otherwise gets permissions of its own. Linux removes group and other write
  permissions. Folders above the installation are never changed; the notice names one
  that other accounts can still rename or replace.
- **Move to my profile…** copies the complete installation to
  `%LOCALAPPDATA%\Programs\Midnight Explorer` (Linux:
  `$XDG_DATA_HOME/midnight-explorer/app`, by default `~/.local/share/...`), verifies
  every file by size and SHA-256, asks before replacing an earlier copy and, when the
  copy is private, offers **Restart from the copy**. The original folder stays; **Open
  the original folder** shows it in the active panel so you can delete it.
- **Dismiss** hides the notice for this installation folder for good (it is
  remembered in `dismissed-notices.json` beside the preferences).

Moving the folder yourself with Windows Explorer on the same drive keeps its old
permissions; copy it or use the buttons instead. **Retry with authorization** still
needs an installation that only administrators can change, such as a folder under
`C:\Program Files` ([directory authorization helper](../SECURITY.md#directory-authorization-helper)).

On Windows, `build.bat` therefore deploys each build to a new private installation
under your user profile and registers it, so that `_build\release\midnight-explorer.exe` can use
protected plugins even when the checkout is on a shared or writable drive
([Windows x64](#windows-x64) has the commands). Run `midnight-explorer.exe` from the printed
installation path or the matching `_build` path. Deployment copies the executable and
its plugin packages, protects the new installation's permissions, and refuses unsafe
parents or an existing destination. It never changes the checkout's permissions or
launches the app. Compiler and intermediate files remain in `_build`.
Registrations live in `%USERPROFILE%\.midnight-explorer\plugin-runtimes`. Each is
named by the deployed host's SHA-256, which deployment checked against the build
output. By default, a protected launcher pointer for the build output's `midnight-explorer.exe` path
selects the registration while that executable keeps the size and modification time
recorded at deployment; startup then hashes no executable. With
`MIDNIGHT_VERIFY_RUNTIME_SHA256=1`, startup instead hashes the running host, selects
the registration named by that SHA-256 and requires the registered executable to have
the same hash. Protected portable installations still discover their adjacent
plugins directly, and so do shared ones without a registration.

Options → Editors configures F4. Ordered editor rules match the file name with
wildcard masks or a regular expression; F4 uses the first enabled match, and the
fallback handles other files. Each rule and the fallback use the system default
editor or an absolute executable with literal arguments, one per row, in which
`{file}` must occupy a complete argument; a rule can also set a working directory (by
default the editor's folder). Paths are never expanded through a shell. The platform
default and launch errors are shown explicitly.

The [documentation map](README.md) distinguishes current contracts,
accepted future work and historical verification records.
