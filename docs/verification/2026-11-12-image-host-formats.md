# Restricted image-format host qualification

Windows source-build checkpoint, 2026-11-12. This supplements the independent
[decoder and plugin tests](2026-11-12-image-formats.md); it does not replace the
production packaging gate or claim a newly installed main application.

An owned QA probe links the image-viewer host, thumbnail host and thumbnail API
from their exact successful Cargo artifact receipt:
`_build/qa/tree-user-commands-20261112/windows-build.jsonl`. It does not patch host
or plugin application sources. The probe uses the normal trusted discovery,
restricted worker process, content broker and cancellation paths. Fixtures are
inert authored SVG and the attributed AVIF/PNG fixtures from the independent
decoder project. No user's image, application settings or active runtime is changed.

The probe and separately compiled workers were copied with SHA-256 readback to a
new private directory with protected current-user/SYSTEM/Administrators permissions:
`C:/Users/tjelinek/MidnightExplorer-QA-ImageFormats-20261112-81db99e7`.
At this checkpoint, its test-only image manifest additionally advertised AVIF.
The public association awaited production packaging qualification recorded below.

| Artifact | SHA-256 |
| --- | --- |
| Host QA probe | `81db99e7e1877a8a7464e67d75c8a3af7786b23a126794ea1f29c3482fdaf53b` |
| Image worker | `b5330a0ea75d1f7f5c35267719ece2ae75b9bc683e39cd08cdcc5e0a884aec1f` |
| Thumbnail worker | `3ce5c2ae22278d8f7a289a860d039f7bdb9e4d88ce967fec686d1c2de8bf771e` |

The seven actual host/worker tests pass in 7.12 seconds with two concurrent
headless test threads. They verify:

- SVG exact red/green pixels through both viewer and thumbnail protocols.
- Real 33 x 11 AVIF pixels with partial alpha; thumbnails preserve aspect at 15 x 5.
- PNG viewer and thumbnail behavior remains available.
- Viewer zoom uses retained decoded pixels: the source read counter does not
  increase, and enlarged pixels match the original frame.
- Viewer render cancellation, thumbnail pre-cancellation and viewer
  pre-cancellation return errors; pre-cancelled viewing performs no source reads.
- Malformed input and truncated real AVIF never produce a viewer session/frame.

The source, compile log, test log and copied-artifact hash record are retained in
`_build/qa/image-formats-20261112/host-probe/`. This is a native restricted-process
acceptance run, not a desktop rendering/performance measurement. The existing
process sandbox itself has separate security tests. Production package scripts,
notices remain separate qualification steps.

## Linux source-build checkpoint

The same seven host/worker cases also pass on Linux with two headless test
threads (0.05 seconds of libtest execution, excluding compilation/staging).
The probe links exact successful Linux host artifacts from
`_build/qa/tree-user-commands-20261112/build-final.jsonl`. This is another
source-build restricted-process checkpoint, not production packaging validation.

The initial harness mistakenly inherited `TMPDIR` on the shared `/mnt/d` mount.
Trusted discovery correctly refused that writable installation: three thumbnail
protocol checks passed and four viewer checks were refused before decode. Those
results are retained as `linux-restricted-worker-untrusted-mount.*`. The harness
then staged the same bytes under an owned private native Linux `/tmp` directory
with an `_build/runtime` descendant. No discovery or sandbox check was relaxed.
The rerun passes all seven cases; no failing case was filtered out.

| Linux artifact | SHA-256 |
| --- | --- |
| Host QA probe | `e8982e06d9e5017b21ed443d530ec43990bfac09d0cc0a238274ace97d0f1f00` |
| Image worker | `7dc0022f7427f431db50a1ecfd9b1bbc72df87a683a336ba33887833ad9285cc` |
| Thumbnail worker | `a6aafde13afa2315bcbc2c77c496deb2d7b912c98900f0c609bddec7d525db36` |

The exact private location, executable hashes, source/build command and complete
results are retained in `host-probe/linux-probe.sh` and
`linux-restricted-worker.{json,log}` under the same QA root. This measures no UI
pacing; the different Windows/Linux elapsed times include platform worker startup
and should not be interpreted as a decoder throughput comparison.

## Packaged-worker review regression

The first Windows production package passed the original seven cases plus an
actual-manifest association check (eight cases, 8.17 s). Independent review then
found that floating-point SVG sizing could produce 8 x 8 pixels for a 13 x 13
source requested at 7 x 7. A ninth actual-worker case reproduced rejection by the
thumbnail protocol before the fix; the failing result is preserved in
`host-probe/packaged-svg-rounding-red.log`. The initial eight-case checkpoint alone
therefore does not qualify final SVG sizing. The corrected package must pass all
nine cases, including exact requested bounds and pixels, before deployment.

### Final Windows package

After capping the rounded SVG extent to the requested dimensions, the unchanged
production packaging scripts rebuild both workers. All **nine** actual restricted
host cases pass in **8.44 s** with two test threads. They include the same
13-to-7 regression that failed against the previous package, plus actual shipped
SVG/AVIF associations (including case-insensitive lookup), PNG compatibility,
retained-pixel zoom, cancellation, malformed input and partial alpha.

| Final Windows worker | SHA-256 |
| --- | --- |
| Image viewer | `a8a154b67b3b21bf31366e3a65c0120b502a82e88c3e53570c94a2d94e85e3bb` |
| Thumbnailer | `14d06fb724a134cf82129e95c014a56dc97d49f6b383c7d604fbd80fdc1d35fe` |

The source is `host-probe/final-packaged-probe.rs`; logs and exact private-runtime
hashes are `final-packaged-windows-restricted-worker.{json,log}`. Both packages
contain all 213 expected shared notice/source files with exact readback hashes.
Their PE imports contain Windows/system CRT libraries, with no dynamic codec DLL.
The thumbnail manifest uses the new codec cache version, so prior decoder results
cannot masquerade as results from this package. HEIC/HEIF remains unsupported.
This qualifies the workers, not foreground app rendering speed.

## Final Linux package

The exact final production packages pass the same **nine** host/worker cases,
with no ignored tests, in 0.06 s of libtest execution (excluding build/staging).
This is protocol/pixel/cancellation acceptance, not desktop latency or a Windows
versus Linux decoder performance comparison. The probe links exact successful
Linux production host artifacts and uses unchanged shipped plugin manifests.

| Final Linux worker | SHA-256 |
| --- | --- |
| Image viewer | `8546ea70c70114c55aae20e43aac2b9224c6b14618020fd517edf74a55f3917d` |
| Thumbnailer | `6761d005338e109dc3a1854db3fe18e544c06abf81e9596ebb7bcfc32b5fe8af` |

Both packages include all 213 expected shared notice/source files. ELF imports
contain only libgcc_s, libm, libc and the normal loader; neither a dynamic codec
library nor RPATH is required. Actual manifests advertise SVG/AVIF and retain the
r3 thumbnail cache key; HEIC/HEIF is absent.

The first QA copy preserved permissive DrvFS modes through `copytree`. Trusted
viewer discovery correctly refused that native Linux copy: four thumbnail protocol
cases passed and five viewer/discovery cases failed before decode. These results
remain in `final-packaged-linux-inherited-mode-refusal.{json,log}`. The QA
installer now assigns private directory/file modes before discovery, keeping
executables executable; it does not change source packages, manifest contents,
plugin bytes, sandbox policy or host validation. All nine unchanged cases then
pass from the private native `/tmp` installation. The staging script, compile
receipt, installed hashes and complete results are
`host-probe/final-linux-probe.sh`, `final-packaged-linux-compile.log`, and
`final-packaged-linux-restricted-worker.{json,log}` under the same QA root.
