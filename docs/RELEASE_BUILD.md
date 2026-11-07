# Midnight Explorer release builds

The application package `midnight-explorer` builds `me.exe` on Windows and `me` on
Linux. Native output is `_build/release/me.exe` or `_build/release/me`.

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
executable. These helpers also build five independent viewer packages against
the released public APIs/SDKs and stage them under `viewers/` beside the executable.
Text/binary, source and Markdown use the native C ABI; image and PDF viewers use
the restricted process protocol and its separate worker SDK release.
Direct Cargo builds compile only the host; use `scripts/build-viewers.ps1` or
`scripts/build-viewers.sh` to build its viewer packages separately. Each plugin has
its own lockfile, tests, release profile and third-party notices.
Native tests that exercise integrity and queue/cancellation semantics
must also run under the release profile. Builds take longer and may use more RAM
because of whole-program optimization. Development builds remain available for
fast iteration and debugging.

Primary references: [Cargo profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html)
and [Rust PGO workflow](https://doc.rust-lang.org/rustc/profile-guided-optimization.html).

## Distribution archives

The initial 0.1.0 GitHub release is a preview: Windows x64 ZIP and Linux x86-64
GNU/glibc tar.gz, plus `SHA256SUMS`. Linux is built on Ubuntu 24.04; native Fedora,
openSUSE and other desktop qualification remains pending. The Windows executable
is unsigned. Archive checksums establish download integrity, not code signing.

`scripts/package-release.py` uses Python 3.11+ standard-library tools to package an
already built/verified executable. It neither builds nor publishes. All output must
be under `_build`; it refuses an existing staging directory or archive. Generate
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
dependency inventory and original license texts. The packager requires all five
viewer packages under `viewers/` (or `--viewers-root`), validates their platform,
native ABI or image-worker protocol, code entry and package paths, and records
their manifests, file sizes and SHA-256 values in `BUILD-INFO.json`. Plugin-owned
notices remain inside each package.
It rejects symlinks, escaped paths and missing notices. Package hashes establish
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
