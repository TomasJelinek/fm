# Native AV1 decoder and private build tools

AVIF uses [dav1d 1.5.4](https://download.videolan.org/pub/videolan/dav1d/1.5.4/),
licensed BSD-2-Clause. Its source archive is checked against the pinned SHA-256
before extraction. The native wrappers build a release static library with
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

Every build/tool root must be an explicit absolute ordinary path below a directory
named `_build`. Downloads, extraction, NASM objects, executables and temporary
files stay there. Downloads are checksum-verified and bounded; archive paths,
links, case collisions and expansion limits are checked before publication. On
reuse, immutable tool contents are compared with the verified archive, including
rejection of unexpected files. Verified source timestamps are retained, and NASM
build recipes are regenerated from the pinned source before object reuse.
Existing linked output directories are refused.

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
See the
[native qualification evidence](../../../docs/verification/2026-11-12-native-av1.md)
for exact hashes and limits. A compiled library and C lifecycle probe do not
qualify AVIF decoding: actual pixels/alpha, dimension and memory budgets,
malformed input, cancellation, restricted worker and packaging checks must pass
before AVIF is advertised in plugin manifests.
