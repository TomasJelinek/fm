# Midnight Explorer

Midnight Explorer is a fast, keyboard-friendly two-panel file manager for Windows and
Linux, written in Rust with [Slint](https://slint.dev). It pairs two independent panels
with tabs, internal viewers, verified Copy and Move, archives browsed as folders, Find,
comparison tools and checksums, and keeps every file name's Unicode intact. Viewers,
filesystems and thumbnailers are separate plugin packages that run isolated from the
application.

Current version: **0.6.3**, a preview: features and plugin interfaces are still
changing, archives are read-only, the Windows executable and installer are unsigned and
macOS is not implemented. [Feature status](docs/feature-status.md) lists the current
limits.

## Start Here

- [Download and install](#download-and-install) a release for Windows or Linux.
- [Run it and check that it works](#run-and-check).
- Learn the controls in the [user guide](docs/user-guide.md) and the
  [keyboard shortcuts](docs/keyboard-shortcuts.md); F1 shows the shortcuts in the
  application.
- Fix a problem with [Troubleshooting](docs/troubleshooting.md), or
  [get help](#support).
- [Build from source](#build-from-source).

## Features

The terms are defined in the [glossary](docs/glossary.md).

- **Panels:** two independent [panels](docs/glossary.md#panel) with tabs, history,
  [Hot Paths](docs/glossary.md#hot-paths) (numbered saved locations),
  [inline folder trees](docs/glossary.md#inline-folder-tree), the Brief, Detailed and
  Thumbnails [views](docs/glossary.md#views), the
  [panel filter](docs/glossary.md#panel-filter), selection by mask and folder sizes.
- **Copy and Move** (F5, F6, the clipboard and drag and drop): one background queue
  that reads every copied file back and compares it by SHA-256 by default; Skip, a new
  name, folder Merge or a Replace that keeps the original recoverable; Retry, Skip or
  Cancel on I/O errors; a progress window per job with Pause, and a
  [Jobs](docs/glossary.md#jobs) list; up to 1,000,000 selected items per job.
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
  Neighborhood (the computers and shares of the local network); drive information;
  [Custom Commands](docs/glossary.md#custom-commands), the command line and a terminal
  button.
- **Options:** four application icons, 30 dark and 16 light themes that follow the
  system appearance, density and fonts, filename colors, configurable keyboard
  shortcuts, file-type assignments and settings export and import.

[Feature status](docs/feature-status.md) lists what is implemented on each platform and
where it is verified; planned work and known limits are in the
[implementation backlog](docs/implementation-backlog.md).

## Platforms

| Platform | Status |
| --- | --- |
| Windows 10 22H2 and Windows 11, x64 | Supported; development checks run on Windows 11. |
| Ubuntu 24.04 LTS and Linux Mint 22.x, x86-64 | Supported; X11 and Wayland desktops. |
| Fedora 44+ and openSUSE Leap 16 / Tumbleweed, x86-64 | Compatibility targets; native desktop tests pending. |
| macOS | Not implemented yet (planned). |

Provision at least 4 GB of RAM (8 GB recommended). Versions, runtime libraries, graphics
and filesystem notes are in [system requirements](docs/system-requirements.md).

## Download and install

Preview builds are published on the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases). Choose one:

- **Windows installer** (`midnight-explorer-<version>-x86_64-pc-windows-msvc.msi`): it
  asks whether to install just for you or for all users
  ([Windows installer](docs/user-guide.md#windows-installer)), adds a Start menu entry
  and updates itself from 0.6.3 on.
- **Windows archive** (`midnight-explorer-<version>-x86_64-pc-windows-msvc.zip`) or
  **Linux archive** (`midnight-explorer-<version>-x86_64-unknown-linux-gnu.tar.gz`):
  extract the complete archive into a folder of your own and keep the `viewers`,
  `filesystems` and `thumbnailers` folders beside the executable. Its `README.txt`
  has the details.

On Windows, install Microsoft's
[Visual C++ x64 Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
if `VCRUNTIME140.dll` is missing. On Linux, install the runtime packages that
[system requirements](docs/system-requirements.md#runtime-software) list. Each release
also has `SHA256SUMS` and its signature `SHA256SUMS.minisig` to
[check a download by hand](docs/troubleshooting.md#check-a-download-by-hand).

### Run and check

1. Start `midnight-explorer.exe` (Windows; the installer's Start menu entry does the
   same) or `./midnight-explorer` in a graphical session (Linux). Releases up to 0.5.0
   named the executable `me.exe` and `me`.
2. Check that a window titled `Midnight Explorer 0.6.3 — File Manager` opens with two
   panels listing folders. The application icon at the right end of the top toolbar
   opens **About**, which shows the version too.
3. When the window does not open or the panels stay empty, see
   [Troubleshooting](docs/troubleshooting.md).

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

Debug is the default; `release` builds with full optimization and
[LTO](docs/glossary.md#lto) (link-time optimization). Build output goes to `_build/`
(`_build/debug/` or `_build/release/`, with the executable `midnight-explorer.exe` or
`midnight-explorer` and its `viewers/`, `filesystems/` and `thumbnailers/` folders);
toolchain caches go to `.tools/`. On Windows, `build.bat` also deploys the build to a
[private installation](docs/glossary.md#private-installation-root) that the built
`midnight-explorer.exe` loads its plugins from.

The [user guide](docs/user-guide.md#build-and-run) has the complete build and run
instructions, [release build](docs/release-build.md) describes release settings and
packaging, and the [developer documentation](docs/developer/README.md) covers the
plugin APIs and SDKs, with [test execution](docs/developer/test-execution.md) for the
checks.

## Documentation

- [Documentation map](docs/README.md): every current contract and reference.
- [User guide](docs/user-guide.md): running the application, controls, viewers,
  operations and settings.
- [Keyboard shortcuts](docs/keyboard-shortcuts.md), [settings files](docs/settings-files.md)
  and [viewer associations](docs/viewer-associations.md).
- [Troubleshooting](docs/troubleshooting.md) and the [glossary](docs/glossary.md).
- [Feature status](docs/feature-status.md) and
  [implementation backlog](docs/implementation-backlog.md).
- [System requirements](docs/system-requirements.md) and
  [release build](docs/release-build.md).
- [Security and integrity rules](SECURITY.md).
- [Developer and plugin API documentation](docs/developer/README.md).
- [Third-party libraries and licenses](docs/third-party.md) and the
  [dependency policy](docs/dependency-policy.md).

## Releases

- **Artifacts:** every release on the
  [GitHub Releases page](https://github.com/TomasJelinek/fm/releases) has the Windows
  ZIP and, from 0.6.1, the MSI; the Linux tar.gz; and `SHA256SUMS`, signed from 0.5.1
  on in `SHA256SUMS.minisig`. The archives include every plugin, the dependency notices
  and the license. All releases so far are previews (GitHub pre-releases).
- **Release notes:** [0.1.0](docs/releases/0.1.0.md), [0.5.0](docs/releases/0.5.0.md),
  [0.5.1](docs/releases/0.5.1.md), [0.5.2](docs/releases/0.5.2.md),
  [0.5.3](docs/releases/0.5.3.md), [0.5.4](docs/releases/0.5.4.md),
  [0.6.0](docs/releases/0.6.0.md), [0.6.1](docs/releases/0.6.1.md),
  [0.6.2](docs/releases/0.6.2.md) and [0.6.3](docs/releases/0.6.3.md).
- **Compatibility:** Windows 10 22H2 and Windows 11 on x64; Linux x86-64 with the
  Ubuntu 24.04 / glibc 2.39 baseline ([Platforms](#platforms)). Plugins built on the
  viewer SDK 0.1.0, 0.2.0 or 0.2.1 keep loading
  ([developer guide](docs/developer/README.md)).
- **Updating:** from 0.5.1 on, an extracted copy that your account can change checks
  for, verifies and installs signed updates itself, and so does an MSI installation
  from 0.6.3 on; a 0.6.1 MSI installation is updated by running the newer MSI once
  ([Updates](docs/user-guide.md#updates)). Settings are kept. 0.6.2's MSI does not
  install; use a later one.
- **Source:** the source of each release is its tagged commit in this repository.

## Support

- **Questions and problems:** read [Troubleshooting](docs/troubleshooting.md) first, then
  search or open an issue in the
  [GitHub issue tracker](https://github.com/TomasJelinek/fm/issues). Include the version,
  how you installed it, the operating system and the message text
  ([what to include](docs/troubleshooting.md#get-help)).
- **Security:** [SECURITY.md](SECURITY.md) states the security and integrity
  requirements. Do not publish the details of a suspected vulnerability in a public
  issue.

## Contributing

Read the [project rules](AGENTS.md), the [code style](CODE_STYLE.md) and the
[security rules](SECURITY.md) before changing code. Third-party dependencies must be
free of mandatory fees and follow the [dependency policy](docs/dependency-policy.md).

## License

Midnight Explorer is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software Foundation,
either version 3 of the License, or (at your option) any later version
(`GPL-3.0-or-later`). It is distributed WITHOUT ANY WARRANTY; see [LICENSE](LICENSE)
for the full text.

The plugins under [plugins/](plugins/) and the public API/SDK workspaces are licensed
under the same terms, GPL-3.0-or-later. Third-party components keep
their own licenses; they are listed in [third-party.md](docs/third-party.md), and
release archives carry their notices. The source of each release is its tagged commit
in this repository.

Copyright © 2026 Tomas Jelinek.

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
