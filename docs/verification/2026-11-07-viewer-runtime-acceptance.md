# Installed viewer runtime acceptance — 2026-11-07

The host-owned `midnight-viewer-ui/examples/native_acceptance.rs` probe opens
actual `ViewerWindows` against the separately installed Basic, Source, Markdown,
Image and PDF packages. It uses a synthetic parent window, authored content,
the default shortcut map and an in-memory copy callback. It does not instantiate
the application, load preferences, save tabs or touch the system clipboard.

The successful Windows run verified all thirteen native steps:

- The viewer matches the parent's physical client size and outer origin on the
  desktop's 225% display scale (2250 × 1575 client pixels).
- Text paints visible Unicode glyphs. Clicking Wrap reflows the same text and
  removes horizontal overflow. Ctrl+F, literal text with a space and Enter find
  a match beyond the first loaded page.
- Binary shows hexadecimal offsets/bytes and printable columns; changing to
  Text replaces that presentation. Space cycles the captured file order.
- Source paints actual green syntax pixels. Markdown removes source markup and
  paints styled headings. The resulting screenshots were visually inspected.
- Image decoding produces expected authored pixels. Actual controls change zoom
  from 100% to 125% and back to 100%; text search is hidden.
- PDF renders the red first page and blue second page. Clicking the next-page
  control and Ctrl+PageUp navigate the real worker output in both directions.
- Escape hides the viewer while preserving the parent window.

Evidence: `_build/viewer-native-acceptance-final.log` and ten PNGs in
`_build/viewer-native-acceptance-final/`. The acceptance steps completed in 5.27 seconds
on this workstation; this is a functional acceptance observation, not a benchmark.
The packages came from
`C:\Users\tjelinek\MidnightExplorer-20261107-113047-bb5e8fef` and were copied with
the private deployment helper into
`C:\Users\tjelinek\_build\viewer-qa-20261107-native`. The probe executable was
built from the current host sources and placed beside those packages. No installed
application executable was run in ordinary mode.

The same trusted installed packages passed the host/engine suites: 25 viewer-engine
tests, 3 image-host unit tests, 3 restricted image-worker tests and 2 restricted
PDF-worker tests. These additionally cover source replacement, cancellation,
deadline termination, retained source reads, large text search, copy, UTF-16,
image panning, stale commands, disabled packages and duplicate plugin identities.
The ordinary renderer/UI suites passed 19 and 7 tests respectively.

Build and verification use `scripts/cargo.ps1` with `CARGO_PROFILE_DEV_DEBUG=0`,
`CARGO_PROFILE_TEST_DEBUG=0` and `CARGO_INCREMENTAL=0`. Commands:

```powershell
pwsh -File scripts/cargo.ps1 build --locked --offline -p midnight-viewer-ui --example native_acceptance
pwsh -File scripts/cargo.ps1 test --locked --offline -p midnight-viewer-render -p midnight-viewer-ui -- --test-threads=1
pwsh -File scripts/cargo.ps1 clippy --locked --offline -p midnight-viewer-ui --all-targets -- -D warnings

# Run under the actual desktop user, with an explicitly trusted installation:
$env:MIDNIGHT_VIEWER_TEST_INSTALL = '<installation directory>'
$env:MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION = '<installation directory>\me.exe'
$env:MIDNIGHT_PDF_VIEWER_TEST_APPLICATION = $env:MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION
pwsh -File scripts/cargo.ps1 test --locked --offline -p midnight-viewer-engine -p midnight-image-viewer-host -- --include-ignored --test-threads=1 --nocapture

# Stage native_acceptance.exe beside trusted viewers/ packages before running:
# Windows: match the application's explicit D3D backend selection.
$env:SLINT_BACKEND = 'winit-femtovg-wgpu'
$env:WGPU_BACKEND = 'dx12'
& '<private stage>\native_acceptance.exe' '<fresh output directory under _build>'
```

Fresh generated BMP/PDF files acquired delayed metadata revisions during early
runs. LiveValidated correctly rejected those revisions. The probe waits for an
unchanged captured revision before opening; if a desktop scanner changes a
fixture later, an explicit second positional argument reuses that generated
`fixtures/` directory read-only while writing screenshots to a new output directory.
The successful run reused `_build/viewer-native-acceptance-04/fixtures/` this way.
No production retry, source validation, sandbox or trust check was weakened.

Native screenshot/control acceptance was performed on Windows. It does not claim
Linux compositor placement, every PDF feature, or all image codecs. The independent
plugin projects retain their own parser/decoder tests and public SDK boundaries.

## Shared build launch through a registered private runtime

The final probe also ran as
`D:\experiments\dev\FM\_build\qa\shared-viewer-probe-profile-20261107\me.exe`, with a
copied viewer tree beside it in the shared checkout. `deploy-private.ps1`
copied that exact executable and packages into
`C:\Users\tjelinek\_build\viewer-qa-profile-20261107` and registered the
matching executable digest under the protected QA profile
`C:\Users\tjelinek\_build\viewer-profile-root-20261107`, in
`.midnight-explorer/plugin-runtimes/`. This matches the final production default
`USERPROFILE/.midnight-explorer/plugin-runtimes`, separate from preferences.
Only the probe child received the QA `USERPROFILE`; `LOCALAPPDATA` retained its
ordinary value. The real preferences and runtime registrations were not touched.

`_build/viewer-native-profile-dx12.log` records the launched D: executable,
the selected protected C: executable, and all thirteen passing native stages
(7.08 seconds). Its ten screenshots are in
`_build/viewer-native-profile-dx12/`. This exercises the production resolver,
native plugin discovery and restricted image/PDF worker discovery through the
actual viewer service. `_build/viewer-native-profile-unregistered.log` is the negative
control: the same shared executable with an empty protected QA user profile
refuses loading with the writable/replaceable-path and missing-registration error.

The final run explicitly set child-only `SLINT_BACKEND=winit-femtovg-wgpu` and
`WGPU_BACKEND=dx12`, matching the application's existing Windows D3D selection.
Earlier probe attempts left backend selection implicit and failed before content
verification with `Failed to create device: Parent device is lost`; the same error
also occurred with the ordinary profile and direct invocation. The production
Windows backend already requires D3D because the local Vulkan driver path is
unreliable. The exact same staged probe passed after selecting DX12 explicitly;
no runtime-resolver or viewer fix was needed for this probe initialization issue.

Before the final registration-path change, an additional
`SLINT_BACKEND=winit-software` diagnostic run passed all thirteen
stages in 9.11 seconds (`_build/viewer-native-registered-software.log` and its
matching screenshot directory). It is supplemental evidence, not a promotion of
the software renderer to a qualified production Unicode fallback. No graphics
settings or drivers were changed. These are viewer-probe results, not claims
about launching the normal application or preserving its real saved state.
