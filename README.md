# Midnight Explorer

[Make File List](docs/MAKE_FILE_LIST.md) exports a captured panel selection with
Salamander-compatible templates to the clipboard, internal viewer or a UTF-8 file
(Ctrl+M; also available from the top toolbar and F10 Tools menu).

A dual-pane desktop file manager written in Rust with Slint and Tokio.

An optional [HTML viewer](docs/developer/WEB_VIEWER_API.md) renders HTML with
embedded Chromium inside the normal viewer. Its runtime is a separate plugin
package; building `me.exe` alone does not install Chromium. Build/package steps
are in the [independent provider README](plugins/midnight-viewer-html/README.md).
F3 handles `.html`, `.htm` and `.xhtml`; Ctrl+F searches and Escape closes the
viewer. HTML starts offline with JavaScript disabled. Windows rendering is
qualified; the Linux browser runtime still requires native qualification.

Product version **0.2** (Cargo SemVer `0.2.0`); the native window title shows
`Midnight Explorer 0.2 — File Manager`. The displayed version is derived from the package version.

Prebuilt Windows/Linux previews are published on the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases).
See [feature status](docs/FEATURE_STATUS.md) for the distinction between implemented
functionality and the complete Copy/Move, Find and filesystem-plugin backlog.
Published preview archives may precede the current source; their bundled feature-status file
describes that particular build.

Lossless Unicode throughout the product, security and backend-only non-UI work
are explicit requirements. See [security and integrity rules](SECURITY.md).
Third-party libraries must be free of mandatory license fees, with open source
preferred. See the [dependency policy](docs/DEPENDENCY_POLICY.md),
[complete library/license inventory](docs/THIRD_PARTY.md) and
[developer API documentation](docs/developer/README.md).

Stage 1 provides independent local directory navigation, mounted-volume switching,
history, sorting, selection, mouse/keyboard navigation, and background metadata/capacity
queries, native watches, a command editor and appearance preferences. Internal
file operations include confirmed rename, recursive directory creation, empty-file
creation (toolbar or Shift+F4), and queued deletion (system Trash by default,
permanent deletion with Shift). Empty-file creation accepts a name and never
overwrites an existing entry.
Explicitly opened files use host associations,
and explicitly submitted shell commands run with
the selected user/sudo permissions. F3 opens an installed internal viewer; Alt+F3 forces
plain text, and F4 opens the configured external editor. Ctrl+C/Ctrl+V copies files and ordinary folders through a
background queue with independent SHA-256 destination readback by default. Existing names
prompt for Skip, a different name, explicit folder Merge or recoverable Replace;
replacement retains the displaced original. F6 and Ctrl+X/Ctrl+V use ordinary
Move on qualified Windows NTFS and Linux ext storage. Same-volume moves use native
rename without reading file contents. Cross-volume moves independently verify each
destination, then delete that exact source before continuing. Recursive ordinary
folders use the same per-file checks and remove emptied source folders last.
There is no source-recovery mode or journal. See the
[current accepted follow-ups](docs/superpowers/specs/2026-11-07-panel-viewer-followups.md).
Link/provider moves, link copying and full metadata preservation remain pending;
required modified-time preservation currently accepts regular files only.
See [the transfer capabilities and implementation limits](docs/developer/TRANSFERS.md).

Copy/Move pauses for supported Retry/Skip/Cancel decisions on I/O errors, including
an unavailable destination or a full disk. Retry preserves the verification policy captured when the job started;
Skip All applies only to the same error type and phase in that job. Decide later
keeps the pending decision available from Jobs. Copy, Move and Delete share one
progress-window component, with a separate modeless window for each operation.
The speed chart also shows overall completion; current-file writes have their own
meter. Discovery precedes known totals, and Pause/Resume waits for safe backend
checkpoints. ETA remains Calculating when its work estimate is unavailable.
X, Escape and Cancel operation request cancellation of that exact job, including
cleanup of unpublished temporary files; they never mean silent background work.
Successful automatically opened progress closes. Manually reopened progress and
manually opened Jobs stay open. All job receipts remain for the session until
explicitly cleared; Jobs is the ledger and opens individual progress windows.
See [operation progress behavior and limits](docs/developer/OPERATION_PROGRESS.md).
See [error recovery and validation](docs/verification/2026-11-08-transfer-errors.md).

Mouse file dragging uses the same verified copy queue for drops into either panel's
displayed directory. Dragging a selected row keeps the selected group; dragging
an unselected row targets that item. Native exchange with other applications uses
Windows OLE or Linux X11/XWayland XDND. Native Wayland currently supports internal
panel dragging only. Drops copy and retain sources; copying out is performed by
the receiving application. See [drag/drop controls](docs/KEYBOARD_SHORTCUTS.md)
and the [native boundary](SECURITY.md#file-drag-and-drop).

## Build and run

See [system requirements](docs/SYSTEM_REQUIREMENTS.md) for OS versions, runtime
libraries, RAM and development-machine requirements. Current compatibility targets
are Windows 10 22H2 / Windows 11 x64, Ubuntu 24.04+ LTS, Mint 22.x,
Fedora 44 and newer maintained releases, and openSUSE Leap 16.x / Tumbleweed on x86-64;
qualification gaps and the planned macOS port are documented there. Provision
4 GB total RAM (6 GB for Ubuntu 26.04), preferably 8 GB, with 1 GiB
available for ordinary application use. These are provisional budgets, not measured
hard minima; building the project needs substantially more memory.

Release builds use full optimization, fat LTO and one code-generation unit while
retaining integrity checks. See [release settings and verification](docs/RELEASE_BUILD.md).

Desktop compatibility targets are **Windows x64 with MSVC** and **Linux**, including
Ubuntu, Linux Mint, Fedora and openSUSE. Fedora/openSUSE are newly added targets;
their native qualification runs are pending. macOS is planned; it has no platform adapter and does not
currently build. Slint's own macOS support does not change Midnight Explorer's platform support.

For Windows or direct native Cargo commands, install [Rust with rustup](https://rust-lang.org/tools/install/),
then reopen your terminal so `cargo` and `rustup` are on `PATH`. The Linux helper
can bootstrap Rust as described below. The repository pins **Rust 1.99.1**
with rustfmt and Clippy in [rust-toolchain.toml](rust-toolchain.toml). From the
repository root, `rustup show` selects/downloads that toolchain. Cargo downloads
Slint and the other locked dependencies on the first build; no separate Slint SDK
installation is needed. The first build takes longer than subsequent builds.

The commands below assume the current directory is the repository root. The
optional root build helpers locate the repository themselves, so they also work
when invoked by a quoted path from another directory. For builds they accept no
argument (debug), `debug`, or `release`, run a locked build of `midnight-explorer`,
and return failures to the caller. Linux `build.sh` also supports `clean`,
`--dry-run` and explicit `--install-deps`. It selects native or Docker builds and
can bootstrap Rust; see the Linux instructions below. `build.bat` expects its
prerequisites to be installed already. Application builds through the helpers also build the
platform's independent native and Network Neighborhood
filesystem packages under `filesystems/native.{windows,linux}/` and
`filesystems/network.{windows,linux}/`, then the five
independent viewer packages using the public SDK under `viewers/`. They do
not launch the application. Host Linux packages are installed only with explicit
`--install-deps`; Docker setup installs packages inside its image. The Linux
Docker path runs the full app/elevation/native-filesystem/network/viewer build
chain. Direct `cargo build -p midnight-explorer` builds the host only; its elevation
helper and provider/viewer packages need separate build/staging. Run
`scripts/build-native-filesystem.ps1` or
`scripts/build-native-filesystem.sh` before launching it, add Network Neighborhood with
`scripts/build-network-filesystem.ps1` or `.sh`, and use `scripts/build-viewers.ps1`
or `scripts/build-viewers.sh` to add viewers. Native browsing and F3 source reads have
no built-in fallback when their native filesystem package is absent or incompatible.
Archive and thumbnail workers have separate builds; the root helpers do not install
them. Use the archive plugin's `scripts/build.ps1` or `scripts/build.sh` with a
canonical worker SDK configuration, and `scripts/build-thumbnailers.ps1` or
`scripts/build-thumbnailers.sh` for thumbnails. Stage the results in executable-adjacent
`filesystems/` and `thumbnailers/` respectively. See the
[filesystem API and build reference](docs/developer/FILESYSTEM_API.md) and
[worker SDK guide](docs/developer/WORKER_SDK.md). Release packaging accepts these
package roots, requires one compatible native and one network filesystem provider, and
preserves executable permissions and notices. Archive and thumbnail packages remain optional.
The [network plugin guide](plugins/midnight-filesystem-network/README.md) describes
its optional Linux GIO/GVfs runtime and explicit unavailable/authentication states.
See the [native filesystem build instructions](docs/developer/NATIVE_FILESYSTEM_API.md#building-the-native-package)
for package-only builds, test switches and offline options.
The Cargo package remains `midnight-explorer`; its executable is `me.exe` on Windows
and `me` on Linux.
Output paths below assume the default native target and no `CARGO_TARGET_DIR`
override. Cargo's checked-in configuration puts compiled and generated output
under `_build/`. The build helpers also keep their child processes'
temporary files under `_build/tmp/`.

Permission-denied native folders offer an explicit authorized listing retry using
Windows UAC or Linux polkit (`pkexec` and a desktop authorization agent). The build
helpers also build `midnight-elevated-listing` alongside the host; keep its `.exe`
on Windows, or native executable on Linux, in the same protected installation.
This one-shot helper lists the requested directory only, loads no plugins and
does not elevate ordinary file operations, change ACLs or store passwords. See
[the authorization boundary](docs/verification/2026-11-11-directory-authorization.md).

### Windows x64

Install the Visual Studio **Desktop development with C++** workload, including
the MSVC x64/x86 tools and a Windows SDK, as described in the
[rustup MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
Use the `x86_64-pc-windows-msvc` Rust host toolchain; Midnight Explorer's Windows build does not
support the GNU/MinGW toolchain. A complete Visual Studio IDE is optional.

Build from PowerShell or Command Prompt:

```powershell
.\build.bat
.\build.bat release
```

Host-only commands with a normal Rust installation (native filesystem and viewer packages
must also be built with the helpers above):

```powershell
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo build -p midnight-explorer -p midnight-elevation --locked --release
```

The default build is debug. The frame rendering libraries use optimization level 2
even in debug, and generated/UI code uses level 1. Backend/application code retains
normal debug behavior. Release still optimizes the full application. The first build after
changing these profiles recompiles the affected dependencies under `_build`.

Run either executable directly, or let Cargo build and run it. `build.bat` also
deploys the completed Windows bundle to a new private installation and registers
it for the exact host executable's SHA-256. This lets `_build\release\me.exe`
use protected plugins even when its checkout is on a shared drive. Plugin code
is never loaded from an unsafe shared directory. After a raw Cargo build, or after
adding/removing plugin packages, deploy and register explicitly:

```powershell
powershell -NoProfile -File scripts/deploy-private.ps1 -SourceDirectory _build/release -IncludeFilesystems -IncludeThumbnailers -RegisterRuntime -CreateDesktopShortcut
```

Use the original build EXE, the **Midnight Explorer** desktop shortcut, or the
private installation path printed by that command. Restart after deployment.
`-IncludeFilesystems` includes the required native filesystem and any built archive
workers. Omit `-IncludeThumbnailers` when optional thumbnail packages are absent.
The helper does not launch the app or change saved settings. Direct launches:

```powershell
.\_build\debug\me.exe
.\_build\release\me.exe --left "C:\Users" --right "C:\Program Files"
cargo run -p midnight-explorer --locked -- --left "C:\Users" --right "C:\Windows"
cargo run -p midnight-explorer --locked --release
```

For a filesystem smoke check that exits without opening the UI:

```powershell
.\_build\debug\me.exe --smoke-test --left "C:\Users" --right "C:\Windows"
```

An existing development checkout may have a private toolchain/cache under
`.tools/`. It is optional: `build.bat` prefers `cargo` on `PATH`, and falls back
to [scripts/cargo.ps1](scripts/cargo.ps1) only when that local cache exists and
normal Cargo is unavailable. You can also invoke the helper explicitly:

```powershell
.\scripts\cargo.ps1 run -p midnight-explorer --locked '--' --left "C:\Users" --right "C:\Windows"
```

Quote the literal `'--'` separator when forwarding application arguments through
that PowerShell helper. Regular `cargo` commands use the ordinary `--` separator.

### Linux / Ubuntu / Mint

Run `./build.sh` or `./build.sh release`. When the native compiler or required
libraries are missing, the helper builds an Ubuntu 24.04 Docker image containing
the dependencies and pinned Rust toolchain. Docker must already be running and
accessible to your user without sudo. The first build needs internet access;
subsequent builds reuse the image and Cargo cache.

The container mounts source read-only and runs the same application, elevation
helper, native/network filesystem and viewer chain as a native build. It writes
output as your user under `_build/`; Cargo downloads and the container home are
cached under `.tools/docker/`.
The Docker build always writes to `_build/`, regardless of host `CARGO_TARGET_DIR`.
The resulting binary targets Ubuntu 24.04 or a compatible system and needs the
usual desktop runtime libraries to run outside the container.

To choose the build environment explicitly:

```sh
FM_BUILD_BACKEND=docker ./build.sh release
FM_BUILD_BACKEND=docker ./build.sh release --dry-run
FM_BUILD_BACKEND=native ./build.sh release
```

The default is `FM_BUILD_BACKEND=auto`: missing native compiler/pkg-config
libraries select Docker. Native mode uses host libraries and can install Rust
in your user account without sudo; explicit native package installation is
described below.
If Docker's default DNS cannot resolve package servers, set an accessible DNS
server for both image setup and the build, for example
`FM_DOCKER_DNS=1.1.1.1 ./build.sh release`. This does not change host DNS settings.

For native builds, `build.sh` reuses Cargo on `PATH`, under
`${CARGO_HOME:-$HOME/.cargo}/bin`, or in `.tools/cargo/bin`. If none exists, it
bootstraps rustup using curl and CA certificates. New toolchain caches default to
`.tools/cargo/` and `.tools/rustup/`, respecting explicit `CARGO_HOME` and
`RUSTUP_HOME` overrides; no host sudo is needed for this Rust setup.

For native builds, install the compiler/linker and development libraries used by
the current Linux build. Viewer package helpers also require Python 3.11 or newer for the
released SDK configuration. The [Linux dependency helper](scripts/setup-linux.sh)
detects Ubuntu/Mint, Fedora or openSUSE and prints a package plan by default:

```sh
sh scripts/setup-linux.sh
sh scripts/setup-linux.sh --install --dry-run
```

Only explicit `--install` runs the package manager, using `sudo` when needed.
`--dry-run` keeps it plan-only even with `--install`. This package helper does not
install Rust; native `build.sh` can bootstrap it when needed. Alternatively,
install the packages manually. For Ubuntu/Mint:

```sh
sudo apt update
sudo apt install build-essential python3 pkg-config libfontconfig1-dev libfreetype6-dev \
    libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libxcb1-dev libwayland-dev libudev-dev
```

Fedora 44+ (maintained desktop release, x86-64):

```sh
sudo dnf install gcc gcc-c++ make python3 pkgconf-pkg-config fontconfig-devel freetype-devel \
    libxkbcommon-devel libxkbcommon-x11-devel libX11-devel libXcursor-devel \
    libxcb-devel wayland-devel systemd-devel xdg-utils
```

openSUSE Leap 16.x / Tumbleweed (x86-64):

```sh
sudo zypper install gcc gcc-c++ make python3 pkg-config xdg-utils \
    'pkgconfig(fontconfig)' 'pkgconfig(freetype2)' 'pkgconfig(xkbcommon)' \
    'pkgconfig(xkbcommon-x11)' 'pkgconfig(x11)' 'pkgconfig(xcursor)' \
    'pkgconfig(xcb)' 'pkgconfig(wayland-client)' 'pkgconfig(libudev)'
```

The openSUSE command asks Zypper for library capabilities so it can select the
distribution's development packages. Fedora/openSUSE instructions have not yet
been exercised on clean native machines. Keep the distro's normal security policy
enabled when qualifying filesystem operations and desktop integration. See
[Fedora's C++ setup](https://developer.fedoraproject.org/tech/languages/c/cpp_installation.html)
and the [Zypper manual](https://manpages.opensuse.org/Tumbleweed/zypper/zypper.8.en.html).

For command-line execution, install a currently recognized terminal: `xterm`,
Konsole, GNOME Terminal or Xfce Terminal. For example, `sudo dnf install xterm`
or `sudo zypper install xterm`. An arbitrary desktop's default terminal is not
automatically supported by the current launcher.

To install Rust manually, use the [official rustup instructions](https://rust-lang.org/tools/install/),
then open a new terminal or load rustup's environment with `. "$HOME/.cargo/env"`.
The Linux helper accepts
`./build.sh [debug|release|clean] [--install-deps] [--dry-run]`:

```sh
./build.sh
./build.sh debug
./build.sh release
./build.sh --dry-run
FM_BUILD_BACKEND=native ./build.sh debug --install-deps --dry-run
FM_BUILD_BACKEND=native ./build.sh debug --install-deps
```

`--install-deps` selects native setup and explicitly opts into the dependency
helper's host package installation. It is rejected with
`FM_BUILD_BACKEND=docker`. `--dry-run` prints the selected setup/build plan without
package installation, Rust bootstrap or compilation. With no profile, debug is
selected. The helper requires Linux and rejects macOS; application and build
support for macOS remain pending.

Build only the host with Cargo (install the native provider/viewers separately as
described above):

```sh
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo build -p midnight-explorer -p midnight-elevation --locked --release
```

Preview or remove repository build output with:

```sh
./build.sh clean --dry-run
./build.sh clean
```

Clean is limited to the repository's `_build/`, regardless of `CARGO_TARGET_DIR`.
It preserves `.tools/` and the legacy `_build/docker/` cache. It rejects a symlinked
`_build` and validates Linux mount metadata before removal, refusing mounts/bind
mounts in the deletion scope outside the preserved Docker cache. Missing or invalid
mount metadata fails closed. Clean cannot be combined with `--install-deps`.
It needs no Cargo, Docker, network or sudo and leaves `.tools/docker/` caches and
the Docker dependency image intact. Native output in a custom `CARGO_TARGET_DIR`
is outside this cleanup scope; clean that separately if needed.

Run in a graphical desktop session with X11 or Wayland and working graphics
drivers. Slint's [Winit backend documentation](https://docs.slint.dev/latest/docs/slint/guide/backends-and-renderers/backend_winit/)
describes the display/runtime libraries. The normal renderer is FemtoVG-WGPU.

```sh
./_build/debug/me
./_build/release/me --left "$HOME" --right /tmp
cargo run -p midnight-explorer --locked -- --left "$HOME" --right /tmp
cargo run -p midnight-explorer --locked --release
```

The filesystem smoke check does not require a display:

```sh
./_build/debug/me --smoke-test --left "$HOME" --right /tmp
```

Linux discovers already-mounted filesystems through `/proc/self/mountinfo`;
mounting or safely ejecting devices is not implemented. The `.tools/` WSL paths
and isolated native-library prefix used for internal QA are not installation
prerequisites for an ordinary Linux build.

For other Cargo commands, [scripts/cargo.sh](scripts/cargo.sh) keeps temporary
files in `_build/tmp/` too, for example `sh scripts/cargo.sh test --workspace --locked`.
It forwards arguments and Cargo's exit status and uses `cargo` on `PATH`.
When Windows and WSL use the same checkout, give Linux its separate cache:
`CARGO_TARGET_DIR="$PWD/_build/linux" sh scripts/cargo.sh test --workspace --locked`.

### Linux desktop launcher

The moon icon is embedded for X11. Native Wayland resolves it using the application
ID and an installed desktop entry. Optional per-user installation after a release
build, from the repository root:

```sh
install -Dm755 _build/release/me "$HOME/.local/bin/me"
install -Dm755 _build/release/midnight-elevated-listing \
    "$HOME/.local/bin/midnight-elevated-listing"
mkdir -p "$HOME/.local/bin/viewers"
cp -R _build/release/viewers/. "$HOME/.local/bin/viewers/"
install -Dm644 packaging/linux/io.github.TomasJelinek.MidnightExplorer.desktop \
    "$HOME/.local/share/applications/io.github.TomasJelinek.MidnightExplorer.desktop"
install -Dm644 assets/branding/midnight-explorer.png \
    "$HOME/.local/share/icons/hicolor/256x256/apps/io.github.TomasJelinek.MidnightExplorer.png"
```

The graphical session's `PATH` must include `$HOME/.local/bin`; sign in again after
changing that environment. If using custom XDG data directories, install the entry
and icon under `$XDG_DATA_HOME` instead of `$HOME/.local/share`. These commands
install the application but do not make it the default file manager.

### Rendering diagnostics

GPU acceleration is optional. Normal rendering uses Slint FemtoVG-WGPU with
system-font fallback; Windows prefers Direct3D. When default GPU selection or
initial window/device setup fails before the first window is shown, the app
retries once with Slint's CPU renderer. Existing file operations and already
visible windows are never restarted by this startup recovery. Explicit
`SLINT_BACKEND` choices remain authoritative; `--software` selects CPU rendering:

```sh
cargo run -p midnight-explorer --locked -- --software
```

`SLINT_BACKEND` is respected when `--software` is not supplied. GUI smoke checks
and `--snapshot FILE.png` need a working display; see [Checks](#checks) below.
The pinned software renderer does not render bitmap emoji; complete glyph and
font qualification remains separate from functional CPU startup and scrolling.
For WGPU diagnostics only,
`SLINT_WGPU_CPU=1 ./_build/debug/me` permits a CPU adapter. This still needs the
graphics runtime and fonts; it does not establish production GPU performance.

## Controls

See the [complete current keyboard map](docs/KEYBOARD_SHORTCUTS.md), including
editor scope and shortcuts reserved for later stages.

- Ctrl+A selects top-level files only; Ctrl+Shift+A includes top-level folders.
  Both replace selection and clear nested marks. Ctrl+D clears all selected files
  and folders, including marked inline descendants; Ctrl+Shift+D has no default binding.
  Ctrl+I inverts files only; Ctrl+Shift+I inverts top-level files and folders.
  This works inside archives too. The parent `..` is excluded; text editors retain
  normal editing. These shortcuts can be reassigned in Options.
- Type in the file pane to search filename prefixes, always Unicode
  case-insensitively. The matching prefix is colored; Up/Down visits matches and
  Escape exits. Options can filter the list while typing instead of just jumping.

- Ctrl+Shift+B / Ctrl+Shift+L / Ctrl+Shift+T: Brief / Detailed / Thumbnails
  in the active pane. Reassign these in Options → Keyboard; F10 shows current keys.
- Tab: switch active file pane.
- Ctrl+T / Ctrl+W: add / close a folder tab in the focused pane.
- Ctrl+Tab: focus the command editor; press again to return to the active pane.
  Ctrl+Shift+Tab selects the previous folder tab. Captions follow the folder name;
  full paths are in tooltips. Tabs restore their order, paths and active position
  after restart; explicit `--left`/`--right` arguments override that pane. Middle-click
  also closes a tab. Tabs shrink uniformly before revealing scroll arrows.
- Ctrl+U: swap the complete panels while keeping their operations and active content.
  Drag the divider for unequal widths; double-click it to center the panels.
- Ctrl+Shift+1–9: select a numbered tab. Tabs sit below the drive toolbar.
- Hot Paths store unique native paths, shown without redundant names. Their menus
  show assigned numbers **1–9, 0**; pressing a digit in a focused panel opens its
  path. Editors/search keep normal digit input. Ctrl+number and Shift+number remain
  aliases (Alt targets the other pane); Ctrl+Shift+0 assigns slot 10. The manager
  keeps a clicked path selected while its shortcut is edited. Saved paths also
  appear in the full drive dropdown, without extra buttons on the drive toolbar.
  See [identity and assignment rules](docs/superpowers/specs/2026-11-06-unique-numbered-hot-paths.md).
- Left click: focus the row and its pane; Ctrl+click toggles selection, Shift+click
  selects an inclusive range, Ctrl+Shift+click adds that range. Accepted gestures
  appear immediately while backend confirmations arrive. Panel status shows
  selected file/folder counts; attributes use readable flags or Unix chmod notation.
- Enter / unmodified left double click: enter a directory or open a file in its
  default system application. Directory links stay inside the file manager.
- Backspace / toolbar Up: parent directory.
- Alt+Left / Alt+Right: back / forward history; adjacent dropdowns choose a
  particular history entry. Click a highlighted path prefix to visit its ancestor.
  Right-click the prefix to copy it to the clipboard without navigating.
- Arrow keys, Page Up/Down, Home/End: move the cursor.
- Space / Insert: toggle selection and advance, clamped at the last row. Space
  also schedules the focused directory's size without following links.
- Ctrl+R / toolbar Refresh: reload the current directory.
- F10 opens the main menu with file operations, selection, panel navigation/view
  controls, Find, comparisons, checksums, Hot Paths, Options and Jobs. Choose a
  category to narrow the list; entries show configured keys and unavailable
  operations remain visible but disabled.
- Ctrl+Shift+S: calculate selected folder sizes, or all folders when none are selected;
  press again to cancel. Partial/running values are marked; links are skipped.
- Ctrl+[ / Ctrl+]: insert the left/right path at the caret or replace the selection.
  The physical bracket keys also work with Shift and with non-US layouts.
  Ctrl+P inserts the active path. Directory insertions end with a native path
  separator, without doubling an existing one.
- Ctrl+Enter: append the focused filename at the end of the command draft.
  Insertion adds no quotes, escaping or spaces and never submits the command.
- Alt+F1 / Alt+F2: open left / right drive choices. Ctrl+F3/F4/F5/F6 sort by
  name/type/time/size; Ctrl+F9 refreshes; Ctrl+Shift+F10 calculates folder sizes.
- Enter in the command editor: open the system terminal in the active directory
  and run the submitted command. Insertion alone never executes a command.
- The full command directory ends with `>`. Its history dropdown restores the
  selected draft without running it. The latest 100 unique exact strings are
  persisted (up to 1MiB aggregate text); recalling never enables elevated mode.
- The terminal icon opens a new interactive terminal in the active folder and
  preserves the command draft. Its adjacent arrow chooses PowerShell/cmd on Windows
  or Terminal/Terminal(sudo) on Linux. A nonempty draft stays bound to its grammar;
  clear it before switching PowerShell/cmd. Sudo selection is session-only.
- The compact terminal icon and navigation controls explain themselves through
  tooltips. Drive tooltips show free space and Windows volume names; expanded
  drive choices include those details. Quick buttons and their dropdown share
  discovered drives and special locations, with full-path tooltips. The neutral
  location-menu button avoids repeating the current drive letter. Its dropdown
  uses the available window height; named OneDrive accounts remain in the menu
  while the toolbar shows one primary OneDrive shortcut.
  Suitable host icons load on the backend; missing native action icons retain
  semantic application vectors and native colors remain intact.
- The top toolbar's Options gear groups settings by topic, including Panel, Appearance, Command,
  Keyboard and Custom Commands. It offers readable
  or exact-byte sizes, history persistence/Clear, 26 themes including Slate Blue,
  Slate Teal and Slate Violet, Desktop/Touch spacing, an installed font family and
  independent Desktop/Touch base sizes (8–32px). The [theme design](docs/superpowers/specs/2026-11-06-theme-expansion-design.md)
  and [palette provenance](docs/THEME_PROVENANCE.md) describe the catalog. Preview
  is immediate and reversible. **OK** validates all pages, saves asynchronously
  and closes after success. **Cancel**, Escape and X discard all edits and restore
  previewed theme, spacing and fonts. Validation or save errors keep Options open.
  **Reset to defaults** resets all Options pages while preserving tabs, command
  history, hot paths and custom commands. It is a preview until OK; Cancel undoes
  the reset too.
  See the [Options transaction design](docs/superpowers/specs/2026-11-08-options-transaction.md).
  Colors offers a visible priority list of wildcard/regex filename rules; the
  first matching enabled rule wins. Set a base text color and
  optional background; focused/selected variants are derived automatically and
  previewed together. Matching runs on backend workers in every panel view.
  Color swatches open a movable, resizable picker popup. Its controls scroll in
  small windows while Use color / Cancel stay visible; Escape cancels just the
  picker, and the outer Options OK / Cancel still controls saving.
  Desktop uses denser rows/toolbars; compact Touch uses 40px base targets. The active folder
  appears in full beside the command input in the same single row, without
  elision or a fixed path-width cap. Extreme paths remain fully accessible in a
  horizontal viewport while preserving editor space.
  The command row uses the same height as the function-button toolbar. Alt+Up
  and Alt+Down rotate stored commands without changing the chosen terminal;
  Enter submits the displayed draft.
  The window minimum stays 320×200 logical pixels at every font/density. Compact
  layout keeps the active panel and essential controls accessible; wide controls scroll.
- One shared toolbar above both panels operates on the active panel. It includes
  navigation/history, New File, Attributes, Find, Filter, file/directory comparison,
  Checksums, Folder Sizes, Properties, Hot Paths, view mode and sort. Drives,
  paths and tabs remain per-panel. Narrow windows scroll the shared toolbar.
  The Options gear stays fixed at its right end; the moon button immediately
  beside it opens About with the application version. The bottom bar keeps
  F1–F12 and Jobs.
  Switching panels or locations invalidates an open context-dependent popup.
- Custom Commands has a dedicated toolbar above the command editor whenever at
  least one command is configured. At compact window sizes, the shared toolbar's
  **Custom commands** button opens the same commands in a popup to preserve file
  browsing space. Icons may have optional text; overflow scrolls.
  Right-click opens the matching settings page and selects that command.
  Drag rows in Options → Custom Commands to change their order. A marker shows
  the insertion point; the list scrolls at its edges. Escape cancels the drag.
  Order is staged without changing command IDs or shortcuts; Up/Down remain
  available. **Save** validates and stages an individual form so another command
  can be edited. Options **OK** persists staged changes; **Cancel** discards the
  session.
  Program mode passes native arguments separately; terminal mode uses literal
  user-authored PowerShell/POSIX source with explicitly declared context variables.
  Preview does not execute. Jobs reports started/not-started processes; Cancel
  stops future launches without terminating applications already started.
  Assign `user-command.<id>` shortcuts in Options → Keyboard. The
  [Custom Commands contract](docs/developer/USER_COMMANDS.md) records platform
  and workload limits. See the [settings verification](docs/verification/2026-11-07-custom-command-settings.md)
  for Windows/Linux input, scrolling and persistence checks.
- Panel scrolling animates discrete wheel input in pixels and gently settles on
  a row after input and inertia stop. Native touch/trackpad physics are preserved.
  Native drive icons precede captions and dropdown choices; special folders show
  their containing volume, while detected native network paths show Network.
- Each panel's details strip shows the focused name, exact byte size, date/time,
  attributes and Unix permissions where available. Click a field to copy it.

- Alt+Enter: properties of selected items or the focused file/folder.
- `.` in the drive dropdown, or click **. As Other Panel**: open the opposite
  panel's current folder. Enter in files opens the focused folder or file.
- Shift+drive letter: switch the focused file pane on Windows.
- F2: rename the focused item. A regular-file name collision uses the shared
  overwrite prompt; replacement requires confirmation of that exact destination.
  Self, hardlink, directory and link replacement remain protected.
  F7: create an absolute or relative folder path,
  including `..` and missing intermediate folders. Paths may leave the active panel.
- Shift+F4 or the shared toolbar's **New empty file** button: enter a filename and press
  Enter to create a zero-byte file. Escape cancels; existing entries are never
  overwritten. The new file is focused when creation finishes.
- Delete/F8: confirm moving selected items to Trash. Shift+Delete/Shift+F8:
  explicitly confirm permanent deletion. A Trash failure never permanently deletes.
  One file shows its name, exact size and modified date/time. Multiple entries show
  file count/total file bytes and folder count; folder contents are not recursively
  counted just to open the confirmation.
- F11/F12: Windows system dialogs for connecting/disconnecting network drives.
  The Options gear is at the right of the top toolbar; Jobs shows the operation
  queue and session history; Open displays the selected operation's progress.
- Hold Ctrl to show the effective Ctrl+F1–F12 captions and actions in the footer.
  Remapped or disabled shortcuts are reflected in this layer. Ctrl+F11 expands
  the active panel across the entire panels area, completely hiding the other
  panel and splitter. Tab switches the visible panel; another Ctrl+F11 restores
  the previous split. Outside zoom, double-clicking the splitter centers it.
  The normal panel width ratio is saved automatically after adjustment and restored
  on the next startup, including when the window size changes.
- Ctrl+F10 or **Compare directories** in the shared top toolbar compares the
  two displayed native directories on the backend, selecting
  missing, larger or newer files. Size and modified time are enabled by default,
  with a two-second timestamp tolerance. Name-case matching and unmatched
  directories are configurable. Matching metadata does **not** verify file contents;
  content and recursive comparison are planned extensions.
- Ctrl+F12 opens a saved per-panel file filter. Choose wildcard masks, a Rust
  regular expression, or the supported metadata expression language, with explicit
  case sensitivity. Help/F1 explains each mode with examples. Directories remain visible;
  the filter combines with typing-based filename search. Invalid expressions do
  not replace the active filter.
- Ctrl+Shift+C or **Compare files** in the shared top toolbar compares two
  selected regular files in the active panel, or one focused/selected file with
  its same-name counterpart in the other panel. An explicit selection in the other
  panel takes priority. Automatic matching uses the exact native name among visible
  files; select both files explicitly to compare different names or letter case.
  Text pairs show line differences; image
  pairs provide Side-by-Side, Swipe and Onion Skin controls.
- F3 opens PDFs with the installed independent PDF viewer. Ctrl+PageUp/Down or
  the page buttons switch pages; Fit, zoom and pan are shared with image viewing.
  Each new PDF opens fitted to the window. Page changes retain manual zoom/100%;
  Fit adapts to each page's dimensions.
- F5 inside an archive copies the captured members to a native destination using
  the background queue and SHA-256 destination readback. Archive contents stay
  read-only. XZ, Bzip2 and Zstandard streams and their TAR variants are supported
  by the archive plugin alongside ZIP, 7z, GZIP and TAR.
- Ctrl+F2 or the panel's attributes icon opens **Attributes, times and case**.
  Ctrl+F7 opens the same editor with case conversion enabled. Other edits start
  as Leave unchanged. Preview the captured selection, then Apply to queue the
  reviewed draft. Native Windows attributes/compression/timestamps and Linux
  permissions/timestamps follow reported capabilities. Unicode case conversion
  supports whole names, basenames and extensions. Recursive edits process
  children first without following links or crossing mount points. Name conflicts
  offer Skip, Skip all or Cancel; existing entries are never overwritten.
- Click the blank area after the top breadcrumb to enter a custom path; Enter
  navigates and Escape cancels. Breadcrumb navigation/copy gestures remain available.
- Panel view picker: independent Brief, Detailed and large-icon Thumbnails modes.
  Brief fills columns downward; Left/Right changes columns. Thumbnails uses
  Left/Right between cells and Up/Down between grid rows. Page Up/Down moves a
  visible page. Active prefix search retains its existing match navigation.
  Installed thumbnailer plugins provide cached previews for supported image/video
  formats; see the [thumbnail design](docs/superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).
  **Options → Previews → Thumbnail size** offers 48, 64, 72 (default), 96, 128,
  192 and 256 logical pixels. Larger sizes retain the current preview while a
  sharper image is prepared for that file; smaller sizes reuse larger cached
  images. The setting previews immediately, Cancel restores it and OK saves it.
  See the [size and cache policy](docs/superpowers/specs/2026-11-09-thumbnail-size-and-cache.md)
  for display scaling, cache reuse and storage layout.
- Click column headers: change sorting. Cursor and selection remain attached to
  the original raw filesystem paths when the listing order changes.
  Repeated clicks toggle ascending/descending order. Completed folder sizes are
  retained and sorted on the backend; Refresh invalidates these live estimates.

Application hotkeys can be changed, disabled, or restored in **Options → Keyboard**.
Bindings are validated for conflicts in their applicable contexts; native text editing
and IME behavior are protected. The [shortcut reference](docs/KEYBOARD_SHORTCUTS.md)
lists defaults. Numbered tab and Hot Path bindings use physical digits across layouts.

Options, tabs, bookmarks, panel filters, enabled command history, viewer preferences
and shortcut overrides are stored as versioned UTF-8 JSON (supported schemas through 21) in the native
`%LOCALAPPDATA%\Midnight Explorer\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/midnight-explorer/preferences.json` /
`~/.config/midnight-explorer/preferences.json` on Linux. When the new file is absent,
the former `FM` (Windows) / `fm` (Linux) location is read safely; saves use only the
new location and preserve the old file.
Viewer preferences include wrapping, line numbers, fonts, Markdown presentation
and image/PDF zoom modes; see [viewer settings](docs/developer/VIEWER_PREFERENCES.md).
Source mode exposes line numbers and the current logical line/column. Markdown
offers GitHub (default), Compact and Comfortable layouts, plus font and size
controls. Exact distant source positions index in the background without delaying
scrolling. See [viewer presentation](docs/developer/VIEWER_PRESENTATION.md).
Storage and font/icon discovery run on backend workers. See the
[navigation and appearance design](docs/superpowers/specs/2026-11-04-navigation-and-appearance-design.md).
Exact byte sizes and date/time use system regional settings captured on the backend;
restart after changing the OS locale. Missing native formats use invariant/UTC
fallbacks. Name and extension sorting uses native system-locale case and accent
weights; equal size/date values use that same name order. The locale is captured
until restart. Invalid native names remain lossless and use deterministic raw-name
ordering after collatable names. Linux falls back to the C locale when the configured
locale is unavailable. Cached backend sort keys have a 4 MiB resource budget per key;
names are never truncated. See the [locale sorting design](docs/superpowers/specs/2026-11-06-locale-sorting-design.md).

The current [operation design](docs/superpowers/specs/2026-11-05-rename-delete-scroll-design.md)
documents confirmations, cancellation and native limitations: Linux mutations need
Linux5.6+ and same-device home Trash; Windows Shell Trash can refuse long paths
that native rename/create/permanent operations support.
The [panel navigation and details extension](docs/superpowers/specs/2026-11-05-panel-navigation-details-design.md)
defines current controls. A separate [Network filesystem plugin](docs/superpowers/specs/2026-11-05-network-filesystem-design.md)
discovers devices/shares through separate Windows and Linux adapters; macOS support remains planned.
The [panel and scrolling verification](docs/verification/panel-navigation-and-smooth-scrolling.md)
records regression coverage, native checks and the clean build layout.
The [pixel-scrolling contract](docs/developer/SCROLLING.md) and
[native measurements](docs/NATIVE_SCROLL_VERIFICATION.md) cover current list/viewer
input, renderer optimizations and remaining measured performance limits.
The [interaction and layout verification](docs/verification/panel-interaction-and-layout.md)
records Ctrl/Shift selection, splitter/tab/terminal controls, and measured debug
selection/resize performance, including remaining native rendering limits.

A small badge on a file/folder icon identifies compression reported by its
filesystem. File sizes remain logical sizes. An archive extension alone does not
set the badge; unavailable compression metadata remains unknown. Search, tooltip
placement and native metadata contracts are in the
[current design extension](docs/superpowers/specs/2026-11-05-panel-search-and-metadata-design.md).
The [verification record](docs/verification/panel-search-and-reuse.md) covers
search, native metadata and UI reuse. The
[generated-code review](docs/verification/slint-code-generation-size.md) explains
the remaining Slint expansion and measured optimization options.

Directory links and junctions have distinct link artwork. Windows uses the
system link overlay; other platforms prefer host-theme icons and fall back to
application artwork. Recursive folder-size scans skip links. Browsing a folder
does not inspect a link target merely to choose its icon.

Paths and file identities are kept separately from their displayed text. A failed
navigation preserves the last usable directory and reports the failed operation.
Each pane remembers its last visited directory for each volume during this run.
Visible directories are watched on the backend. Changes are batched after one
second of quiet, with a three-second cap during continuous activity. A 15-second
metadata check catches missing native/network notifications. Automatic refresh
preserves the viewport, selection and command editing; it invalidates earlier
recursive folder-size estimates.
Folder sizing uses a bounded fair batch and a shared I/O budget: up to four reads
only for proven local SSD, one stream for HDD/network/unknown storage. Detection
errors remain conservative. Other navigation work has independent capacity.
An active size scan continues across sorting and same-directory refresh; moving
the cursor, changing selection or working in the other pane does not cancel it.
Running totals retain a contrasting color and `≈` without an exclamation prefix.
Completed partial totals retain `≈` in ordinary text: inaccessible or skipped
entries mean the result is incomplete, even though the scan has finished.
File and folder rows have no hover tooltips; control/drive tooltips remain.
Size-only UI updates are combined for both panes and shown
at most four times per second; input and navigation remain immediate.
Progress and scan completion update values without reordering rows; explicitly
sort again to apply newly completed sizes to the order.
Quick drive/location buttons sit beside the drive dropdown.
The single command row, quiet file hover and completed-partial size feedback
have an updated [verification record](docs/verification/compact-command-and-size-feedback.md).

## Build output and version control

New project build products and diagnostic artifacts share the ignored `_build/`
root. Cargo's `target-dir` configuration applies to build, test, Clippy, generated
Slint Rust, dependency outputs, incremental caches and rustdoc output:

| Directory | Contents | Git |
| --- | --- | --- |
| `_build/` | Default Cargo outputs, including native debug/release executables | Ignored |
| `_build/linux/` | Separate Linux/WSL outputs when sharing the checkout with Windows | Ignored |
| `_build/tmp/` | Temporary compiler/test files when using the build/Cargo helpers | Ignored |
| `_build/qa/` | New test logs, screenshots, diagnostics and disposable experiments | Ignored |
| `.tools/` | Optional existing toolchains/dependency caches/native prerequisites and retained historical QA artifacts | Ignored |
| `.superpowers/` | Retained historical execution/verification evidence | Ignored |
| `crates/`, `api/`, `plugins/`, `scripts/`, `.cargo/`, `.github/`, root build helpers | Handwritten code, independent plugin/API projects, tests, build configuration and CI | Tracked |
| `docs/` | Design, keyboard map and intended documentation images | Tracked |

Cargo writes directly into `_build/`; there is no separate `target/` directory
or compatibility junction. Windows binaries are in `_build/debug/` or
`_build/release/`. Linux/WSL builds sharing this checkout use `_build/linux/`;
standalone Linux checkouts use the default `_build/` layout. Retained historical
QA evidence outside `_build/` stays separate; new diagnostics use `_build/qa/`.

Explicit `CARGO_TARGET_DIR`/`--target-dir` overrides remain supported. Direct Cargo
commands use `_build/` but may use the OS temporary directory; the helpers
scope `TEMP`/`TMP` on Windows or `TMPDIR` on POSIX to `_build/tmp/`. Global rustup,
Cargo download caches and the application's runtime preferences are not build
products and retain their normal locations. Root `build/` and `dist/`, plus
compiled binary/object/library extensions, are ignored as an extra safeguard.
Cargo.lock is committed for reproducible application dependency resolution.

## Structure

| Crate | Responsibility |
| --- | --- |
| `midnight-core` | Domain data, commands/events, provider contract and sorting |
| `midnight-platform` | Target-selected facade; implementation lives in separate OS crates |
| `midnight-platform-common` | Portable safe Rust filesystem helpers |
| `midnight-platform-windows` | Windows-only service implementation and Win32 integration |
| `midnight-platform-linux` | Linux-only service implementation and mount discovery |
| `midnight-engine` | Tokio orchestration, cancellation, bounded workers and batches |
| `midnight-ui` | Slint components, virtualized lazy row model and presenter |
| `midnight-explorer` | Runtime, bounded channels, coalesced UI wakeup and lifecycle |

No filesystem operations or engine implementation are reachable from `midnight-ui`.
Full-list sorting and identity-index construction run off the UI thread. UI
delivery is batched with a short work budget and one outstanding wakeup.

The `midnight-platform` facade only selects the target adapter. `DirectoryService`,
`VolumeService` and `StorageService` are defined in core; their `FileSystem`
aggregate is the engine contract. Each platform implements these interfaces in
its own crate and runs the common conformance tests. Windows/Linux implementation
bodies and OS-specific tests do not share source files.
Optional `DesktopServices` aggregates `IconService`, `TerminalService`,
`PropertyService`, `LocationService` and `OpenService`; native workers implement
these separately from filesystem navigation. Desktop work and size scans have
independent budgets.

## Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p midnight-explorer -p midnight-elevation --locked
cargo run -p midnight-explorer -- --smoke-test
cargo run -p midnight-explorer -- --ui-smoke-test --software
```

During implementation, build the affected test targets once and use the
[targeted parallel runner](docs/developer/TEST_EXECUTION.md) for independent
headless suites. It shares a CPU thread budget across up to four processes,
coalesces filters of the same executable and retains separate logs. Keep
desktop/input and performance checks serial; run full gates after the
implementation batch is coherent.

The application also supports `--snapshot FILE.png` for a native-window smoke
check which saves a screenshot after both real directories have finished loading.
UI smoke checks use fresh in-memory preferences and never load or save the user's
settings. Application error dialogs fail the check instead of producing a passing
screenshot.
Image encoding/writing runs in a backend worker and refuses to overwrite an
existing file. Linux and Windows use the
same source; see `.github/workflows/ci.yml` for automated build/test commands.
CI configuration is not evidence of a completed CI run.

The [code style](CODE_STYLE.md) follows the public Rust Style Guide and Rust API
Guidelines: rustfmt at 100 columns, UTF-8/LF, and a 100,000-byte maximum for
handwritten source files. Run `scripts/check-source-size.ps1` on Windows or
`bash scripts/check-source-size.sh` on Linux; CI runs both platform checks.

## Design and scope

See [the architecture](docs/superpowers/specs/2026-11-04-dual-pane-file-manager-design.md)
and [the stage 1 plan](docs/superpowers/plans/2026-11-04-stage-1-implementation.md).
Mouse activation has a [design](docs/superpowers/specs/2026-11-04-mouse-activation-design.md)
and [verification record](docs/verification/mouse-activation.md).
Compact controls and volume tooltips have a
[design](docs/superpowers/specs/2026-11-04-compact-tooltips-design.md) and
[verification record](docs/verification/compact-tooltips.md).
Salamander sources supplied separately are behavioral reference only; this
implementation does not copy its source, assets or resource strings. The About
dialog includes Slint attribution. A project distribution license has not yet
been selected. macOS adapters and distribution are later work.
The [asset provenance record](docs/ASSET_PROVENANCE.md) distinguishes project-created
artwork from icons supplied at runtime by Windows or installed desktop themes.

Further designs cover [the viewer and virtual filesystems](docs/superpowers/specs/2026-11-04-viewer-vfs-design.md)
and [reusable sudo terminal sessions](docs/superpowers/specs/2026-11-04-terminal-sessions-design.md).
[Checksums on Ctrl+F8](docs/developer/CHECKSUMS.md) use a separate modeless window
with SHA-256 by default, optional SHA-512/MD5/SHA-1, background calculation,
per-algorithm copy and safe manifest export. Ctrl+F7 retains filename case conversion.
The [verification record](docs/verification/2026-11-07-checksum-window.md) distinguishes
tested source from release/native-desktop qualification still in progress.
The [finalized dynamic plugin interface](docs/superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
specifies separately loadable text/source viewers and native, ZIP, GZIP, TAR/TAR.GZ
and 7z filesystem providers under executable-adjacent `viewers/` and `filesystems/`.
The three text/source/Markdown viewer packages use a versioned C ABI and run as
trusted native code. Image viewers and read-only archive packages use separate
versioned executable-worker protocols, with restricted processes for untrusted
parsing. Slint rendering and Tokio
orchestration stay in the host. Missing packages remove the corresponding plugin
features after restart. Native panel browsing, metadata, capacity/storage queries
and native F3 content use separate Windows/Linux filesystem libraries. Other
native operations and watches still use explicit host platform services; the
generic provider migration remains in progress.
Plugins share this Git repository but have independent Cargo workspaces under
`plugins/`, with their own builds, dependencies, tests and CI. They share only the
public API/SDK and must build from an exported plugin directory without host
sources. Plugins cannot depend on private
application crates or modify the main application's sources; API extensions are
separate host-owned changes. Distribution packaging consumes plugin artifacts.
ZIP, 7z, GZIP and TAR/TAR.GZ browsing is implemented with installed archive workers:
Enter/double-click opens an archive, directories and `..` navigate it, saved tabs
retain typed provider addresses, and F3 reads member content through the broker.
Members have opaque identities; duplicate names remain distinct. Missing providers
or changed sources retain unavailable saved tabs with a diagnostic. Mutations,
external editing, passwords, nested archives and RAR are unavailable. F5 copies
members out through the verified background copy queue.
The initial bounded mount permits 64 MiB source/per-member data and 128 MiB total
decoded data; large or high-expansion archives fail explicitly. Full limits and
supported codecs are in the [archive API reference](docs/developer/FILESYSTEM_API.md).
[Archive associations](docs/ARCHIVE_FORMATS.md) include Python wheels and other ZIP
packages. Options → Archives keeps document/media/application containers unchecked
by default; each extension can be enabled explicitly. The entrance-arrow icon marks
enabled native archives only when their provider is installed.
The remaining generic provider migration, live SMB/FUSE qualification and reusable terminal
sessions remain planned. Each OS has a separate native provider implementation.
Copy/move conflicts require a source/destination metadata
comparison and explicit overwrite policy before publication.

### Internal viewers

F3 opens the independent image viewer for PNG, JPEG, GIF, WebP, BMP, ICO and TIFF,
Markdown for `.md`/`.markdown`, source highlighting for the package's
[supported language catalogue](plugins/midnight-viewer-source/README.md), and the basic
text/hex viewer for other files. Alt+F3 always requests the basic plugin's Text mode,
including for binary input. The mode selector changes between modes supported by
the current plugin. Unknown or budget-limited syntax stays readable as plain text.

Each viewer starts at the main window's size. Escape closes it, including from its
search field. Space in the content area advances through the captured panel order,
wrapping after the last regular file. Ctrl+F opens search; F3/Shift+F3 find the next/
previous match. Scrollbars, the wheel, arrows, Page Up/Down and Ctrl+Home/End navigate
without loading the complete file. Drag selects text; Ctrl+A selects the loaded page
and Ctrl+C copies the selection. These shortcuts are configurable in Options → Keyboard.
Text and source modes support word wrap using the Wrap control or Ctrl+W. Binary
files open with byte offsets, hexadecimal bytes and a printable character column;
their columns remain fixed when wrapping is enabled for text.

Images use the same `image` and `moxcms` libraries as image thumbnails, with original
pixels decoded once in a restricted worker. Use Fit, 1:1, +/−, Ctrl+mouse wheel,
Ctrl+Plus/Equal, Ctrl+Minus, Ctrl+0 and Ctrl+1 to zoom. Drag to pan. Animated formats
initially show the first frame. This viewer does not require or enlarge a thumbnail.

Plugins are independent Cargo projects in `plugins/`, with their own tests and locks.
Only built packages under `viewers/<package>/plugin.toml` (text/source/Markdown) or
`viewers/<package>/image-viewer.json` (isolated image/PDF worker) in the resolved installation are
discovered. Options → Plugins lists available/refused packages and lets you disable
or enable them, edit their extension associations and choose precedence for the next
startup. See [viewer file types](docs/VIEWER_ASSOCIATIONS.md). Missing or disabled plugins remove their
capability; the host has no built-in parser fallback. At most four viewer windows
may be open. Reading, parsing, search, layout and rasterization run on backend workers.

Native plugin code is trusted application code, **not sandboxed**. The loader rejects
installations writable or replaceable by untrusted users, including unsafe ancestors.
Build artifacts in a shared checkout may therefore be refused. Deploy the executable
and `viewers/` together into a private user-owned or administrator-owned installation;
do not relax the check or install DLLs from the directory being browsed. On Linux,
directories should normally be `0755` and package files `0644`, with no group/world
write access. On Windows, keep modification rights restricted to your account,
Administrators and SYSTEM. See the [original viewer verification](docs/verification/viewer-implementation.md)
and [text, binary and image controls verification and limits](docs/verification/2026-11-07-viewer-controls.md).

For a Windows checkout on a shared/writable drive, the full helper builds,
deploys and registers a private runnable copy:

```powershell
.\build.bat release
```

The deployment helper prints a fresh installation path under your user profile.
Run `me.exe` from that path or the matching `_build` path. It copies the executable and viewer packages, protects
the new installation's permissions, and refuses unsafe parents or an existing
destination. It never changes the checkout's permissions or launches the app.
This is a runtime installation; compiler and intermediate files remain in `_build`.
Registrations live in `%USERPROFILE%\.midnight-explorer\plugin-runtimes`, keyed by
the host's SHA-256. Protected portable installations still discover their adjacent
plugins directly. See the [resolution and security contract](docs/developer/RUNTIME_DISCOVERY.md).

Options → Editors configures F4 using an absolute executable and a JSON array of
literal arguments. `{file}` must occupy a complete argument; paths are never expanded
through a shell. The platform default and launch errors are shown explicitly. See
[external editor verification](docs/verification/external-editor.md).

The [documentation map](docs/README.md) distinguishes current contracts,
accepted future work and historical verification records.
