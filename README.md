# FM

A dual-pane desktop file manager written in Rust with Slint and Tokio.

Product version **0.1** (Cargo SemVer `0.1.0`); the native window title shows
`FM 0.1 — File Manager`. The displayed version is derived from the package version.

Lossless Unicode throughout the product, security and backend-only non-UI work
are explicit requirements. See [security and integrity rules](SECURITY.md).

Stage 1 provides independent local directory navigation, mounted-volume switching,
history, sorting, selection, mouse/keyboard navigation, and background metadata/capacity
queries, native watches, a command editor and appearance preferences. Internal
file operations remain read-only; explicitly opened files use host associations,
and explicitly submitted shell commands run with
the selected user/sudo permissions. Copying with mandatory BLAKE3 destination verification
is the next milestone; no unverified copy operation is exposed.

## Run on Windows x64

Install Rust with [rustup](https://rust-lang.org/tools/install/) and Visual Studio
C++ build tools with a Windows SDK. This workspace also supports the local
toolchain under `.tools` used during development, without modifying system PATH:

```powershell
.\scripts\cargo.ps1 run -p fm-app
.\scripts\cargo.ps1 run -p fm-app '--' --left D:\experiments --right C:\Users
```

With a regular Rust installation, use `cargo run -p fm-app`. For a release build:

```powershell
.\scripts\cargo.ps1 build --release -p fm-app
.\target\release\fm-app.exe
```

Normal rendering uses Slint FemtoVG-WGPU with system-font fallback. The separate
software renderer is a developer diagnostic option; Slint documents script
limitations, so it is not a qualified full-Unicode production fallback:

```powershell
.\scripts\cargo.ps1 run -p fm-app '--' --software
```

`SLINT_BACKEND` is respected when `--software` is not supplied. First builds
download and compile Slint's dependencies and take substantially longer than
later builds. The pinned toolchain is specified in `rust-toolchain.toml`.
When forwarding application arguments through the PowerShell helper, quote the
literal `'--'` separator as shown above; regular `cargo.exe` commands use `--`.

## Linux / Ubuntu / Mint

Install Rust with rustup and the distribution's native build prerequisites.
For Ubuntu/Mint, the development package set is:

```sh
sudo apt install build-essential pkg-config libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libx11-dev libxcb1-dev libwayland-dev libudev-dev
cargo run -p fm-app
```

Linux supports already-mounted filesystems via `/proc/self/mountinfo`; mounting
or safely ejecting devices is not part of this milestone. Desktop smoke checks
require a working X11 or Wayland display. Software rendering is available with
`cargo run -p fm-app -- --software`.

## Controls

See the [complete current keyboard map](docs/KEYBOARD_SHORTCUTS.md), including
editor scope and shortcuts reserved for later stages.

- Ctrl+A / Ctrl+D / Ctrl+I: select all / clear / invert selection in the focused
  file pane. The parent `..` is excluded; text editors retain normal editing.

- Tab: switch active file pane.
- Ctrl+T / Ctrl+W: add / close a folder tab in the focused pane.
- Ctrl+Tab / Ctrl+Shift+Tab: next / previous folder tab. Each caption follows
  the current folder name; full paths are in tooltips. Tabs are session-only.
- Star button: add, open or remove named saved paths. Ctrl+Shift+1–9/0 assigns
  a numbered path; Ctrl+number or Shift+number opens it (Alt targets the other pane).
- Left click: focus the row and its pane; Ctrl+left click toggles selection.
- Enter / unmodified left double click: enter a directory or open a file in its
  default system application. Directory links stay inside the file manager.
- Backspace / toolbar Up: parent directory.
- Alt+Left / Alt+Right: back / forward history.
- Arrow keys, Page Up/Down, Home/End: move the cursor.
- Space: toggle selection; Insert: toggle selection and advance.
- Ctrl+R / toolbar Refresh: reload the current directory.
- Ctrl+Shift+S: calculate all folder sizes in the active pane on the backend;
  press again to cancel. Partial/running values are marked; links are skipped.
- Ctrl+[ / Ctrl+]: insert the left/right path as a quoted shell token.
- Ctrl+P: insert the active path; Ctrl+Enter: append the focused filename at
  the end of the command draft. Insertion never submits the command.
- Alt+F1 / Alt+F2: open left / right drive choices. Ctrl+F3/F4/F5/F6 sort by
  name/type/time/size; Ctrl+F9 refreshes; Ctrl+Shift+F10 calculates folder sizes.
- Enter in the command editor: open the system terminal in the active directory
  and run the submitted command. Insertion alone never executes a command.
- The picker left of the editor chooses PowerShell/cmd on Windows or
  Terminal/Terminal(sudo) on Linux. A nonempty draft stays bound to its grammar;
  clear it before switching PowerShell/cmd. Sudo selection is session-only.
- The compact terminal icon and navigation controls explain themselves through
  tooltips. Drive tooltips show free space and Windows volume names; expanded
  drive choices include those details. Metadata loads on the backend.
- Options beside Run offers Blue, Dark, Light, Desktop/Touch spacing, an installed
  font family and independent Desktop/Touch base sizes (8–32px). Preview
  is immediate; typed preferences are saved asynchronously by the backend.
  Desktop uses denser rows/toolbars; compact Touch uses 40px base targets. The active folder
  appears as a compact read-only prompt beside the command input.
- Panel scrolling moves by pixels and gently settles on a row after input and
  inertia stop. Native drive icons precede captions and dropdown choices.

- Alt+Enter: properties of selected items or the focused file/folder.
- Shift+drive letter: switch the focused file pane on Windows.
- F1–F10: commander bottom bar; unavailable file-writing/view/edit actions are
  disabled until their implementation stage.
- Click column headers: change sorting. Cursor and selection remain attached to
  the original raw filesystem paths when the listing order changes.
  Repeated clicks toggle ascending/descending order. Completed folder sizes are
  retained and sorted on the backend; Refresh invalidates these live estimates.

Options and bookmarks are stored as version3 UTF-8 JSON in the native
`%LOCALAPPDATA%\FM\preferences.json` on Windows, or
`$XDG_CONFIG_HOME/fm/preferences.json` / `~/.config/fm/preferences.json` on Linux.
Storage and font/icon discovery run on backend workers. See the
[navigation and appearance design](docs/superpowers/specs/2026-11-04-navigation-and-appearance-design.md).

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
Non-final totals retain a contrasting color, `≈` and tooltips without an
exclamation prefix. Size-only UI updates are combined for both panes and shown
at most four times per second; input and navigation remain immediate.
Progress and scan completion update values without reordering rows; explicitly
sort again to apply newly completed sizes to the order.
Quick drive/location buttons sit beside the drive dropdown.

## Build output and version control

Build products are not source files and are ignored by Git:

| Directory | Contents | Git |
| --- | --- | --- |
| `target/` | Standard Cargo outputs, including Windows debug/release executables | Ignored |
| `.tools/linux-target/` | Local WSL/Linux Cargo outputs used for development | Ignored |
| `.tools/` | Local toolchains, dependency caches, native prerequisites, helper scripts and logs | Ignored |
| `.superpowers/` | Local execution/verification evidence | Ignored |
| `crates/`, `scripts/`, `.cargo/`, `.github/` | Handwritten code, tests, portable build configuration and CI | Tracked |
| `docs/` | Design, keyboard map and intended documentation images | Tracked |

There is no required `_build/` directory. Root `_build/`, `build/` and `dist/`
are also ignored if introduced later. Compiled binary/object/library extensions
are excluded as a second safeguard. Cargo.lock is committed for reproducible
application dependency resolution; temporary QA captures are excluded.

## Structure

| Crate | Responsibility |
| --- | --- |
| `fm-core` | Domain data, commands/events, provider contract and sorting |
| `fm-platform` | Target-selected facade; implementation lives in separate OS crates |
| `fm-platform-common` | Portable safe Rust filesystem helpers |
| `fm-platform-windows` | Windows-only service implementation and Win32 integration |
| `fm-platform-linux` | Linux-only service implementation and mount discovery |
| `fm-engine` | Tokio orchestration, cancellation, bounded workers and batches |
| `fm-ui` | Slint components, virtualized lazy row model and presenter |
| `fm-app` | Runtime, bounded channels, coalesced UI wakeup and lifecycle |

No filesystem operations or engine implementation are reachable from `fm-ui`.
Full-list sorting and identity-index construction run off the UI thread. UI
delivery is batched with a short work budget and one outstanding wakeup.

The `fm-platform` facade only selects the target adapter. `DirectoryService`,
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
cargo build -p fm-app --locked
cargo run -p fm-app -- --smoke-test
cargo run -p fm-app -- --ui-smoke-test --software
```

The application also supports `--snapshot FILE.png` for a native-window smoke
check which saves a screenshot after both real directories have finished loading.
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
These are designs, not enabled viewer/VFS/session-reuse implementations. Each OS
will have its own native provider; archives and typed Registry values use the
same provider boundary. Copy/move conflicts require a source/destination metadata
comparison and explicit overwrite policy before publication.
