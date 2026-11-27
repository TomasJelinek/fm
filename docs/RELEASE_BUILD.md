# Midnight Explorer release builds

The application package `midnight-explorer` builds `midnight-explorer.exe` on Windows
and `midnight-explorer` on Linux. Release output is `_build/release/midnight-explorer.exe`
or `_build/release/midnight-explorer`. Releases up to 0.5.0 named the executable
`me.exe` and `me`; the packager refuses a binary under any name but the current one.

Use **debug and test builds for ordinary development**. `build.bat` and `build.sh`
default to debug, producing `_build/debug/midnight-explorer.exe` or
`_build/debug/midnight-explorer`. For a focused host-code change, build only
`midnight-explorer` with Cargo, and rebuild plugins when their own code or API changes. Plugin trust and deployment requirements apply in debug
too. Build release for final integration and packaging or for a specific performance
comparison, not after every UI edit: fat [LTO](GLOSSARY.md#lto) (link-time
optimization across the whole program) and one code-generation unit trade build time
for the published binary.

OS baselines, runtime dependencies and build RAM and storage allowances are in the
[system requirements](SYSTEM_REQUIREMENTS.md). Test a release on those baselines: a
successful build on a newer host does not prove compatibility with an older OS, glibc
or graphics driver.

## Release profile

The workspace release profile (`[profile.release]` in `Cargo.toml`):

- `opt-level = 3`, whole-program **fat LTO**, one code-generation unit and no
  incremental compilation;
- no debug information, stripped symbols and no debug assertions;
- integer overflow checks stay enabled, and `panic = "unwind"` keeps the backend's
  worker-panic reporting;
- the generated main-window bindings (`midnight-ui-bindings`) alone use
  `opt-level = "s"`; runtime services, renderers, text engines and the other
  application crates keep level 3. Build-time host tools keep Cargo's normal build
  overrides.

Each independently built viewer, thumbnailer and filesystem plugin also strips its
release binaries. The Linux HTML packager verifies the pinned [CEF](GLOSSARY.md#cef)
(Chromium Embedded Framework) SDK and runs GNU `strip --strip-unneeded` on staged
copies of its four ELF binaries before writing the runtime inventory. The final ZIP
and tarball check rejects debug sidecars, embedded debug information and full symbol
tables across the application and its [workers](GLOSSARY.md#worker). Dynamic exports,
unwind data, build IDs and references to external PDBs support loading and
diagnostics and may be present in a release image.
The [0.6.0 payload checks](verification/2026-11-25-release-payload.md) record
the measured changes and native Chromium rendering results.

A same-source Windows comparison of this profile reduced the executable from 42,425,856 to
37,096,448 bytes (12.56%), with overlapping native resize and redraw timings across six
alternating runs per profile
([release optimization verification](verification/2026-11-07-release-optimization.md));
it was not repeated on Linux.

File integrity, bounds checks, native identity validation and explicit confirmation
are required behavior in every profile; there are no unsafe floating-point rewrites or
unchecked data-access shortcuts. The executable targets the platform's portable CPU
baseline. Do not set `target-cpu=native` globally: a redistributed binary would then
depend on the builder's processor. A machine-specific build for known hardware must be
labeled and tested separately. Profile-guided optimization (PGO) is not used; it would
need representative browsing, scrolling, search and operation workloads to train on.

References: [Cargo profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
and the [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html).

## Building a release

1. Run `build.bat release` (Windows) or `./build.sh release` (Linux), which build with
   Cargo's `--release`.
   - Adding `--validate` also runs CI's tests for the platform and the installed smoke
     test of that release build
     ([full validation](developer/TEST_EXECUTION.md#full-validation)).
2. Smoke-test the executable in `_build/release/`.

The helpers build the host and its [authorization helper](GLOSSARY.md#elevated-listing-helper)
(`midnight-elevation`), then
the platform's native and network filesystem packages, the six viewer packages, both
thumbnail workers and the archive worker against the public APIs and SDKs, and stage
them in `filesystems/`, `viewers/` and `thumbnailers/` beside the executable. Every
module is required. The text, source and Markdown viewers and the filesystem libraries
use the native C ABI; the image and PDF viewers use the restricted worker protocol of
the separate worker SDK; the HTML viewer is a web-worker package with its CEF runtime
and Chromium's sandbox.

Direct Cargo builds compile only what they name. Build the modules separately with
`scripts/build-native-filesystem.ps1` or `.sh`, `scripts/build-network-filesystem.ps1`
or `.sh`, `scripts/build-viewers.ps1` or `.sh`,
`scripts/build-default-thumbnailers.ps1` or `.sh` and
`scripts/build-archive-filesystem.ps1` or `.sh`. Each rebuild replaces a package's
generated notices, and the thumbnail folder as a whole, so files of a dependency or
FFmpeg version that is no longer shipped never reach a deployment or archive. The
native helper's test and offline options are in the
[native filesystem build instructions](developer/NATIVE_FILESYSTEM_API.md#building-the-native-package);
the network helper has the same options, and the
[network filesystem plugin README](../plugins/midnight-filesystem-network/README.md)
covers its Linux system runtime. Both use the shared filesystem package helper, which
stages all inputs before it replaces an installed library. Each plugin has its own
lockfile, tests, release profile and third-party notices.

CI and full validation run the workspace tests in the debug test profile (the plugin
jobs use their own profiles) and, against a release build, the installed filesystem
smoke test. CI's `release` job builds the release host and helper, checks this run's
filesystem packages with `scripts/package-release.py validate-filesystems` and runs
that smoke test; it does not package or publish. Release builds take longer and may
use more RAM because of whole-program optimization.

### Build environment variables

The build helpers set what they need themselves. These variables let a caller supply
inputs or change a helper's behavior:

| Variable | Value | Effect |
| --- | --- | --- |
| `MIDNIGHT_ELEVATED_LISTING` | absolute path of the built `midnight-elevated-listing` (`.exe` on Windows) | `crates/midnight-explorer/build.rs` records that file's SHA-256 in the executable ([Authorization helper digest](#authorization-helper-digest)). Unset or empty: no digest is recorded. `build.bat`, `build.sh`, the `validate` host build and the CI release job set it. |
| `MIDNIGHT_FFMPEG_PREFIX` | a prebuilt FFmpeg 9.0.2 installation prefix (on Windows an MSVC build) | Used instead of building FFmpeg for the video thumbnail and playback workers. Windows builds require it; Linux builds the pinned FFmpeg when it is unset. Needs `MIDNIGHT_FFMPEG_SOURCE_ARCHIVE` and `LIBCLANG_PATH` too. |
| `MIDNIGHT_FFMPEG_SOURCE_ARCHIVE` | the FFmpeg 9.0.2 source archive | The corresponding source shipped with that prefix's libraries; refused unless it matches the pinned SHA-256. |
| `LIBCLANG_PATH` | libclang's library folder | Generates the FFmpeg bindings of the video workers. When it is unset, the Linux build searches `/usr/lib/llvm-*/lib`, the usual library folders and `llvm-config --libdir`. |
| `CEF_PATH` | on Linux, the root of a CEF SDK | Used instead of the downloaded CEF SDK after every file is checked against the pinned digests. On Windows it may name only the build's own verified `cef-sdk` folder. |
| `MIDNIGHT_HTML_TARGET_DIR` | Windows: an absolute drive path of plain folder names, such as `D:\mh` | Cargo target folder of the HTML viewer, short enough that CEF's objects stay below the 260-character MSVC path limit; inside the checkout it must be below `_build`. |
| `MIDNIGHT_VIEWER_SDK_CONFIG`, `MIDNIGHT_IMAGE_VIEWER_SDK_CONFIG`, `MIDNIGHT_HTML_SDK_CONFIG`, `MIDNIGHT_THUMBNAIL_SDK_CONFIG`, `MIDNIGHT_ARCHIVE_SDK_CONFIG` | path of a packaged public SDK bundle's Cargo configuration file | The viewer, image and PDF viewer, HTML viewer, thumbnail or archive build uses that bundle instead of packaging the public SDK, unless its own SDK argument is given. |
| `MIDNIGHT_VIEWERS_OFFLINE` | `1` | The viewer, thumbnail and archive builds run Cargo offline. |
| `MIDNIGHT_THUMBNAILERS_OFFLINE`, `MIDNIGHT_ARCHIVE_OFFLINE` | `1` | Only the thumbnail or only the archive build runs offline. |

The variables of the extracted-installation check are in
[Before publishing](#before-publishing).

### Build output and links

The build helpers in `scripts/`, the viewer plugins' build scripts, the image, PDF and
thumbnail plugins' Linux CI scripts, the archive worker's Linux build script, the image
decoder's native helpers (`plugins/image-decoder/native`), the release packager and the
test runners follow one rule for generated output:

- Output lies strictly below the checkout's own `_build`, recognized however a path
  spells the checkout, or, outside the checkout, below the first `_build` of a custom
  target directory. That `_build` and every folder below it must not be a link,
  junction or other reparse point; the checkout and the folders above it may be (links,
  a `subst` drive, hidden folders such as `AppData`).
- For a plugin's scripts, the plugin and the repository that holds it in `plugins/`
  are checkouts, so a plugin built on its own writes below its own `_build`; the image
  decoder's native helpers accept the repository's `_build` and any
  `plugins/NAME/_build`. The basic, Markdown and source viewers may install anywhere
  and use the rule only to decide whether notices without their marker may be
  replaced.
- An input folder that the caller names (a prebuilt FFmpeg prefix, `LIBCLANG_PATH`, a
  Linux `CEF_PATH`) may itself be a link or lie below links; nothing below it may be a
  link or `..`, except an FFmpeg prefix's versioned runtime library names, which must
  resolve inside it.
- The CEF and FFmpeg download caches must be plain folders inside the checkout, with no
  link from the checkout down, because a verified archive is opened again by path
  after its checksum is compared. In the image decoder's native helpers, nothing in a
  cached tool tree, the download folder or the extracted dav1d source may be a link;
  tool archives are verified in memory, and the dav1d archive is verified and extracted
  from the same bytes.

The scripts that do not follow this rule yet are listed as BUILD-05 in the
[implementation backlog](IMPLEMENTATION_BACKLOG.md).

### Files held by endpoint protection on Windows

On Windows, endpoint protection briefly holds files that were just written. While it
does, renaming them or a folder above them fails with access denied (5) or a sharing
(32) or lock (33) violation, and `File.Replace`, which writes the runtime registration
records, can report that it could not remove the replaced record (1175), leaving both
files unchanged. The build, packaging and registration scripts therefore retry renames
of their own fresh output for about 2 s with a short backoff, then fail with the first
error. This covers the CEF and FFmpeg downloads and stages, the FFmpeg build record
(`verified-build.json`), the image decoder's native tool and dav1d downloads and its
tool record, the thumbnail and HTML package swaps, the filesystem package files and the
runtime registration records. Publishing an extracted native tool folder waits up to
12.7 s instead, because it holds new executables, which Microsoft Defender's
first-sight cloud check can hold for up to 10 s. Other errors, including
`File.Replace`'s partial outcomes (1176, 1177), other platforms and deletions are never
retried. The thumbnail and HTML package swaps use one folder rename, which leaves
nothing behind when it is refused.

## Distribution archives

A release consists of a Windows x64 ZIP and a Linux x86-64 GNU/glibc tar.gz, from 0.6.1
on a Windows installer (MSI) made from that ZIP ([Windows installer](#windows-installer-msi)),
plus a `SHA256SUMS` file and, from 0.5.1 on, its signature `SHA256SUMS.minisig`
([Signing the release](#signing-the-release)). Every release so far is a preview: [0.1.0](releases/0.1.0.md), [0.5.0](releases/0.5.0.md),
[0.5.1](releases/0.5.1.md), [0.5.2](releases/0.5.2.md), [0.5.3](releases/0.5.3.md),
[0.5.4](releases/0.5.4.md), [0.6.0](releases/0.6.0.md), [0.6.1](releases/0.6.1.md),
[0.6.2](releases/0.6.2.md) and [0.6.3](releases/0.6.3.md), the current one. Linux archives
are built on Ubuntu 24.04; native Fedora, openSUSE and other desktop tests are pending
(PLAT-02). The Windows executable and the MSI are unsigned (REL-03): the checksums
establish download integrity, not code signing.

`scripts/package-release.py` packages an already built and tested executable with
Python 3.11 or later and its standard library. It neither builds nor publishes.

- All output must be below the checkout's own `_build` (see
  [Build output and links](#build-output-and-links)). It refuses an existing staging
  folder or archive.
- Every module input and the README version are validated before the staging folder is
  created, so a refused run leaves nothing that blocks the next attempt. The `--readme`
  file must begin with `Midnight Explorer VERSION ` for the workspace version and name
  the release's source, `https://github.com/TomasJelinek/fm/tree/vVERSION`, and
  `--revision` must be the full 40-character Git revision.
- `--metadata` takes target-filtered Cargo metadata:
  `cargo metadata --locked --format-version 1 --filter-platform TARGET`. Its source
  paths must be readable on the packaging host.
- `--dependency-list` takes
  `cargo tree -p midnight-explorer --locked --target TARGET --edges normal,build --prefix none --format '{p}'`.
  This uses the application's actual dependency selection instead of the metadata's
  workspace-wide feature union, which can include unused renderers. Use the target and
  features of the binary build; both runtime and build dependency notices are kept.
- `--rust-docs` names the toolchain's `share/doc/rust` folder, for the Rust standard
  library's copyright notices.
- `--license-overrides packaging/licenses` supplies license files that a crate omits
  from its published source, from a `NAME-VERSION/` subfolder copied from the crate's
  recorded upstream Git commit with its source URLs and hashes (each subfolder's
  `SOURCE.md` records them). Missing notices fail packaging; never substitute another
  version's license.

The packager's tests run with `python -B -m unittest discover -s scripts/tests -v`, and
CI's `scripts-and-lint` job runs them on Windows and Linux.

Example after the Windows build and checks of 0.6.3, in PowerShell 7 at the repository
root with Python installed. `$sysroot` holds the toolchain's sysroot, whose
`share/doc/rust` folder `--rust-docs` names:

```powershell
$sysroot = (rustc --print sysroot)
cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc |
    Out-File -Encoding utf8 _build/windows-metadata.json
cargo tree -p midnight-explorer --locked --target x86_64-pc-windows-msvc `
    --edges 'normal,build' --prefix none --format '{p}' |
    Out-File -Encoding utf8 _build/windows-dependencies.txt
python -B scripts/package-release.py --binary _build/release/midnight-explorer.exe `
    --metadata _build/windows-metadata.json --target x86_64-pc-windows-msvc `
    --dependency-list _build/windows-dependencies.txt `
    --format zip --readme packaging/windows/README.txt `
    --revision (git rev-parse HEAD) --rust-docs "$sysroot/share/doc/rust" `
    --license-overrides packaging/licenses --output _build/releases/v0.6.3
```

For Linux, use the
Linux binary, metadata and target (`x86_64-unknown-linux-gnu`), `--format tar.gz`,
`--readme packaging/linux/README.txt` and
`--desktop-entry packaging/linux/io.github.TomasJelinek.MidnightExplorer.desktop`.
Archives set executables and folders to mode 0755 and other files to 0644, whatever
the staging permissions; tar ownership is uid/gid 0 with no user or group names. Keep
Windows and Linux build folders separate when they share one checkout.

An archive contains:

- the executable and `midnight-elevated-listing` (`.exe` on Windows) beside it;
- on Linux, `install-html-viewer-apparmor.sh` and, with `--desktop-entry`, the desktop
  entry;
- the program's license, the unmodified GNU GPL version 3 text from the repository's
  `LICENSE`: `LICENSE.txt` in the Windows ZIP, so that it opens on a double-click, and
  `LICENSE` in the Linux tar.gz;
- `README.txt` (run instructions, which must name the version and its source,
  `https://github.com/TomasJelinek/fm/tree/v<version>`), the project's `README.md`,
  `SECURITY.md`, `SYSTEM_REQUIREMENTS.md`, `KEYBOARD_SHORTCUTS.md`, `FEATURE_STATUS.md`,
  `ASSET_PROVENANCE.md`, `THEME_PROVENANCE.md` and the application icon;
- `BUILD-INFO.json` with the version, target, source revision, Rust version and the
  executable's SHA-256, and the manifests, file sizes and SHA-256 values of the
  authorization helper and every module ([BUILD-INFO.json](#build-infojson));
- `Cargo.lock`, the dependency inventory (`THIRD-PARTY.json` and `THIRD-PARTY.txt`)
  and the original license texts under `licenses/`, including the Rust standard
  library's and the adapted themes';
- the module folders `viewers/`, `filesystems/` and `thumbnailers/`.

Everything lies below one folder, `midnight-explorer-<version>-<target>/`, and the
archive is named after it. The update check installs only an archive with exactly
this layout whose `BUILD-INFO.json` matches its files
([Updates](developer/UPDATES.md#trust)), so do not repack or rename it.

### BUILD-INFO.json

`scripts/package-release.py` writes `BUILD-INFO.json` at the top of the release folder:
one JSON object in UTF-8, indented, ending with a newline.

| Field | Type | Meaning | Example |
| --- | --- | --- | --- |
| `version` | string | The workspace version. | `"0.6.3"` |
| `target` | string | The Rust target of the build. | `"x86_64-pc-windows-msvc"` |
| `revision` | string | The packaged commit, the full 40-character Git revision. | |
| `profile` | string | Always `"release"`. | `"release"` |
| `rust` | string | The toolchain channel of `rust-toolchain.toml`. | `"1.99.1"` |
| `binary` | string | The executable's file name. | `"midnight-explorer.exe"` |
| `binary_sha256` | string | The executable's SHA-256, 64 lowercase hexadecimal digits. | |
| `authorization_helper` | [file record](#file-records) | `midnight-elevated-listing` (`.exe` on Windows). | |
| `viewer_plugins` | array of objects | One per viewer package: `id`, `version`, `target`, `package_sha256` and `files` ([file records](#file-records)); a worker viewer adds `kind` (`"image-viewer-worker"` or `"web-viewer-worker"`), `protocol`, `entry_executable` and `extensions`, a native viewer `abi_major`, `entry_library` and `models`. | |
| `filesystem_plugins` | array of objects | One per filesystem package: the fields of its package manifest plus `package_sha256` and `files`. | |
| `thumbnail_workers` | object | `providers` (the thumbnail providers' manifests), `target`, `package_sha256` and `files`. | |

#### File records

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string | The file's path in the release folder, with `/` separators, for example `"thumbnailers/avcodec-63.dll"`. |
| `bytes` | number | The file's size. |
| `sha256` | string | The file's SHA-256, 64 lowercase hexadecimal digits. |
| `executable` | boolean | Whether the archive gives the file mode 0755. |

A `package_sha256` is the SHA-256 of the package's file records written as compact
JSON with sorted keys.

The updater reads `version`, `target`, `binary` and `binary_sha256` and checks every
object anywhere in the file that has a `path` and a `sha256`, with its `bytes` where
given ([Updates, Trust](developer/UPDATES.md#trust)). It refuses a `BUILD-INFO.json`
larger than 16 MiB. An MSI installs every file except `BUILD-INFO.json`
([Windows installer](#windows-installer-msi)).

### Module checks

The packager validates every module before copying it:

- All six viewer packages under `viewers/` (or `--viewers-root`): platform, native ABI,
  image-worker or web-worker protocol, code entry and package paths, and the HTML
  viewer's complete CEF runtime inventory. Plugin-owned notices stay inside each
  package.
- Exactly one native and one network filesystem provider and the `midnight.fs.archive`
  worker under `filesystems/` (or `--filesystems-root`): protocol, ABI, target,
  address encoding, capabilities, library entry and embedded notices. Network
  providers need host ABI 1.2 or later, opaque addresses and the activate, refresh and
  resolve-text capabilities. Library entries and archive worker executables stay
  distinct in `BUILD-INFO.json`.
- Both the image and the video thumbnail workers under `thumbnailers/` (or
  `--thumbnailers-root`), with the video playback worker, the bundled FFmpeg 9.0.2
  libraries and their corresponding source and build notices. The two platform decoder
  helpers (`midnight-thumbnail-video-system`, `midnight-video-playback-system`) are
  required on Windows; on Linux they are optional but must come as a pair.
- The authorization helper must be a regular file beside the executable.

It rejects links (symbolic links, junctions and other reparse points), `..` and other
escaping paths inside module packages, crate license files and license overrides, and
missing notices. Folders above a package root, such as a linked Cargo home or Fedora
Silverblue's `/home`, may be links. Package hashes establish integrity, not publisher
authenticity. Native C ABI viewers and filesystem providers run with the application's
authority; image decoding also runs in the required restricted worker.

**Retry with authorization** uses the shipped `midnight-elevated-listing` only from a
folder that no other account can change. An executable that records the helper's digest
([Authorization helper digest](#authorization-helper-digest)) accepts the shipped helper
also in the user's own installation (the per-user MSI, a ZIP extracted into the
profile); one without it accepts only an installation that an administrator (Windows,
for example below `C:\Program Files`) or root (Linux, for example
`/opt/midnight-explorer`) made. Where the helper cannot be used, a denied folder's
message offers no retry and says why. Packages built before these checks used
the shipped helper from any folder; see
[the authorization helper](../SECURITY.md#directory-authorization-helper) for what
their users must replace or delete.

### Authorization helper digest

`crates/midnight-explorer/build.rs` records the SHA-256 of the file that the
`MIDNIGHT_ELEVATED_LISTING` environment variable names in the executable
(`MIDNIGHT_ELEVATED_LISTING_SHA256`), as the marked record
`MIDNIGHT_AUTHORIZATION_HELPER_SHA256=<digest>;`; unset, the record is empty. The helper
must be built before the executable that records it, so `build.bat`, `build.sh`, the
`validate` host build and the CI release job build only the helper first and then both
packages with the variable naming that helper. In PowerShell at the repository root:

```powershell
cargo build -p midnight-explorer -p midnight-elevation --bin midnight-elevated-listing --release --locked
$env:MIDNIGHT_ELEVATED_LISTING = (Resolve-Path _build\release\midnight-elevated-listing.exe).Path
cargo build -p midnight-explorer -p midnight-elevation --release --locked
```

Both runs select the same packages, so the helper and its dependencies keep their
features and are not rebuilt: the second run builds only the executable. The package must
then ship exactly that helper; a different one is refused at run time as altered (the
retry is not offered), never trusted. `scripts/package-release.py` refuses an executable
without a recorded digest or with one that differs from the packaged helper, and
`package-release.py check-authorization-helper --binary EXECUTABLE --target TARGET`
(run by the CI release job; a synopsis, with the executable's path and its Rust target
in place of the capitals) checks a build. The variable is not a run-time setting, and
a digest kept in a data file such as `BUILD-INFO.json` would not do: anything beside the
executable in a per-user installation can be changed by its user.

The public download page carries the official Slint attribution badge, and the
application keeps its AboutSlint widget. Midnight Explorer is GPL-3.0-or-later, so each
published release must have its corresponding source available: push the release tag
(`v<version>`) for the packaged commit before publishing the archives.

### Before publishing

1. Run `scripts/package-msi.py verify` on the MSI ([Windows installer](#windows-installer-msi)).
2. Unpack both archives.
3. Compare the unpacked executables' hashes with those of the tested artifacts.
4. Run the unpacked executables' filesystem and UI smoke checks.
5. Inspect the ELF requirements and the archive paths and permissions.
6. Check the workers' grant on the unpacked ZIP's `thumbnailers` folder, as the next
   steps describe.
   - A Windows ZIP carries no NTFS permissions, so the build scripts' read-and-execute
     grant for the workers' [AppContainer](GLOSSARY.md#appcontainer) on the FFmpeg
     libraries and playback helpers is not in it; the application adds it at run time
     in a private installation, one that no other account can change
     ([SECURITY.md](../SECURITY.md#the-workers-grant-on-installed-files)).
   - The unpacked folder must not name the workers' SID.
7. Set `MIDNIGHT_THUMBNAILERS_TEST_PACKAGE` to the unpacked ZIP's `thumbnailers` folder,
   for example after unpacking into `_build/release-check`:

   ```powershell
   Expand-Archive _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip -DestinationPath _build/release-check
   $env:MIDNIGHT_THUMBNAILERS_TEST_PACKAGE = (Resolve-Path _build/release-check/midnight-explorer-0.6.3-x86_64-pc-windows-msvc/thumbnailers).Path
   ```

8. Set `HOST_TEST_INSTALL_ROOT` to a new private folder, as
   [Private installation root](developer/TEST_EXECUTION.md#private-installation-root)
   creates it (`scripts/validate.ps1` makes it the same way).
9. Run the two ignored tests, which copy the folder as an extraction does and start the
   video thumbnail helper and both playback helpers:

   ```powershell
   cargo test -p midnight-thumbnail-host --test integration extracted_installation -- --ignored
   cargo test -p midnight-media-host --test extracted_installation -- --ignored
   ```

10. Verify `SHA256SUMS` ([Signing the release](#signing-the-release)).

Do not include private preferences, caches, toolchains, PDBs or generated intermediate
objects.

### Windows installer (MSI)

`scripts/package-msi.py` turns the finished Windows ZIP into
`midnight-explorer-<version>-x86_64-pc-windows-msvc.msi` beside it. It runs on Windows
with Python 3.11 or later and needs nothing else: it writes the database with the
Windows Installer API (`msi.dll`) and the cabinet with `makecab.exe`, and checks it with
`expand.exe`, all part of Windows ([installer tooling decision](DEPENDENCY_POLICY.md#current-decisions)).
It neither builds nor publishes. For 0.6.3, in PowerShell at the repository root:

```powershell
python -B scripts/package-msi.py build `
    --zip _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip
python -B scripts/package-msi.py verify `
    --msi _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi `
    --zip _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip
```

`build` writes below the checkout's `_build` (`--output` names another folder there) and
refuses an existing MSI.

- **Single source:** the files come from the ZIP alone, so the MSI cannot drift from
  it. `build` first applies the ZIP's own checks: the archive name and the workspace
  version, one root folder, regular files with portable ASCII names (the database uses
  code page 1252) and paths of at most 160 characters (so that installed paths stay
  below Windows' 260-character limit), `BUILD-INFO.json` naming the version, target and executable, the
  executable's SHA-256, `README.txt` naming the version and its source, `LICENSE.txt`
  equal to the repository's `LICENSE`, the helper and the notices. The MSI then holds
  every file of the ZIP except `BUILD-INFO.json`: the application updates in place
  only a folder with that file ([Updates](developer/UPDATES.md#staging)), and a folder
  that Windows Installer manages must be updated by a newer MSI instead.
- **Installation:** one dual-purpose package (Windows Installer's single package
  authoring: `ALLUSERS=2` with `MSIINSTALLPERUSER=1`; built from 0.6.2, whose MSI stops
  with error 2228 for lack of a `Signature` table, so 0.6.3's is the first that installs;
  0.6.1's MSI was per user only). Its first dialog, **ScopeDlg**, asks who can use the
  application:
  - **Just for me** (default): per user, without administrator rights or a UAC prompt,
    into `%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)`, the 0.6.1 per-user MSI's
    folder. `%LOCALAPPDATA%\Programs\Midnight Explorer` is left to the application's
    own **Move to my profile…** copy, which the shared-installation notice offers
    ([Shared installations](USER_GUIDE.md#shared-installations)).
  - **For all users:** per machine into `C:\Program Files\Midnight Explorer (MSI)`;
    the Install button carries the UAC shield and Windows Installer asks for consent
    when installation starts. Not `C:\Program Files\Midnight Explorer`, which is left
    to an administrator's own copy of a build ([Build and run](USER_GUIDE.md#build-and-run)).
    The package sets no permissions, so the folder inherits Program Files' (only
    administrators can change it).

  The `INSTALLDIR` property follows the scope ([MSI properties](#msi-properties)):
  `Directory` puts it below
  `ProgramFiles64Folder`, and the property-setting action `SetUserInstallFolder` sets the
  per-user folder before `CostFinalize` when the installation is per user and no
  `INSTALLDIR` is given; the dialog's Install button sets `ALLUSERS=2`,
  `MSIINSTALLPERUSER` (`1` or empty) and `INSTALLDIR` (`[LocalAppDataFolder]Programs\...`
  or `[%ProgramW6432]\...`) for its choice, because the folder properties of the
  dialog's session do not change with the scope. Without the dialog,
  `msiexec /i <msi> /qn` installs per user and `msiexec /i <msi> /qn ALLUSERS=1` (or
  `MSIINSTALLPERUSER=""`) per machine from an elevated prompt (synopses: `<msi>` is the
  MSI's path); the dialog's Install
  overrides an `INSTALLDIR` from the command line. A Start menu shortcut **Midnight
  Explorer**, in the user's or the all-users Start menu, starts the executable in the
  folder where the left panel opens on a first start: per user the user's profile
  folder (`[%USERPROFILE]`), like the Desktop shortcut of `scripts/deploy-private.ps1`;
  for all users the Windows drive's root (`[WindowsVolume]`, for example `C:\`), which
  every account can open. A profile variable cannot serve all users: Windows Installer
  expands it when it creates the shortcut, so every account would start in the
  installing administrator's profile, and Windows refuses to start a shortcut whose
  start folder it cannot open. Add/Remove Programs (HKCU or HKLM, as Windows
  Installer chooses for the scope) shows the version, the publisher Tomas Jelinek, the
  Harvest Navigator icon (`assets/branding/midnight-explorer.ico`) and links to the
  repository, this release's source and the releases page; it offers Remove and
  Repair, not Change. Running the installed version's MSI again shows **MaintenanceDlg**
  (Repair or Remove). The other dialogs are the progress window (with a cancel
  question) and the closing messages for success, cancellation and failure; error
  messages use Windows Installer's own message box.
- **One scope at a time:** an upgrade finds only the installation of its own scope. A
  per-user installation therefore upgrades the 0.6.1/0.6.2 per-user MSI in place (same
  `UpgradeCode`). One version (one product code) is never installed in both scopes:
  Windows Installer treats a second install of it as maintenance of the installed copy.
  Choosing For all users beside an earlier per-user version (or the other way round)
  still installs a second copy, but the package supports one scope at a time: the
  component codes follow the files, not the scope, so both copies register the same
  components at different paths, and removing one of them leaves its files, shortcut and
  key (CI saw this; REL-05 tracks refusing the second scope). The dialog reads both
  scopes' registry key (`AppSearch`/`RegLocator`, `Software\Midnight Explorer Installer`
  in HKCU and HKLM) and then says that the other copy stays installed; with only an
  all-users copy present it preselects For all users. An extracted ZIP, the Move to my
  profile copy and an administrator's copy of a build use other folders.
- **Upgrades:** the `UpgradeCode` is fixed in the script and must never change. The
  product code follows from the version and the package code from the ZIP's SHA-256,
  so one ZIP always gives the same codes. A newer MSI removes the installed version
  completely before installing; an older one stops with "A newer version of Midnight
  Explorer is already installed". Running the installed version's MSI again offers
  Windows Installer's repair or remove.
- **Nothing runs code:** there are no custom actions besides five that set properties
  (the install location shown in Add/Remove Programs, the shortcut's start folder per
  user and for all users, the per-user `INSTALLDIR` and the dialog's preselected scope), no `Binary` table and no
  services. Each folder is one component whose key path is a value under
  `Software\Midnight Explorer Installer` with registry root -1: HKCU per user, HKLM per
  machine ([Registry values](#registry-values)). That key and the installed folders,
  once empty, are removed on uninstall.
  Settings, caches and update downloads in `%LOCALAPPDATA%\Midnight Explorer` are never
  touched.
- **Updates:** the application swaps in place only a folder with `BUILD-INFO.json`,
  which the MSI leaves out. From 0.6.3, an MSI installation finds
  its installation through Windows Installer (the product of this `UpgradeCode` whose
  `InstallLocation`, set from `INSTALLDIR` by `SetInstallLocation`, is its folder, in
  the per-user or the per-machine context) and updates with the next release's MSI: the
  update dialog downloads `midnight-explorer-<version>-x86_64-pc-windows-msvc.msi`,
  checks it against its line in the signed `SHA256SUMS` and runs
  `msiexec /i <msi> /passive /norestart` with `ALLUSERS=2 MSIINSTALLPERUSER=1` (per
  user) or `ALLUSERS=1` (per machine, through UAC) and `INSTALLDIR` set to the copy's
  folder ([MSI installations](developer/UPDATES.md#msi-installations)). The MSI therefore
  must keep its name, its `UpgradeCode`, `SetInstallLocation`, the `INSTALLDIR`
  property and its major upgrade, and its line must be in the signed `SHA256SUMS`
  ([Signing the release](#signing-the-release)); a release without it offers MSI installations the
  release page. No marker file is added: Windows Installer's own records identify the copy,
  also for a copy installed by 0.6.1. That version cannot run the hand-off itself; its
  users run a newer MSI once by hand. The first update through the hand-off is from an
  installed 0.6.3 to the release after it.
- **Not included:** the Visual C++ runtime (as for the ZIP), code signing (REL-03) and
  permissions of its own. **Retry with authorization** works from either scope's copy
  when its executable records the helper's digest
  ([Authorization helper digest](#authorization-helper-digest)).
- **Checks:** `build` writes to a temporary name and keeps the MSI only after the same
  checks as `verify`: the summary information (x64, schema 500, compressed, no
  elevation for the default per-user installation, package code), only the expected
  tables, every table row (dialogs included) as the ZIP defines it, the dual-purpose
  rules (`ALLUSERS=2` and `MSIINSTALLPERUSER=1`, the fixed `UpgradeCode`, `INSTALLDIR`
  below `ProgramFiles64Folder` with the per-user folder action, key paths and registry
  values with root -1, no profile folder in `Directory`, shortcuts in
  `ProgramMenuFolder`, exactly the package's own folders removed on uninstall, the
  dialog's scope and folder events, property-setting custom actions only, and
  `SetInstallLocation` recording `[INSTALLDIR]` as the install location, by which the
  application finds an installed copy), the
  installed paths, and every file of the embedded cabinet, extracted with `expand.exe`,
  byte for byte against its ZIP entry. An administrative installation of an MSI made
  from the 0.6.0 ZIP extracted every file unchanged
  ([MSI extraction verification](verification/2026-11-26-release-msi.md)), as did one of
  a dual-purpose stand-in, whose per-user costing resolved the per-user folders
  ([dual-scope MSI verification](verification/2026-11-26-dual-scope-msi.md)). Real
  installations run in CI ([MSI installation test](#msi-installation-test)).
- **Release notes:** describe the one MSI with both choices: Just for me (no
  administrator rights, `%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)`, updates an
  installed per-user MSI) and For all users (administrator approval,
  `C:\Program Files\Midnight Explorer (MSI)`; an existing per-user copy stays installed beside it).
- **Tests:** `scripts/tests/test_package_msi.py`, in the scripts suite that CI runs on
  Windows and Linux, checks the payload rules and the database plan on both and, on
  Windows, builds and verifies an MSI of a small stand-in release. CI does not package
  releases, so the release MSI itself is built on the owner's machine with the ZIP.

#### MSI properties

The properties that choose the installation, as the package defines them and as
`msiexec` accepts them on its command line:

| Property | Default in the package | Meaning |
| --- | --- | --- |
| `ALLUSERS` | `2` | With `MSIINSTALLPERUSER=1`: per user. `1` on the command line, or `2` with an empty `MSIINSTALLPERUSER`: per machine. |
| `MSIINSTALLPERUSER` | `1` | `1`: per user; empty (`MSIINSTALLPERUSER=""`): per machine. |
| `INSTALLDIR` | below `ProgramFiles64Folder`; per user `SetUserInstallFolder` sets `[LocalAppDataFolder]Programs\Midnight Explorer (MSI)\` | The installation folder. The update hand-off passes the installed copy's folder. |
| `ME_SCOPE` | `user` | The dialog's choice, `user` or `machine`; `SetMachineScope` preselects `machine` for a per-machine command line or when only an all-users copy exists. |
| `ME_USER_COPY`, `ME_MACHINE_COPY` | not set | Set by `AppSearch` when the `RootFolder` value exists in HKCU (`ME_USER_COPY`) or HKLM (`ME_MACHINE_COPY`), so the dialog can say that the other scope's copy stays installed. |
| `MESTARTFOLDER` | (none) | The Start menu shortcut's start folder: `SetShortcutWorkingFolder` sets it to `[%USERPROFILE]` per user, `SetMachineShortcutWorkingFolder` to `[WindowsVolume]` for all users. |
| `ARPINSTALLLOCATION` | set by `SetInstallLocation` to `[INSTALLDIR]` | Windows Installer's `InstallLocation`, by which the application finds an MSI installation. |
| `UpgradeCode` | `{DDF2A2BA-6901-484C-97B2-4EF662BF5E72}` | Fixed for every MSI of the product. |
| `ProductCode`, `ProductVersion` | derived from the version | One product code per version. |
| `ARPNOMODIFY` | `1` | Add/Remove Programs offers Remove and Repair, not Change. |

#### Registry values

Under `Software\Midnight Explorer Installer`, in HKCU per user and HKLM per machine
(registry root -1), each value is a DWORD `1` that serves as a component's key path:

| Value | Component |
| --- | --- |
| `RootFolder` | The installation folder; `AppSearch` looks for it in the other scope. |
| one value per subfolder, named after its component | That subfolder. |
| `StartMenuShortcut` | The Start menu shortcut. |

#### MSI installation test

The Windows leg of CI's `release` job installs the MSI for real on its disposable hosted
runner (an administrator) with `scripts/test-msi-install.py`, after the release build
and this run's plugin packages. Its commands, in order:

1. `stand-in` writes a release-shaped ZIP.
   - It holds the job's release `midnight-explorer.exe` and
     `midnight-elevated-listing.exe` and its viewer and filesystem packages, with
     stand-in notices and no thumbnailers: CI does not build the full release.
   - It passes the checks of `package-release.py`'s layout that `package-msi.py`
     applies.
2. `prepare` builds the MSI of that ZIP and of a copy versioned 0.0.1 (same files,
   README and `BUILD-INFO.json` rewritten).
3. `ice` runs the Windows SDK's Internal Consistency Evaluators
   ([ICE](GLOSSARY.md#ice) rules, `darice.cub`; see the
   [dependency policy](DEPENDENCY_POLICY.md#current-decisions)).
   - Errors beyond the recorded findings fail the job.
   - Recorded (REL-05): ICE03 for every column, as the package has no `_Validation`
     table; ICE20 for the missing `FilesInUse` dialog, `ErrorDialog` property and
     `AdminUISequence` closing dialogs; ICE57 for the `StartMenuShortcut` component
     (per-user data, scope-dependent key path).
4. `previous` downloads the published per-user MSIs older than the workspace version
   (0.6.1 and 0.6.2 for 0.6.3) and checks each against its release's signed
   `SHA256SUMS`.
5. `run --disposable-machine --scenario scopes --scenario previous --ui-smoke` installs
   with `msiexec /qn /l*v`:
   1. It refuses to start when any product with the MSI's `UpgradeCode`, either install
      folder, shortcut or registry key exists, and writes a marker in
      `%LOCALAPPDATA%\Midnight Explorer`.
   2. It takes **Just for me** (`ALLUSERS=2 MSIINSTALLPERUSER=1`) first, then **For all
      users** (`ALLUSERS=1`), and in each scope runs items 3 to 6, beginning with an
      installation of 0.0.1.
   3. It upgrades 0.0.1 to the release, which removes 0.0.1.
   4. It installs 0.0.1 again, which must be refused (exit 1603 and the "A newer
      version" message).
   5. It starts the installed `midnight-explorer.exe --smoke-test`, which must report a
      private installation, and `--ui-smoke-test --software`.
   6. It deletes the executable, `README.txt` and the deepest file and repairs the
      installation (`/fomus`).
   7. It removes each scope's installation before the next is installed, leaving no
      files, folders, shortcut, key or entry (the package supports one scope at a time).
   8. It installs each published MSI, upgrades it in place with the release (per user)
      and removes it.
   9. It checks that the marker survived every removal and that nothing stays
      installed.
6. `run --scenario ui` installs through the dialogs.
   - `scripts/msi-ui-install.ps1` starts `msiexec /i` with its full UI, clicks Just for
     me or For all users in ScopeDlg, presses the Install button that the choice shows
     and Finish; the same checks follow, then removal.
   - It is new on the hosted runners and so does not fail the job
     (`continue-on-error`) until it has passed there.

After every installation, `run` checks:

- Windows Installer's record of the scope and version;
- every installed file against the ZIP (byte for byte, nothing extra);
- the `Software\Midnight Explorer Installer` values in HKCU or HKLM;
- the Start menu shortcut's target and start folder (the user's profile, or the Windows
  drive's root for all users);
- the Installed apps entry (name, version, publisher, install location and existing
  icon);
- for all users, the folder's permissions: they may grant changes only to SYSTEM,
  Administrators, TrustedInstaller and CREATOR OWNER or what Program Files itself
  grants.

The logs of every `msiexec` run (and a screenshot of a UI failure) are uploaded as
the `msi-install-logs` artifact. `run --dry-run` prints the plan anywhere without
changing anything; never run the test without `--dry-run` on a machine whose
installation or settings matter. Not covered: the UAC prompt (the runner is already
elevated), a second account starting the all-users copy, **Retry with authorization**,
the thumbnailers and HTML viewer that a real release ZIP adds, and a ZIP made by
`package-release.py` itself.

### Signing the release

The update check installs only releases whose `SHA256SUMS` is signed with the
project's release key ([Updates](developer/UPDATES.md#trust)). Sign on the owner's
machine (Python 3.11 or later, standard library only), in PowerShell at the repository
root, for 0.6.3:

1. Put both archives and the MSI in one folder, `_build/releases/v0.6.3/`.
2. Set `MIDNIGHT_RELEASE_KEY` to the path of the secret key file
   ([Keys](developer/UPDATES.md#keys)).
3. Write `SHA256SUMS`:

   ```powershell
   python -B scripts/release-signing.py checksums `
       _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip `
       _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi `
       _build/releases/v0.6.3/midnight-explorer-0.6.3-x86_64-unknown-linux-gnu.tar.gz
   ```

4. Sign it. `sign` asks for the key's password, writes `SHA256SUMS.minisig` and
   verifies it:

   ```powershell
   python -B scripts/release-signing.py sign --version 0.6.3 `
       --secret-key $env:MIDNIGHT_RELEASE_KEY _build/releases/v0.6.3/SHA256SUMS
   ```

5. Verify the pair against the repository's public key:

   ```powershell
   python -B scripts/release-signing.py verify --version 0.6.3 _build/releases/v0.6.3/SHA256SUMS
   ```

6. Upload `SHA256SUMS` and `SHA256SUMS.minisig` with the archives and the MSI. The
   asset names must stay exactly these.

`checksums` takes files of one folder only and refuses an existing `SHA256SUMS`
(`--force` replaces it); `sign` likewise refuses an existing signature. `SHA256SUMS`
has one ASCII line per file, sorted by file name: the file's SHA-256 in 64 lowercase
hexadecimal digits, two spaces and the file name. For 0.6.3 its lines name, in order:

| Line | File |
| --- | --- |
| 1 | `midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi` |
| 2 | `midnight-explorer-0.6.3-x86_64-pc-windows-msvc.zip` |
| 3 | `midnight-explorer-0.6.3-x86_64-unknown-linux-gnu.tar.gz` |

The application's updater uses its platform's archive line for an extracted
copy and the MSI's line for an MSI installation; the MSI's line also
lets users check the installer they download. A release whose `SHA256SUMS` lacks the
MSI line cannot update MSI installations. The trusted comment of `SHA256SUMS.minisig` is
`timestamp:<now> file:SHA256SUMS release:midnight-explorer-<version>`. `sign` refuses
a secret key inside the repository or one that does not belong to
`packaging/update-signing.pub`. The version must be the workspace version that the
archives' `BUILD-INFO.json` reports, and the public key in the tagged commit must be
the one the release was signed with. The key pair is created once with
`generate-key` (see [Keys](developer/UPDATES.md#keys)).
