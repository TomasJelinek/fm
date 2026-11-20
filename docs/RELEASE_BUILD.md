# Midnight Explorer release builds

The application package `midnight-explorer` builds `me.exe` on Windows and `me` on
Linux. Native output is `_build/release/me.exe` or `_build/release/me`.

Use **debug/test for ordinary development iteration**. `build.bat` and `build.sh`
already default to debug, producing `_build/debug/me.exe` or `_build/debug/me`.
For a focused host-code iteration, build only `midnight-explorer` with Cargo;
rebuild plugins when their own code/API changes. Native plugin trust/deployment
requirements still apply in debug. Run release for final integration/packaging or
a specific performance comparison, rather than after every UI edit. Fat LTO and
one code-generation unit deliberately trade build time for the published binary.

OS baselines, runtime dependencies and build RAM/storage allowances are specified
in [system requirements](SYSTEM_REQUIREMENTS.md). Release qualification must include
those baselines; a successful build on a newer host does not prove compatibility
with an older OS, glibc or graphics driver.

The workspace release profile enables Rust's highest standard optimization level
(`opt-level = 3`), whole-program **fat LTO**, a single code-generation unit and no
incremental compilation. Release artifacts omit debug information and strip
symbols. Development-only assertions are disabled. The generated main-window
bindings (`midnight-ui-bindings`) alone use `opt-level = "s"`; runtime services,
renderers, font/text engines and other application crates retain level 3.
Build-time host tools retain Cargo's normal build overrides.

The [same-source Windows comparison](verification/2026-11-07-release-optimization.md)
reduced `me.exe` from 42,425,856 to 37,096,448 bytes (12.56%) with overlapping
native resize/redraw timing ranges across six alternating runs per profile.
This is a scoped Windows measurement, not proof of unchanged performance for
every workload or a physical Linux desktop.

Integer overflow checks remain enabled, and `panic = "unwind"` preserves the
backend's worker-panic reporting. File integrity, bounds checks, native identity
validation and explicit confirmation are required behavior in every profile.
There are no unsafe floating-point rewrites or unchecked data-access shortcuts.

The default executable targets the supported platform's portable CPU baseline.
Do not set `target-cpu=native` globally: that would make a redistributed binary
depend on the builder's processor. Explicit machine-specific builds can use that
flag when deployment hardware is known, but must be labeled and tested separately.

PGO is a future measured optimization, not an enabled flag without training data.
It requires an instrumented build, representative browsing/scrolling/search/
operation workloads, matching LLVM profile merging and a rebuild with the profile.
Store generated profiles under `_build/pgo`; never commit them. Compare frame-time
tails, operation throughput and memory, rather than assuming that every additional
optimization always improves performance.

Run `build.bat release` or `./build.sh release` (Cargo uses `--release`), then smoke-test that actual
executable. Adding `--validate` also runs CI's tests for the platform and the
installed smoke test of that release build
([full validation](developer/TEST_EXECUTION.md#full-validation)). These helpers also
build the platform's independent native and network
filesystem packages, the six viewer packages, both thumbnail workers and the archive
worker against the public APIs/SDKs and stage them under `filesystems/`, `viewers/`
and `thumbnailers/` beside the executable. Every module is required.
Text/binary, source and Markdown use the native C ABI; image and PDF viewers use
the restricted process protocol and its separate worker SDK release; the HTML
viewer is a restricted web worker with its CEF runtime.
Direct Cargo builds compile only the host; use `scripts/build-native-filesystem.ps1`
or `.sh`, `scripts/build-network-filesystem.ps1` or `.sh`,
`scripts/build-viewers.ps1` or `.sh`, `scripts/build-default-thumbnailers.ps1` or
`.sh` and `scripts/build-archive-filesystem.ps1` or `.sh` to build its packages
separately. Each rebuild replaces a package's generated notices (and the whole
thumbnail directory), so files of a dependency or FFmpeg version that is no longer
shipped never reach a deployment or archive.
The native helper's test and offline options are documented in the
[native filesystem build instructions](developer/NATIVE_FILESYSTEM_API.md#building-the-native-package).
The network helper has the same staging/test/offline options; its
[guide](../plugins/midnight-filesystem-network/README.md) covers Linux system runtime
requirements and discovery qualification limits. Both use the shared filesystem
package helper, which stages all inputs before replacing an installed library.
Each plugin has
its own lockfile, tests, release profile and third-party notices.
Native tests that exercise integrity and queue/cancellation semantics
must also run under the release profile. Builds take longer and may use more RAM
because of whole-program optimization. Development builds remain available for
fast iteration and debugging.

The helpers in `scripts/`, the Basic, Markdown, Source, image and PDF viewers' build
scripts, the image, PDF and thumbnail plugins' Linux CI scripts, the archive worker's
Linux build script, the image decoder's native helpers in `plugins/image-decoder/native`
and the test runners share one link rule. Generated output lies strictly below the
checkout's own `_build`, recognised however a path spells the checkout, or, outside
the checkout, below the first `_build` of a custom target directory; that `_build` and
every folder below it must not be a link, junction or other reparse point. For a
plugin's scripts, the plugin and the repository holding it in `plugins/` are
checkouts, so a plugin built on its own writes below its own `_build`; the native
helpers, which the image, PDF and thumbnail plugins share, accept the repository's
`_build` and any `plugins/NAME/_build`. The Basic, Markdown and Source viewers may
install anywhere and use the rule only to decide whether notices without their marker
may be replaced. The image, PDF and thumbnail plugins' Windows CI scripts and the
archive worker's Windows build script still check only for a `_build` component (the
image and PDF CI scripts apply the rule only when their build script runs, after the
tests). The
checkout and the folders above it may be links, a `subst` drive or hidden folders such
as `AppData`. An input folder that the caller names (a prebuilt FFmpeg prefix,
`LIBCLANG_PATH`, a Linux `CEF_PATH`) may itself be a link and may lie below links;
nothing below it may be a link or `..`, except the versioned runtime library names of
an FFmpeg prefix, which must resolve inside it (packaging inputs are described below).
The CEF and FFmpeg download caches stay stricter: they must be plain folders inside the
checkout, with no link from the checkout down, because a verified archive is opened
again by path after its checksums are compared. The native helpers keep their caches in
the build roots they are given, as before: nothing in a cached tool tree, the download
folder or the extracted dav1d source may be a link, tool archives are verified in
memory and the dav1d archive is verified and extracted from the same bytes.
`scripts/run-targeted-tests.py` and `scripts/ci-workspace-tests.py` accept Cargo's
paths however they spell the checkout, under the same rule.

On Windows, endpoint protection briefly holds files that were just written. While it
does, renaming them or a folder above them fails with access denied (5) or a sharing
(32) or lock (33) violation, and File.Replace, which writes the runtime registration
records, can report that it could not remove the replaced record (1175), leaving both
files unchanged. The build, packaging and registration scripts therefore retry the
renames of their own fresh output (CEF and FFmpeg downloads and stages, the FFmpeg
receipt, the image decoder's native tool and dav1d downloads, its tool receipt and
dav1d build fixes, the thumbnail and HTML package swaps, the filesystem package files
and the runtime registration records) for about 2 s with a short backoff, and then
fail with the first error. Publishing an extracted native tool directory waits up to
12.7 s instead, because it holds new executables, which Defender's first-sight cloud
check keeps for up to 10 s by default. Other errors, including File.Replace's partial
outcomes (1176, 1177), other platforms and deletions are never retried.
The thumbnail and HTML package swaps use one directory rename, which leaves nothing
behind when it is refused.

Primary references: [Cargo profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
and [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html).

## Distribution archives

The initial 0.1.0 GitHub release is a preview: Windows x64 ZIP and Linux x86-64
GNU/glibc tar.gz, plus `SHA256SUMS`. Linux is built on Ubuntu 24.04; native Fedora,
openSUSE and other desktop qualification remains pending. The Windows executable
is unsigned. Archive checksums establish download integrity, not code signing.

`scripts/package-release.py` uses Python 3.11+ standard-library tools to package an
already built/verified executable. It neither builds nor publishes. All output must
be under the checkout's own `_build`; the checkout may be reached through a link, but
`_build` and every folder below it must not be one. It refuses an existing staging
directory or archive. Every module
input and the README version are validated before the staging directory is created,
so a refused run leaves nothing that blocks the next attempt. The `--readme` file
must begin with `Midnight Explorer VERSION ` for the workspace version. Generate
target-filtered metadata with `cargo metadata --locked --format-version 1
--filter-platform TARGET` and provide it using `--metadata`. Metadata source paths
must be accessible from the packaging host.
Also supply `--dependency-list` from `cargo tree -p midnight-explorer --locked
--target TARGET --edges normal,build --prefix none --format '{p}'`. This uses the
application's actual dependency selection instead of metadata's workspace-wide
feature union, which can include unused renderers. Use the same target/features
as the binary build; retain both runtime and build dependency notices.
Inventory parser regressions run with `python -B -m unittest discover -s scripts/tests -v`
and in CI on both platforms.

Example after the Windows build and checks (PowerShell, Python installed):

```powershell
cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc |
    Out-File -Encoding utf8 _build/windows-metadata.json
cargo tree -p midnight-explorer --locked --target x86_64-pc-windows-msvc `
    --edges 'normal,build' --prefix none --format '{p}' |
    Out-File -Encoding utf8 _build/windows-dependencies.txt
python -B scripts/package-release.py --binary _build/release/me.exe `
    --metadata _build/windows-metadata.json --target x86_64-pc-windows-msvc `
    --dependency-list _build/windows-dependencies.txt `
    --format zip --readme packaging/windows/README.txt `
    --revision (git rev-parse HEAD) --rust-docs "RUST_SYSROOT/share/doc/rust" `
    --license-overrides _build/release-license-overrides --output _build/releases/v0.1.0
```

Replace `RUST_SYSROOT` with `rustc --print sysroot`. For Linux, use the Linux
binary/metadata/target, `--format tar.gz`, `packaging/linux/README.txt`, and
`--desktop-entry packaging/linux/io.github.TomasJelinek.MidnightExplorer.desktop`.
Archives normalize the executable/directories to mode 0755 and documentation to
0644, independent of Windows-mounted staging permissions; tar ownership is cleared
to uid/gid 0 with no builder names. Keep Windows and Linux native
build directories separate when sharing one checkout.

Archives include run instructions, feature/shortcut/security requirements,
`BUILD-INFO.json` with the source revision and executable SHA-256, the lockfile,
dependency inventory and original license texts. The packager requires all six
viewer packages under `viewers/` (or `--viewers-root`), validates their platform,
native ABI, image-worker or web-worker protocol, code entry and package paths (and
the HTML viewer's complete CEF runtime inventory), and records their manifests, file
sizes and SHA-256 values in `BUILD-INFO.json`. Plugin-owned notices remain inside
each package. The packager also requires and ships `midnight-elevated-listing`
(`.exe` on Windows) beside the executable. **Retry with authorization** uses it only
when an administrator (Windows, for example below `C:\Program Files`) or root (Linux,
for example `/opt/midnight-explorer`) installs the extracted folder where standard
users cannot change it; the bundled README explains this. In private per-user
installations, including `build.bat`'s, and in an extracted folder that its user can
change, the retry is still offered but is refused with that requirement. Packages
built before this check used the shipped helper from such a folder; see
[the authorization helper](../SECURITY.md#directory-authorization-helper) for what
their users must replace or delete.
The packager also requires one compatible native and one network filesystem provider
and the `midnight.fs.archive` worker under `filesystems/` (or `--filesystems-root`).
It validates the native protocol, ABI, target, address encoding, capabilities,
library entry and embedded notices before copying any filesystem packages. Network
manifests require ABI 1.2, opaque addresses and explicit activate/refresh/resolve-text
capabilities. Library entries and archive executable entries remain distinct in
`BUILD-INFO.json`. Both the image and the video thumbnail worker are required under
`thumbnailers/` (or `--thumbnailers-root`), with the FFmpeg runtime, its corresponding
source and build notices. Native providers run with application authority.
It rejects links (symlinks, junctions and other reparse points), `..` and other escaped
paths inside module packages, crate license files and license overrides, and missing
notices. Folders above a package root, such as a linked cargo home or Fedora
Silverblue's `/home`, may be links. Package hashes establish
integrity, not publisher authenticity. Native C ABI viewers retain application
authority; image decoding additionally runs in the required restricted worker.
If a crate omits license files
from its published source, supply them in a `NAME-VERSION/` subdirectory of the
directory passed to `--license-overrides`,
from its recorded upstream Git commit, retaining source URLs and hashes. Missing
notices fail packaging; do not substitute an unrelated version's license.
The Rust standard-library copyright inventory is included. The public download
page carries the official Slint attribution badge; the application also retains
its AboutSlint widget. This process does not relicense the application's source.

Before publishing, unpack both archives, compare executable hashes with the tested
artifacts, run the unpacked executables' filesystem/UI smoke checks, inspect ELF
requirements and archive paths/permissions, and verify `SHA256SUMS`. Do not include
private preferences, caches, toolchains, PDBs or generated intermediate objects.
