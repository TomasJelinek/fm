# Native AV1 decoder and private build tools

AVIF uses [dav1d 1.5.4](https://download.videolan.org/pub/videolan/dav1d/1.5.4/),
licensed BSD-2-Clause. Its source archive is checked against the pinned SHA-256
before extraction. The native wrappers build a release static library (on Windows
with the static C runtime, `/MT`, matching the plugins' `crt-static` Rust code) with
assembly enabled, tools/examples/tests/docs disabled, and no Meson dependency
downloads. The following Cargo build stays offline and must not use dav1d-sys's
Git fallback. Retain dav1d's `COPYING` in both plugin distribution notices.

The separate bootstrap entry points prepare pinned private tools without pip,
apt, a Visual Studio installer, or machine-wide changes. Python 3.12+ is required.
Windows requires 64-bit PowerShell 7 and an installed x64 MSVC toolchain; the
bootstrap reuses the current developer shell or initializes an existing Visual
Studio installation through `vswhere` and `Launch-VsDevShell.ps1`. Linux requires
an x86-64 host, `cc`, `make` and `perl`; NASM is compiled out of tree with the
requested job limit. Native pkg-config discovery also needs `pkg-config` or a
caller-provided `PKG_CONFIG` executable.

## Pinned tools and licenses

| Tool | Pin | License | Source of archive hash |
|---|---|---|---|
| Meson | 1.9.1 universal wheel | Apache-2.0 | [Official PyPI metadata](https://pypi.org/pypi/meson/1.9.1/json) |
| Ninja | 1.13.0 Windows/Linux x86-64 wheels | Apache-2.0 | [Publisher PyPI metadata](https://pypi.org/pypi/ninja/1.13.0/json) |
| NASM | 2.16.03 Windows x64 binary / Linux source | BSD-2-Clause | [Official release archives](https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/); source also matches [Debian's checksum](https://deb.debian.org/debian/pool/main/n/nasm/nasm_2.16.03-1.dsc) |

Ninja reports `1.13.0.git.kitware.jobserver-pipe-1`; this is the executable inside
the pinned 1.13.0 wheels. Exact URLs and SHA-256 values are committed in
`bootstrap-tools.py`. License files remain in the extracted private packages,
and `tool-provenance.json` records the selected archives and licenses. These are
build tools, not shipped application payloads. All permit the intended use
without mandatory payments; retain their notices if redistributing the tools.

Every build/tool root must be an explicit absolute path strictly below a `_build`
directory: in the repository that holds this package in `plugins/`, the repository's
own `_build` or a plugin's (`plugins/NAME/_build`, as a viewer or thumbnailer built
on its own passes it); outside the repository, the first `_build` of a custom target
directory. The repository is recognised however a path spells it, so it and the
folders above it may be links, a `subst` drive or hidden folders such as `AppData`.
That `_build` and every folder below it must not be a link, junction or other
reparse point, and no path may contain `..`. Downloads, extraction, NASM objects,
executables and temporary files stay there. Downloads are checksum-verified and
bounded; archive paths, links, case collisions and expansion limits are checked
before publication. The verified files are used by path, so they stay stricter
than other output: nothing in a cached tool tree or the download folder, nothing
directly below a dav1d root and nothing in the extracted dav1d source may be a link.
Tool archives are verified in memory, and the dav1d archive is verified and extracted
from the same bytes, so no verified archive is opened again by path.

On Windows, the renames of fresh downloads, the tool receipt and the dav1d build
fixes are retried for about 2 s while endpoint protection holds a file just written
(WinError 5, 32 or 33), and then fail with the first error. The rename that
publishes an extracted tool directory waits up to 12.7 s instead, because it holds
new executables, which an endpoint protection's first-sight cloud check can keep for
up to 10 s. Other errors, other platforms and deletions are never retried. On reuse,
immutable tool contents are compared with the verified archive, including rejection
of unexpected files. Verified source timestamps are retained, and NASM build recipes
are regenerated from the pinned source before object reuse.

## Same-shell build commands

Windows, before Cargo in the same PowerShell process:

```powershell
& plugins/image-decoder/native/bootstrap-windows.ps1 -BuildRoot "$PWD/_build/native-tools"
& plugins/image-decoder/native/build-windows.ps1 -BuildRoot "$PWD/_build/dav1d" -Jobs 2
```

Linux, before Cargo in the same shell:

```bash
source plugins/image-decoder/native/bootstrap-linux.sh "$PWD/_build/native-tools" 2
source plugins/image-decoder/native/build-linux.sh "$PWD/_build/dav1d" 2
```

Add `-Offline` to both Windows commands, or `--offline` after the job count on
both Linux commands, to reuse already downloaded archives without network access.
A missing or mismatched archive fails; it never falls back to mutable Git or a
system install. Bootstrap exports private `PATH`, `MIDNIGHT_IMAGE_NATIVE_TOOLS`
and temporary-file paths only in the calling process/shell. The Meson launcher
uses Python `-I -S -B`, verifies the private wheel/tree again, and preserves these
flags in generated build commands. Cwd modules, inherited `PYTHONPATH`, user sites
and startup hooks cannot override the pinned Meson. Native compilation exports the explicit
static configuration consumed by `system-deps`. Linux callers must avoid applying
an unrelated GUI `PKG_CONFIG_SYSROOT_DIR` to the absolute dav1d prefix; scope its
removal to native/image dependency discovery and restore it for unrelated builds.

Run only one bootstrap/native build per root at a time. The preparation lock
covers archive download/extraction, not the later NASM or dav1d compilation.
Different roots can build concurrently within an explicit total CPU budget.
Reuse the same roots for a CI wrapper and its nested build wrapper. Keep all
compilation separate from foreground performance measurements.

Focused validation:

```powershell
py -3 -B -m unittest discover -s plugins/image-decoder/native/tests
```

```bash
python3 -B -m unittest discover -s plugins/image-decoder/native/tests
```

Set `MIDNIGHT_NATIVE_TEST_TOOL_ROOT` to the prepared private tool root to include
the real pinned-Meson test with hostile cwd/Python startup modules. Without that
explicit cache, the integration test is skipped; offline unit guards still run.
On Windows the suite also runs both PowerShell wrappers below a non-ASCII
directory from a console set to code page 850; the bootstrap-windows.ps1 case
needs PowerShell 7 (pwsh) and is skipped without it.
The [native AV1 record](../../../docs/verification/2026-11-12-native-av1.md) has the
exact hashes and limits. AVIF decoding itself is described in the
[decoder README](../README.md).
