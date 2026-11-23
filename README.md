# Midnight Explorer

Midnight Explorer is a fast, keyboard-friendly dual-pane file manager for Windows and
Linux, written in Rust with [Slint](https://slint.dev). It pairs two independent panels
with tabs, internal viewers, verified Copy and Move, archives browsed as folders, Find,
comparison tools and checksums, and keeps every file name's Unicode intact. Viewers,
filesystems and thumbnailers are separate plugin packages that run isolated from the
application.

Current version: **0.5.1** (preview).

## Features

- **Panels:** two independent panels with tabs, history, Hot Paths, inline folder
  trees, Brief, Detailed and Thumbnails views, the panel filter, selection by mask and
  folder sizes.
- **Copy and Move** (F5, F6, the clipboard and drag and drop): one background queue
  that reads every copied file back and compares it by SHA-256 by default; Skip, a new
  name, folder Merge or a Replace that keeps the original recoverable; Retry, Skip or
  Cancel on I/O errors; a progress window per job with Pause, and a Jobs list; up to
  1,000,000 selected items per job.
- **Delete, create and rename:** Delete to the system Trash or permanently, new folders
  and files, rename, attributes, times and case.
- **Viewers (F3):** text and hex, source code with syntax highlighting, Markdown,
  images (PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, SVG, AVIF), PDF, HTML in sandboxed
  Chromium, previews of Office documents (DOCX, XLSX, PPTX, ODT, ODS, ODP) and video
  playback.
- **Find** (Ctrl+F or Alt+F7): names by masks, regular expressions or metadata
  expressions, text, modification time and identical files, also inside archives.
- **Archives:** ZIP, 7z, GZIP, XZ, Bzip2, Zstandard and TAR, browsed read-only as
  folders, with passwords for ZIP and 7z and nested archives.
- **Tools:** file comparison (text, images, hex) and directory comparison; checksums
  (SHA-256, SHA-512, SHA-1, MD5, CRC32) with a Verify page; Make File List; Network
  Neighborhood; drive information; Custom Commands, the command line and a terminal
  button.
- **Options:** 26 themes, density and fonts, filename colours, configurable keyboard
  shortcuts, file-type assignments and settings export and import.

[Feature status](docs/FEATURE_STATUS.md) lists what is implemented on each platform and
where it is verified; planned work and known limits are in the
[implementation backlog](docs/IMPLEMENTATION_BACKLOG.md).

## Platforms

| Platform | Status |
| --- | --- |
| Windows 10 22H2 and Windows 11, x64 | Supported; development checks run on Windows 11. |
| Ubuntu 24.04 LTS and Linux Mint 22.x, x86-64 | Supported; X11 and Wayland desktops. |
| Fedora 44+ and openSUSE Leap 16 / Tumbleweed, x86-64 | Compatibility targets; native desktop tests pending. |
| macOS | Not implemented yet (planned). |

Provision at least 4 GB of RAM (8 GB recommended). Versions, runtime libraries, graphics
and filesystem notes are in [system requirements](docs/SYSTEM_REQUIREMENTS.md).

## Download

Preview builds for Windows x64 (`.zip`) and Linux x86-64 (`.tar.gz`) are published on
the [GitHub Releases page](https://github.com/TomasJelinek/fm/releases), each with a
`SHA256SUMS` file to verify the download. Extract the complete archive and run
`midnight-explorer.exe` (Windows) or `./midnight-explorer` (Linux); its `README.txt`
has the details. Releases up to 0.5.0 named the executable `me.exe` and `me`. The
Windows executable is unsigned and there is no installer; from 0.5.1 on the application
checks for, verifies and installs signed updates itself. The archives include every
plugin, the dependency notices and the license; the release notes say what each
preview contains ([0.5.0](docs/releases/0.5.0.md), [0.5.1](docs/releases/0.5.1.md)).

## Build from source

Prerequisites:

- Rust **1.99.1** with rustfmt and Clippy, as pinned in
  [rust-toolchain.toml](rust-toolchain.toml); install it with rustup.
- **Windows:** the Visual Studio *Desktop development with C++* workload (MSVC x64 and a
  Windows SDK), PowerShell 7 and Python 3.12 or later.
- **Linux:** a C compiler, `pkg-config` and the desktop development libraries
  ([scripts/setup-linux.sh](scripts/setup-linux.sh) prints or installs them) and
  Python 3.11 or later; or Docker, which `build.sh` uses when native prerequisites are
  missing.

Build the application together with every plugin package from the repository root:

```sh
build.bat              # Windows, debug
build.bat release      # Windows, optimized release
./build.sh             # Linux, debug
./build.sh release     # Linux, optimized release
```

Debug is the default; `release` builds with full optimization and LTO. Build output
goes to `_build/` (`_build/debug/` or `_build/release/`, with the executable
`midnight-explorer.exe` or `midnight-explorer` and its `viewers/`, `filesystems/` and
`thumbnailers/` folders); toolchain caches go to `.tools/`. On Windows, `build.bat` also deploys the build to a private
installation that the built `midnight-explorer.exe` loads its plugins from.

The [user guide](docs/USER_GUIDE.md#build-and-run) has the complete build and run
instructions, [release build](docs/RELEASE_BUILD.md) describes release settings and
packaging, and the [developer documentation](docs/developer/README.md) covers the
plugin APIs and SDKs, with [test execution](docs/developer/TEST_EXECUTION.md) for the
checks.

## Usage

- [User guide](docs/USER_GUIDE.md): running the application, controls, viewers,
  operations and settings.
- [Keyboard shortcuts](docs/KEYBOARD_SHORTCUTS.md); F1 shows them in the application.
- [Settings files](docs/SETTINGS_FILES.md) and
  [viewer associations](docs/VIEWER_ASSOCIATIONS.md).

## Documentation

- [Documentation map](docs/README.md): every current contract and reference.
- [Feature status](docs/FEATURE_STATUS.md) and
  [implementation backlog](docs/IMPLEMENTATION_BACKLOG.md).
- [System requirements](docs/SYSTEM_REQUIREMENTS.md) and
  [release build](docs/RELEASE_BUILD.md).
- [Security and integrity rules](SECURITY.md).
- [Developer and plugin API documentation](docs/developer/README.md).
- [Third-party libraries and licenses](docs/THIRD_PARTY.md) and the
  [dependency policy](docs/DEPENDENCY_POLICY.md).

## Project status

Midnight Explorer is a preview: features and plugin interfaces are still changing,
archives are read-only, the Windows executable is unsigned and macOS is not
implemented. See [feature status](docs/FEATURE_STATUS.md) for the current limits.

## Contributing

Read the [project rules](AGENTS.md), the [code style](CODE_STYLE.md) and the
[security rules](SECURITY.md) before changing code. Third-party dependencies must be
free of mandatory fees and follow the [dependency policy](docs/DEPENDENCY_POLICY.md).

## License

Midnight Explorer is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software Foundation,
either version 3 of the License, or (at your option) any later version
(`GPL-3.0-or-later`). It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE)
for the full text.

The plugins under [plugins/](plugins/) and the public API/SDK workspaces are licensed
under the same terms, GPL-3.0-or-later. Third-party components keep
their own licenses; they are listed in [THIRD_PARTY.md](docs/THIRD_PARTY.md), and
release archives carry their notices. The source of each release is its tagged commit
in this repository.

Copyright © 2026 Tomas Jelinek.

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
