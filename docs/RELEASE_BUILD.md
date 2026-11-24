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
comparison, not after every UI edit: fat LTO and one code-generation unit trade build
time for the published binary.

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

A same-source Windows comparison of this profile reduced the executable from 42,425,856 to
37,096,448 bytes (12.56%), with overlapping native resize and redraw timings across six
alternating runs per profile ([record](verification/2026-11-07-release-optimization.md));
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

Run `build.bat release` or `./build.sh release` (Cargo's `--release`), then smoke-test
that executable. Adding `--validate` also runs CI's tests for the platform and the
installed smoke test of that release build
([full validation](developer/TEST_EXECUTION.md#full-validation)).

The helpers build the host and its authorization helper (`midnight-elevation`), then
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
the network helper has the same options, and its
[guide](../plugins/midnight-filesystem-network/README.md) covers its Linux system
runtime. Both use the shared filesystem package helper, which stages all inputs before
it replaces an installed library. Each plugin has its own lockfile, tests, release
profile and third-party notices.

CI and full validation run the workspace tests in the debug test profile (the plugin
jobs use their own profiles) and, against a release build, the installed filesystem
smoke test. CI's `release` job builds the release host and helper, checks this run's
filesystem packages with `scripts/package-release.py validate-filesystems` and runs
that smoke test; it does not package or publish. Release builds take longer and may
use more RAM because of whole-program optimization.

### Build output and links

The build helpers in `scripts/`, the viewer plugins' build scripts, the image, PDF and
thumbnail plugins' Linux CI scripts, the archive worker's Linux build script, the image
decoder's native helpers (`plugins/image-decoder/native`), the release packager and the
test runners follow one rule for generated output:

- Output lies strictly below the checkout's own `_build`, recognised however a path
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

A release consists of a Windows x64 ZIP and a Linux x86-64 GNU/glibc tar.gz, plus a
`SHA256SUMS` file and, from 0.5.1 on, its signature `SHA256SUMS.minisig`
([signing](#signing-the-release)): the [0.1.0](releases/0.1.0.md), [0.5.0](releases/0.5.0.md),
[0.5.1](releases/0.5.1.md) and [0.5.2](releases/0.5.2.md) previews. Linux archives
are built on Ubuntu 24.04; native Fedora, openSUSE and other desktop tests are pending
(PLAT-02). The Windows executable is unsigned: the archive checksums establish download
integrity, not code signing.

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

Example after the Windows build and checks (PowerShell, Python installed):

```powershell
cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc |
    Out-File -Encoding utf8 _build/windows-metadata.json
cargo tree -p midnight-explorer --locked --target x86_64-pc-windows-msvc `
    --edges 'normal,build' --prefix none --format '{p}' |
    Out-File -Encoding utf8 _build/windows-dependencies.txt
python -B scripts/package-release.py --binary _build/release/midnight-explorer.exe `
    --metadata _build/windows-metadata.json --target x86_64-pc-windows-msvc `
    --dependency-list _build/windows-dependencies.txt `
    --format zip --readme packaging/windows/README.txt `
    --revision (git rev-parse HEAD) --rust-docs "RUST_SYSROOT/share/doc/rust" `
    --license-overrides packaging/licenses --output _build/releases/v0.5.2
```

Replace `RUST_SYSROOT` with the output of `rustc --print sysroot`. For Linux, use the
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
  authorization helper and every module;
- `Cargo.lock`, the dependency inventory (`THIRD-PARTY.json` and `THIRD-PARTY.txt`)
  and the original license texts under `licenses/`, including the Rust standard
  library's and the adapted themes';
- the module folders `viewers/`, `filesystems/` and `thumbnailers/`.

Everything lies below one folder, `midnight-explorer-<version>-<target>/`, and the
archive is named after it. The update check installs only an archive with exactly
this layout whose `BUILD-INFO.json` matches its files
([Updates](developer/UPDATES.md#trust)), so do not repack or rename it.

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

**Retry with authorization** uses the shipped `midnight-elevated-listing` only when an
administrator (Windows, for example below `C:\Program Files`) or root (Linux, for
example `/opt/midnight-explorer`) installs the extracted folder where standard users
cannot change it; the bundled README explains this. In a private per-user installation,
including `build.bat`'s, and in an extracted folder that its user can change, the retry
is still offered but refused with that requirement. Packages built before this check
used the shipped helper from such a folder; see
[the authorization helper](../SECURITY.md#directory-authorization-helper) for what
their users must replace or delete.

The public download page carries the official Slint attribution badge, and the
application keeps its AboutSlint widget. Midnight Explorer is GPL-3.0-or-later, so each
published release must have its corresponding source available: push the release tag
(`v<version>`) for the packaged commit before publishing the archives.

Before publishing, unpack both archives, compare the executable hashes with the tested
artifacts, run the unpacked executables' filesystem and UI smoke checks, inspect the
ELF requirements and the archive paths and permissions, and verify `SHA256SUMS`. A
Windows ZIP carries no NTFS permissions, so the build scripts' read-and-execute grant
for the workers' AppContainer on the FFmpeg libraries and playback helpers is not in
it; the application adds it at run time in a private installation
([SECURITY.md](../SECURITY.md#the-workers-grant-on-installed-files)). Check that with
the unpacked ZIP's `thumbnailers` folder, which must not name the workers' SID:
`MIDNIGHT_THUMBNAILERS_TEST_PACKAGE=<unpacked>\thumbnailers` and a private
`HOST_TEST_INSTALL_ROOT` (as `scripts/validate.ps1` makes it), then
`cargo test -p midnight-thumbnail-host --test integration extracted_installation -- --ignored`
and `cargo test -p midnight-media-host --test extracted_installation -- --ignored`: both
copy the folder as an extraction does and start the video thumbnail helper and both
playback helpers. Do not
include private preferences, caches, toolchains, PDBs or generated intermediate
objects.

### Signing the release

The update check installs only releases whose `SHA256SUMS` is signed with the
project's release key ([Updates](developer/UPDATES.md#trust)). With both archives in
one folder, on the owner's machine (Python 3.11 or later, standard library only):

```powershell
python -B scripts/release-signing.py checksums `
    _build/releases/v0.5.2/midnight-explorer-0.5.2-x86_64-pc-windows-msvc.zip `
    _build/releases/v0.5.2/midnight-explorer-0.5.2-x86_64-unknown-linux-gnu.tar.gz
python -B scripts/release-signing.py sign --version 0.5.2 `
    --secret-key <path outside every checkout>\midnight-explorer-release.key `
    _build/releases/v0.5.2/SHA256SUMS
python -B scripts/release-signing.py verify --version 0.5.2 _build/releases/v0.5.2/SHA256SUMS
```

`checksums` writes `SHA256SUMS` (`<sha256>  <archive name>` lines). `sign` asks for the
key's password, writes `SHA256SUMS.minisig` with the trusted comment
`timestamp:<now> file:SHA256SUMS release:midnight-explorer-<version>`, verifies it and
refuses a secret key inside the repository or one that does not belong to
`packaging/update-signing.pub`. `verify` checks the pair against the repository's
public key. Upload `SHA256SUMS` and `SHA256SUMS.minisig` with the archives; the asset
names must stay exactly these. The version must be the workspace version that the
archives' `BUILD-INFO.json` reports, and the public key in the tagged commit must be
the one the release was signed with. The key pair is created once with
`generate-key` (see [Keys](developer/UPDATES.md#keys)).
