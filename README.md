# Midnight Explorer

A dual-pane desktop file manager written in Rust with Slint and Tokio.

Product version **0.1** (Cargo SemVer `0.1.0`); the native window title shows
`Midnight Explorer 0.1 — File Manager`. The displayed version is derived from the package version.

Lossless Unicode throughout the product, security and backend-only non-UI work
are explicit requirements. See [security and integrity rules](SECURITY.md).

Stage 1 provides independent local directory navigation, mounted-volume switching,
history, sorting, selection, mouse/keyboard navigation, and background metadata/capacity
queries, native watches, a command editor and appearance preferences. Internal
file operations include confirmed rename, recursive directory creation, empty-file
creation (toolbar or Shift+F4), and queued deletion (system Trash by default,
permanent deletion with Shift). Empty-file creation accepts a name and never
overwrites an existing entry.
Explicitly opened files use host associations,
and explicitly submitted shell commands run with
the selected user/sudo permissions. Viewer plugins and configurable external editing
are the next stage. Ctrl+C/Ctrl+V copies files and ordinary folders through a
background queue with independent SHA-256 destination readback. Existing names
prompt for skip or a different name; overwrite, merge, Cut/Move, links and full
metadata preservation are not implemented. See the platform capability limits in
[the copy contract](SECURITY.md#verified-clipboard-copying-and-the-copy-contract).

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

Install [Rust with rustup](https://rust-lang.org/tools/install/), then reopen your
terminal so `cargo` and `rustup` are on `PATH`. The repository pins **Rust 1.99.1**
with rustfmt and Clippy in [rust-toolchain.toml](rust-toolchain.toml). From the
repository root, `rustup show` selects/downloads that toolchain. Cargo downloads
Slint and the other locked dependencies on the first build; no separate Slint SDK
installation is needed. The first build takes longer than subsequent builds.

The commands below assume the current directory is the repository root. The
optional root build helpers locate the repository themselves, so they also work
when invoked by a quoted path from another directory. They accept no argument
(debug), `debug`, or `release`, run a locked build of `midnight-explorer`, and return Cargo's
exit status. They do not install system packages or launch the application.
The Cargo package remains `midnight-explorer`; its executable is `me.exe` on Windows
and `me` on Linux.
Output paths below assume the default native target and no `CARGO_TARGET_DIR`
override. Cargo's checked-in configuration puts compiled and generated output
under `_build/`. The build helpers also keep their child processes'
temporary files under `_build/tmp/`.

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

Equivalent commands with a normal Rust installation:

```powershell
cargo build -p midnight-explorer --locked
cargo build -p midnight-explorer --locked --release
```

The default build is debug. The frame rendering libraries use optimization level 2
even in debug, and generated/UI code uses level 1. Backend/application code retains
normal debug behavior. Release still optimizes the full application. The first build after
changing these profiles recompiles the affected dependencies under `_build`.

Run either executable directly, or let Cargo build and run it:

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

Install the native compiler/linker and development libraries used by the current
Linux build. For Ubuntu/Mint this matches CI prerequisites plus `build-essential`:

```sh
sudo apt update
sudo apt install build-essential pkg-config libfontconfig1-dev libfreetype6-dev \
    libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libxcb1-dev libwayland-dev libudev-dev
```

Fedora 44+ (maintained desktop release, x86-64):

```sh
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config fontconfig-devel freetype-devel \
    libxkbcommon-devel libxkbcommon-x11-devel libX11-devel libXcursor-devel \
    libxcb-devel wayland-devel systemd-devel xdg-utils
```

openSUSE Leap 16.x / Tumbleweed (x86-64):

```sh
sudo zypper install gcc gcc-c++ make pkg-config xdg-utils \
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

Install Rust using the [official rustup instructions](https://rust-lang.org/tools/install/),
then open a new terminal or load rustup's environment with `. "$HOME/.cargo/env"`.
Build using the POSIX helper or Cargo:

```sh
./build.sh
./build.sh release

cargo build -p midnight-explorer --locked
cargo build -p midnight-explorer --locked --release
```

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

Normal rendering uses Slint FemtoVG-WGPU with system-font fallback. Windows defaults
to Direct3D; explicit `SLINT_BACKEND` / `WGPU_BACKEND` overrides remain available.
The separate
software renderer is a developer diagnostic option; it is not a qualified
full-Unicode production fallback:

```sh
cargo run -p midnight-explorer --locked -- --software
```

`SLINT_BACKEND` is respected when `--software` is not supplied. GUI smoke checks
and `--snapshot FILE.png` need a working display; see [Checks](#checks) below.
On an isolated Linux display without a suitable GPU, Slint may fall back to
software, which does not render bitmap emoji. For WGPU diagnostics only,
`SLINT_WGPU_CPU=1 ./_build/debug/me` permits a CPU adapter. This still needs the
graphics runtime and fonts; it does not establish production GPU performance.

## Controls

See the [complete current keyboard map](docs/KEYBOARD_SHORTCUTS.md), including
editor scope and shortcuts reserved for later stages.

- Ctrl+A / Ctrl+D / Ctrl+I: select all / clear / invert selection in the focused
  file pane. The parent `..` is excluded; text editors retain normal editing.
- Type in the file pane to search filename prefixes, always Unicode
  case-insensitively. The matching prefix is colored; Up/Down visits matches and
  Escape exits. Options can filter the list while typing instead of just jumping.

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
- Hot Paths dropdown: open or manage saved paths and numbered assignments;
  saved paths also appear in the full drive dropdown. Ctrl+number or Shift+number
  opens a numbered path (Alt targets the other pane); Ctrl+Shift+0 assigns slot 10.
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
- Ctrl+Shift+S: calculate all folder sizes in the active pane on the backend;
  press again to cancel. Partial/running values are marked; links are skipped.
- Ctrl+[ / Ctrl+]: insert the left/right path as a literal shell argument.
  Simple paths remain bare; special characters use escaped double quotes.
- Ctrl+P: insert the active path; Ctrl+Enter: append the focused filename at
  the end of the command draft. Insertion never submits the command.
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
  all discovered drives and special locations, with full-path tooltips.
  Suitable host icons load on the backend; missing native action icons retain
  semantic application vectors and native colors remain intact.
- Options after F12 has Panel, Appearance, Command and Keyboard tabs. It offers readable
  or exact-byte sizes, history persistence/Clear, Blue, Dark, Light, Dracula, Nord,
  Gruvbox Dark, Catppuccin Mocha, Solarized Light, Desktop/Touch spacing, an installed
  font family and independent Desktop/Touch base sizes (8–32px). Preview
  is immediate; typed preferences are saved asynchronously by the backend.
  Desktop uses denser rows/toolbars; compact Touch uses 40px base targets. The active folder
  appears in full beside the command input in the same single row, without
  elision or a fixed path-width cap. Extreme paths remain fully accessible in a
  horizontal viewport while preserving editor space.
  The command row uses the same height as the function-button toolbar. Alt+Up
  and Alt+Down rotate stored commands without changing the chosen terminal;
  Enter submits the displayed draft.
  At large fonts, the window minimum grows to preserve readable controls and
  a complete file row; ordinary fonts retain the 480 px minimum-height floor.
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
- F2: rename the focused item; F7: create a folder or nested relative path.
- Shift+F4 or the panel's **New empty file** button: enter a filename and press
  Enter to create a zero-byte file. Escape cancels; existing entries are never
  overwritten. The new file is focused when creation finishes.
- Delete/F8: confirm moving selected items to Trash. Shift+Delete/Shift+F8:
  explicitly confirm permanent deletion. A Trash failure never permanently deletes.
- F11/F12: Windows system dialogs for connecting/disconnecting network drives.
  Options follows F12; Jobs shows the operation queue, progress and cancellation.
- Hold Ctrl to show the effective Ctrl+F1–F12 captions and actions in the footer.
  Remapped or disabled shortcuts are reflected in this layer. Ctrl+F11 expands
  the active panel and leaves a rail for switching to the other panel; another
  Ctrl+F11 restores the previous split. Double-clicking the splitter centers it.
- Ctrl+F10 compares the two displayed directories on the backend, selecting
  missing, larger or newer files. Size and modified time are enabled by default,
  with a two-second timestamp tolerance. Name-case matching and unmatched
  directories are configurable. Matching metadata does **not** verify file contents;
  content and recursive comparison are planned extensions.
- Ctrl+F12 opens a saved per-panel file filter. Choose wildcard masks or a Rust
  regular expression, with explicit case sensitivity. Directories remain visible;
  the filter combines with typing-based filename search. Invalid expressions do
  not replace the active filter.
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
  Actual image/video preview decoding is planned in the
  [thumbnail design](docs/superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).
- Click column headers: change sorting. Cursor and selection remain attached to
  the original raw filesystem paths when the listing order changes.
  Repeated clicks toggle ascending/descending order. Completed folder sizes are
  retained and sorted on the backend; Refresh invalidates these live estimates.

Application hotkeys can be changed, disabled, or restored in **Options → Keyboard**.
Bindings are validated for conflicts in their applicable contexts; native text editing
and IME behavior are protected. The [shortcut reference](docs/KEYBOARD_SHORTCUTS.md)
lists defaults. Numbered tab and Hot Path bindings use physical digits across layouts.

Options, tabs, bookmarks, panel filters, enabled command history and shortcut overrides are stored
as version 8 UTF-8 JSON in the native
`%LOCALAPPDATA%\Midnight Explorer\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/midnight-explorer/preferences.json` /
`~/.config/midnight-explorer/preferences.json` on Linux. When the new file is absent,
the former `FM` (Windows) / `fm` (Linux) location is read safely; saves use only the
new location and preserve the old file.
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
will discover available devices/shares; discovery is designed, not implemented yet.
The [panel and scrolling verification](docs/verification/panel-navigation-and-smooth-scrolling.md)
records regression coverage, native checks and the clean build layout.
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
| `crates/`, `scripts/`, `.cargo/`, `.github/`, root `build.bat` / `build.sh` | Handwritten code, tests, build helpers/configuration and CI | Tracked |
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
cargo build -p midnight-explorer --locked
cargo run -p midnight-explorer -- --smoke-test
cargo run -p midnight-explorer -- --ui-smoke-test --software
```

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

Further designs cover [the viewer and virtual filesystems](docs/superpowers/specs/2026-11-04-viewer-vfs-design.md)
and [reusable sudo terminal sessions](docs/superpowers/specs/2026-11-04-terminal-sessions-design.md).
The [finalized dynamic plugin interface](docs/superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
specifies separately loadable text/source viewers and native, ZIP, GZIP, TAR/TAR.GZ
and 7z filesystem providers under executable-adjacent `viewers/` and `filesystems/`.
Missing packages remove their features after restart, including native browsing
when the platform's native provider is absent. Plugins use a versioned C ABI;
Slint rendering and Tokio orchestration stay in the host. Native plugins are
trusted application code, not sandboxed extensions. Initial archives are read-only.
Plugins share this Git repository but have independent Cargo workspaces under
`plugins/`, with their own builds, dependencies, tests and CI. They share only the
public API/SDK and must build from an exported plugin directory without host
sources. Plugins cannot depend on private
application crates or modify the main application's sources; API extensions are
separate host-owned changes. Distribution packaging consumes plugin artifacts.
These are designs, not enabled viewer/VFS/session-reuse implementations. Each OS
will have its own native provider; archives and typed Registry values use the
same provider boundary. Copy/move conflicts require a source/destination metadata
comparison and explicit overwrite policy before publication.
