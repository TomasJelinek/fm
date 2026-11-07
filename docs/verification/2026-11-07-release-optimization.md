# Windows release optimization comparison

The selected profile reduces `me.exe` by **5,329,408 bytes (12.56%)**, from
**40.4604 MiB to 35.3779 MiB**, using `opt-level = "s"` only for the generated
`midnight-ui-bindings` crate. The other application crates, runtime services,
renderers and font/text engines retain level 3. Fat LTO, one codegen unit, symbol
stripping, disabled incremental/debug output, overflow checks and unwinding
remain enabled as specified in [release builds](../RELEASE_BUILD.md).

Six interleaved native runs per profile showed overlapping timing ranges without
a consistent regression. The median of the reported resize medians was **1.424 ms
higher** with `s` (87.885 versus 86.461 ms). This is a size improvement, not a
measured performance gain or a universal guarantee. The decision accepts that
bounded Windows result; no physical Linux desktop performance claim is made.
No global `s`/`z`, executable packer, panic-abort setting or machine-specific CPU
target was introduced. `z` was not needed for this accepted scoped reduction.

## Controlled source and build

Both variants used Rust 1.99.1, the locked offline dependency graph and the same
current source. The earlier 41,606,144-byte executable examined in the
[size inspection](2026-11-07-release-executable-size.md) contains older source
and is not the baseline for this comparison.

The baseline command was:

```powershell
./scripts/cargo.ps1 @('build','--release','--locked','--offline',
    '-p','midnight-explorer','-p','midnight-ui',
    '--bin','me','--example','native_resize')
```

The candidate added this CLI configuration before editing the manifest:

```text
--config profile.release.package.midnight-ui-bindings.opt-level="s"
```

The source-input manifest covered workspace crates, public API/worker sources,
branding assets, Cargo manifests/lockfile and compiler configuration. Its SHA-256
was identical before and after both builds:

```text
077cba94fa46781ebfb67dceb6ad59c4170a159c67b6874ee21bd84f86da39f0
```

After qualification, the package override was added to the root `Cargo.toml` and
the same release targets were rebuilt **without** a CLI profile override. Both
resulting executables were byte-identical to the measured candidate. The final
source manifest differs only in `Cargo.toml`, and remained stable during that
verification build:

```text
5ccf3f180c468fee9e1b6aaace6ca34b21e6b1f42c56661cc5fcb1b46f6705e2
```

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| Baseline `me.exe` | 42,425,856 | `1cae8401bad09d18d37f20a481680c8470bb43de29ff02ccf5e8d36cf4dcfdb9` |
| Selected `me.exe` | 37,096,448 | `64fb0f895f2edd34b0c3c57ebf4873d60ad98d916b932ad75e731731047991fe` |
| Baseline `native_resize.exe` | 32,614,912 | `736e96fdd306aafb3dfa7be191869919bd657d9d4a828fa68644942aef6e22de` |
| Selected `native_resize.exe` | 27,232,768 | `b6de36781d9cc47d898068ac9852a3a6fbd7f10163e1b69ce657aa081cbc8660` |

Observed build wall times were 418.444 seconds for the baseline, 328.993 seconds
for the candidate, and 59.019 seconds for the manifest verification. These are
not a controlled build-speed comparison: the baseline included a package-cache
wait, dependency reuse differed, and other platform checks ran during compilation.

## Native runtime measurements

The preserved benchmark executables ran on the actual Windows desktop, with
compilation and other automated GUI work paused. The benchmark explicitly selects
Winit/FemtoVG-WGPU, supplies 10,000 rows in each pane, warms for eight timer steps,
then alternates logical widths 1240/1340 at height 820. Its measures are resize
request to render submission and native redraw-dispatch span, **not compositor
presentation latency or FPS**.

Hardware: Intel Core i7-13800H, Windows 11 Enterprise build 26200. Installed GPUs
are NVIDIA RTX 3500 Ada Generation Laptop GPU (driver 32.0.16.2114) and Intel Iris
Xe (32.0.101.7088). The benchmark does not emit the selected adapter name, so the
result is not attributed to either adapter specifically.

The sequence was `ABBA` repeated three times, where A is the original level-3
profile and B the generated-bindings `s` profile. Each of the twelve runs completed
with **32 samples, exit code 0 and empty stderr**: 192 samples per profile.
An initial three-run comparison had noisy tails; three additional runs per profile
completed the balanced sequence. No further runs were selected or discarded.

The table gives the median of each profile's six reported statistics, followed
by their run-to-run minimum and maximum. These are not pooled 192-sample percentiles.

| Reported statistic | Baseline, ms: median (range) | Bindings `s`, ms: median (range) |
|---|---:|---:|
| Resize median | 86.461 (78.420–88.713) | 87.885 (79.888–89.001) |
| Resize p95 | 114.462 (111.957–123.922) | 115.089 (112.106–122.422) |
| Redraw median | 12.130 (10.806–13.419) | 12.236 (10.296–12.855) |
| Redraw p95 | 17.778 (15.829–29.382) | 17.847 (15.969–30.991) |

The resize-median difference is approximately +1.65%; the redraw-median difference
is +0.106 ms. The ranges overlap, paired differences change sign, and the redraw
p95 central values differ by only +0.069 ms despite individual tails varying
widely. The evidence supports accepting the size reduction without identifying
a consistent Windows resize regression; it does not establish non-regression
for every interaction, data set, graphics driver or operating system.

## Release integrity and image package

Both focused integrity commands passed under the selected release profile:

```text
cargo test --release --locked --offline -p midnight-platform-windows --test transfers
cargo test --release --locked --offline -p midnight-platform-common --lib transfer::tests
```

The five native Windows tests cover independent readback before Unicode
publication, source preservation, collision races, cancellation cleanup,
directory ancestry and non-merging creation. The five common transfer tests
cover short writes, same-length corruption, changed sources, cancellation,
empty files, sync failures and truncation. **10 passed, 0 failed, 0 ignored**;
the common command filtered out 31 unrelated unit tests. Logs retain the exact
commands and results. These focused checks do not replace the coordinator's
broader feature and platform qualification.

The independent image viewer was rebuilt with its own level-3/fat-LTO release
profile, symbol stripping, overflow checks and unwinding. Its complete package
is staged at `_build/release/viewers/midnight-viewer-image/`, including manifest
and dependency notices. `midnight-viewer-image.exe` is **2,291,200 bytes**, SHA-256
`4872e02497600b747823ed2b7d2590f2a298a603a026056ac11dcfff93cb8450`.
This worker is separate from the measured application executable.

## Retained evidence

All generated evidence is under `_build/qa/release-optimization/`:

- `baseline/`, `bindings-s/`, and `selected/`: preserved executables, SHA-256/source
  snapshots, exact argument arrays, elapsed build times and complete build logs.
- `baseline/native-resize-1..6.*` and `bindings-s/native-resize-1..6.*`: per-run JSON,
  stdout and stderr; `comparison.json` contains the checked aggregate results.
- `build-variant.ps1` and `benchmark-variant.ps1`: reproducible local harnesses.
- `hardware.json` and `rustc-version.txt`: measurement environment.
- `windows-transfers.*`, `common-transfers.*`, and `image-plugin-build.log`:
  focused release qualification and independent worker rebuild.

The selected application is `_build/release/me.exe`. No benchmark or toolchain
output is committed or included in the application package.

## Final private Windows installation

The selected executable and all four viewer packages were copied to a fresh
private installation using `scripts/deploy-private.ps1`, including the existing
filesystem and thumbnail workers. Deployment verified source/destination SHA-256
readback, and the installed application hash matched the selected artifact above.

Both the backend `--smoke-test` and native `--ui-smoke-test --snapshot` exited 0
with empty stderr. The native snapshot was visually inspected: both panels,
icon-only location menus, drive buttons, paths, status details, command line and
the entire bottom toolbar rendered correctly on the initial capture. Smoke runs
used temporary in-memory preferences rather than changing the user's settings.
Logs, deployment path and `native-release.png` are retained under
`_build/qa/2026-11-07-final/`.
