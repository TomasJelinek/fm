# Third-party libraries and licenses

The independent image viewer and thumbnail workers now share
`plugins/image-decoder`: the same pinned raster/color libraries plus resvg/usvg
0.48.1 with only `text` enabled for bounded static SVG. MIT is selected for
resvg/usvg and the existing image-rs dependency; tiny-skia's BSD-3-Clause notices
remain intact. Source-selected image/font/file/network loading is disabled.
Unmodified Noto Sans Regular/Arabic/Hebrew font bytes are embedded under OFL-1.1;
see [source and hashes](../plugins/image-decoder/fonts/README.md). Both worker
packaging paths copy the complete shared `plugins/image-decoder/notices` set,
including font copyright/OFL/provenance. SVG reuses pinned versions. AVIF adds
MIT dav1d 0.11.0/dav1d-sys 0.8.3 bindings, BSD-2-Clause native dav1d 1.5.4 and
MPL-2.0 mp4parse 0.17.0. Both packages retain the full native license and the exact
unmodified MPL parser source archive with its license/source notice. The narrowly
patched image-rs 0.25.10 source is selected only by the three independent image
workspaces; see [patch provenance](../vendor/image/MIDNIGHT_PATCH.md) and the
[shared notice inventory](../plugins/midnight-viewer-image/THIRD-PARTY.md).
No API/application license change is made. HEIC/HEIF remains unsupported pending a
qualified packaged decoder and royalty-free distribution determination.

Managed command artwork reuses the pinned `image` 0.25.10 (`MIT OR Apache-2.0`)
for PNG encoding and pixel resizing only in `midnight-platform-common`; its new
direct declaration enables only `png` with default features disabled. Arbitrary
binary image imports continue through the existing restricted image thumbnailer.
The Linux native icon selector now directly uses `resvg`/`usvg` 0.48.1
(`Apache-2.0 OR MIT`) and their existing `tiny-skia` 0.12.0 (`BSD-3-Clause`)
renderer. The direct `resvg` declaration disables default features. Other UI
consumers may unify features, so the icon renderer explicitly supplies empty font
state and disables both external and embedded-image resolvers; SVG icon rasterization
uses no font-file, network, or compressed-SVG loading.
The exact cached license texts were inspected; MIT is selected for `image`,
`resvg` and `usvg`, and the full BSD-3-Clause notice remains for `tiny-skia`.
The existing release notice collector retains all their upstream license and
copyright files under `licenses/<crate>-<version>/`. No new package version,
mandatory fee, subscription or royalty is introduced.

The application now enables Slint 1.18.1's existing `accessibility` feature in
its normal workspace declaration. Resolution adds eighteen exact package pins,
with no removed or upgraded package. The Windows application graph selects
`accesskit` 0.24.1, `accesskit_consumer` 0.38.0, `accesskit_windows` 0.34.0 and
`accesskit_winit` 0.33.2; Linux/AT-SPI and Apple adapters in the lockfile do not
establish runtime qualification on those platforms. The actual resolved source
archives, checksums, manifests and `.cargo_vcs_info.json` records were checked.
All eight AccessKit packages use the **Apache-2.0** alternative; the other ten
added packages retain their inspected free MIT or MIT/Apache declarations.
The published AccessKit archives omit standalone license texts. Version-specific
[supplements](../packaging/licenses/accesskit-0.24.1/SOURCE.md) retain the exact
upstream Apache, MIT and Chromium BSD notices from each recorded commit, including
source URLs and SHA-256 hashes. Pass `packaging/licenses` to the existing release
collector's `--license-overrides`. The unchanged collector passes for the exact
eighteen-addition notice scope and for the **325-package Windows application
normal/build selection**, including the real pinned Rust standard-library
copyright file; all 637 copied crate-notice files were compared with their
sources. The first full-app attempt failed on eleven pre-existing packages whose
archives also omit root licenses. Their version-specific supplements now retain
matching-commit texts and provenance: clipboard-win uses BSL-1.0;
gl_generator, khronos_api, profiling and spirv use Apache-2.0; Lyon's four crates,
rspolib and taffy use MIT. The original alternative notices remain intact.
`khronos_api` additionally retains the exact pinned ANGLE BSD license and the
original Khronos/EGL/WebGL copyright/permission comments, rather than treating
all embedded definitions as Apache-only. The failed baseline remains recorded;
notice staging is not a packaged executable or runtime qualification.

The scoped RustSec review of the eighteen added package names finds no advisory
files at database commit
[`472a4c9a785030434f15d7da84964d60fbe789ab`](https://github.com/RustSec/advisory-db/tree/472a4c9a785030434f15d7da84964d60fbe789ab).
It includes withdrawn/informational entries in that search and is **not** a
full-lock or newly enabled-feature vulnerability scan or proof of safety.
The full forty-six-pair native-host inventory refresh and generator `--check`
now pass, with the retained capture provenance below. No paid branch, new fee,
application/SDK license change or accessibility runtime qualification is implied
by these dependency and notice checks.

The directory authorization helper adds a project-owned crate and reuses the pinned
`getrandom` 0.3.4 (MIT OR Apache-2.0), `windows-sys` 0.61.2 (MIT OR Apache-2.0),
and `libc` 0.2 (MIT OR Apache-2.0) dependencies already present in the inventory.
Its twelve Windows API namespace features cover foundation types, filesystem, I/O,
threading and process, named-pipe, memory, registry, security and security
authorization, COM initialization, shell and window-message declarations. They add
no package, version or feature to the application: other application crates select
each of them too (for example `midnight-platform-windows` the COM, registry and
window-message features, and `midnight-worker-platform` the named-pipe feature).
The helper contains no plugin discovery or loading entry point. Existing notices apply.

Make File List adds no third-party library or enabled feature. Its engine reuses
the workspace's `midnight-worker-platform` private spool and existing transfer,
clipboard, native picker and viewer services.

`midnight-platform-common` no longer uses `fs2` 0.4.3 (unmaintained since 2018):
command-icon snapshot locking uses the standard library's file locks, and Unix
filesystem capacity uses the already pinned `rustix` 1.1.6 `statvfs` with its
already selected `fs` feature (Windows adapters query capacity natively). This
removes `fs2` and `winapi` 0.3.9, which only `fs2` used, together with the
platform-excluded `winapi-*-pc-windows-gnu` 0.4.0 shims, from the application
lockfile. The generated tables were refreshed by targeted row comparison
(for this removal alone, **644 records / 526 names / 65 direct records**; `rustix`
gains this declaring crate), not by a full recapture or generator `--check`; the
combined result with the `notify` removal is recorded below.

The additional color themes add no Cargo library or feature. Modern palette
references use Apache-2.0 (Tokyo Night) and MIT (One Dark, Rosé Pine and Catppuccin), with
[pinned provenance, adaptation notes and retained license texts](THEME_PROVENANCE.md).
Release packages carry that record and `licenses/themes/` alongside library notices.

Inventory regenerated and checked on **19 November 2026** from all **46 fresh
metadata/tree pairs** and frozen manifests/lockfiles: 22 native Windows x64,
22 native Linux x86-64 captures under existing WSL Ubuntu 24.04, and the two
explicit Darwin cross-target exceptions for the independent native macOS
filesystem package only. Rust/Cargo 1.99.1 and the existing immutable SDK bundles
were used; captures, command/host receipts and hashes are retained under
`_build/qa/accessibility-feature-20261119/`. The earlier 12/17/18 November
counts and targeted row edits below remain historical. The current tables are:

- [Direct dependencies](dependencies/DIRECT_DEPENDENCIES.md): **69**
  package/version records declared by project crates, including build/test tools.
- [Complete Cargo inventory](dependencies/CARGO_LIBRARIES.md): **654**
  package/version/source records, **536** distinct crate names, including transitives.
- [Machine-readable CSV](dependencies/cargo-licenses.csv): the same complete list,
  with licenses, source, workspace, target and direct dependants.

The 19 November AccessKit refresh added twelve inventory records: six
selected AccessKit packages, three AT-SPI packages and three zbus support crates.
Four `phf` 0.13.1 records already belonged to the PDF scope and now also belong to
the application. The eighteen lockfile additions additionally contain Apple-only
adapters that are not selected by these application targets. Existing rows retain
their license/source/version declarations; current Find/tooltip, Media and
work-budget direct-dependant edges are refreshed rather than new dependency pins.

The subsequent integration reused the shared Slint writer for comparator bindings.
This removed the app's last prettyplease dependency. The current tables were
checked against fresh Windows and Ubuntu captures for the app and renderer
workspaces.

On 20 November 2026 the independent image thumbnailer
(`plugins/midnight-thumbnail-image`) gained first-page PDF thumbnails with the PDF
viewer's renderer: a direct `hayro` 0.9.0 (Apache-2.0 OR MIT) dependency with the same
features (`embed-fonts`, `embed-cmaps`, default features off). Its lockfile adds 41
packages, every one already pinned by the PDF viewer at the same version and license
(the Hayro crates, vello_cpu/vello_common 0.3.0, pic-scale 0.7.12, brotli 8.0.4,
read-fonts 0.46.0 and skrifa 0.49.0 beside the decoder's 0.41.0/0.44.0, and their
dependencies); no existing entry changes version. In the thumbnailer's graph moxcms
0.8.1 gains `in_place` and flate2 1.1.11 its `zlib-rs` backend, the features the PDF
viewer selects. No new package, version or license enters the inventory: 43 rows gain
the `thumbnail-image` scope (40 of the 41 additions, all but `glifo` 0.4.0, which no
captured tree selects and the inventory does not list, plus `euclid` 0.22.14,
`hashbrown` 0.17.1 and `zlib-rs` 0.6.8, which the lockfile already held but the graph
selects only now), and `hayro` gains `midnight-thumbnail-image` as a declarer. The tables stay at **654 records / 536 names /
69 direct records**. Linux-host captures of the thumbnailer for both targets
(`cargo metadata --locked --offline --filter-platform <target>` and `cargo tree --locked
--offline --workspace --target <target> --edges normal,build,dev --prefix none`) were
selected with the generator's own code; rendering the previous rows with its `render`
reproduces the committed tables byte for byte, and the same code wrote the changed rows.
This is a targeted refresh, not a full 46-pair recapture or generator `--check`.
The About list therefore names the image thumbnails beside the PDF viewer for these
libraries. The thumbnail package keeps the notices that crate packages omit, from the
plugin's own `notices/` folder (the PDF viewer's reviewed copies): the embedded Foxit
standard fonts' and Adobe CMaps' BSD-3-Clause notices, the CC0 color-data and JPEG 2000
profile notices, Hayro's upstream NOTICE and alloc-stdlib 0.2.4's BSD-3-Clause license.
`scripts/thumbnail-notices.py` adds that folder under `licenses/`.

The first generator attempt exposed older `midnight-plugin-api`/`midnight-plugin-sdk`
0.1.0 releases in the independent Basic/conformance graphs. These are project
code, not unknown-license third parties. The narrow generator repair recognizes
historical versions only for an actual non-vendored workspace-member name from
the **exact** approved registry
`registry+https://midnight-explorer.invalid/api-index`; it still traverses their
external dependencies and preserves existing identity handling. Historical versions
from crates.io, foreign registries or URL-suffix lookalikes remain external, as do
name lookalikes and vendored members. The meaningful regression fails before the correction; all
**12 focused inventory tests** then pass, including those negatives. No license
is invented or assigned to first-party SDK code.

The twenty-three workspaces cover the application, public viewer/filesystem/thumbnail/image/web
APIs and worker SDK, six independent viewer plugins, archive/image/video plugins,
the independent native Windows/Linux/macOS Browse/Read filesystem packages,
the independent Windows/Linux Network Neighborhood plugin, and the
independent viewer ABI conformance project and vendored renderer tests. Project-owned
crates are excluded. The negative descriptor fixture has no third-party dependencies.
Target graphs include normal, build and test dependencies; this is broader
than the libraries linked into `me.exe`/`me`. Unused lockfile entries and optional
packages present only in Cargo metadata's feature union are excluded using
`cargo tree`. The complete list links to each workspace's maintained lockfile.

The scrolling renderer correction vendors the same FemtoVG **0.27.0** under its
MIT alternative, retaining both upstream license texts and
[exact source provenance and patch details](../vendor/README.md). Application
package/version selections are unchanged. The same local source also guards
feature-specific imports/helpers so the app's `swash,wgpu` feature set compiles
without unused-item warnings; that correction changes no dependency, license or
feature selection (the later `skrifa` edge is recorded under 18 November below).
Its lockfile changes only the
renderer source from the registry to the local patch. The `renderer-tests` scope
accounts for the independent vendored test workspace with
`--no-default-features --features wgpu`, including its normal, build and development
dependencies. The standalone lock retains upstream pins except the Windows API
family compatibility correction documented with the vendor source. The broader
test graph adds 95 net inventory records and 17 crate names; these are not new
application runtime dependencies.

Application and renderer-test metadata/tree pairs were captured on Windows and
WSL Ubuntu 24.04 on 11 November 2026; the other forty target pairs retain their
previous capture provenance. The renderer-only `harfrust` 0.5.2 archive declares
MIT in its manifest/README and notes its HarfBuzz Old MIT origin, but omits a root
license file. `resource_list_proc_macro` 0.6.1 declares CC0-1.0 and shares its
recorded upstream commit with `resource` 0.6.1, whose workspace archive includes
the complete CC0 text. These published-source details do not change their
verbatim declarations in the generated inventory.

All entries have a declared license. Alternatives are retained verbatim; they do
not mean all alternatives are selected. The mandatory free-of-charge requirement
and Slint's selected royalty-free branch are in [the policy](DEPENDENCY_POLICY.md).

Normal Linux HTML builds also require CMake and Ninja. The 13 November Ubuntu
24.04 build container used CMake `3.28.3-1build7` (upstream BSD-3-Clause, retaining
its component notices) and Ninja `1.11.1-2` (Apache-2.0, retaining its bundled
component notices). The installed distribution copyright files were inspected;
both tools permit the intended build use without mandatory fees. They are build
tools rather than libraries shipped with ME. The exact version/license capture is
`_build/qa/main-integration-20261113/html-build-tools.txt` and its adjacent package
version receipt. CEF's existing runtime notices remain separate.

The HTML provider, included by normal builds, adds pinned `cef` / `cef-dll-sys`
`156.4.0+156.0.26`, with only `sandbox` enabled and MIT selected for the Rust
bindings. Its separately packaged Chromium Embedded Framework runtime is
`156.0.26+g8ea0c73+chromium-156.0.8078.88`. The main executable does not link CEF.
CEF's BSD license does not replace Chromium's bundled-component licenses;
the package retains its full `LICENSE.txt`, generated `CREDITS.html`, and the
locked Rust packages' copyright/license files. LGPL, MPL, Unicode/data and
other component conditions still apply. See the provider's
[pinned runtime, licenses and source-availability record](../plugins/midnight-viewer-html/THIRD_PARTY.md).
No paid CEF wrapper, Widevine or proprietary codec build option is selected.
The new Windows/Linux target Cargo selections were captured from the Windows
builder, including its build-tool dependencies; this is dependency accounting,
not a claim of native Linux Chromium qualification.

The Linux HTML package prepared on 10 November additionally supports explicitly
bundled Ubuntu 24.04 amd64 runtime libraries when the host does not supply them:

| Native package | Exact package version | License in the pinned package |
| --- | --- | --- |
| NSS (`libnss3`) | `2:3.98-1ubuntu0.3` | MPL-2.0; bundled components retain Zlib, BSD and other notices |
| NSPR (`libnspr4`) | `2:4.35-1.1build1` | MPL-2.0 |
| ALSA (`libasound2t64`) | `1.2.11-1ubuntu0.3` | LGPL-2.1-or-later |

These are separate dynamically loaded native libraries, not additions to the
Cargo inventory or the main executable. The package preserves their exact binary
hashes, original copyright files, complete license texts and matching original
source archives plus distribution patches/build scripts under `sources/native/`.
Retain that bundle when distributing this variant; compatible rebuilt shared
libraries remain replaceable. Source details and the verified CEF archive hashes
are in the [HTML provider record](../plugins/midnight-viewer-html/THIRD_PARTY.md).
No new Rust dependency or feature is needed for Linux's private browser socket
directory or for native tree-watch batching.

The streaming-worker SDK reconciliation on 9 November upgrades only project-owned
packages: worker SDK 0.2.0, image-viewer SDK 0.3.0, thumbnail SDK 0.2.0 and
filesystem API/SDK 0.3.0. Twenty metadata/tree pairs were recaptured on native
Windows and Linux for the application, affected public API workspaces and five
independent archive/media consumers. The external dependency records and enabled
features remain unchanged; regeneration and `--check` retain 490 records, 458
names and 62 direct records, with byte-identical generated inventory tables.
The existing MIT alternatives for windows-sys 0.61.2 and libc 0.2.191 were
rechecked against their pinned license files. Image/PDF notices cover both target
graphs; refreshed thumbnail notice staging now includes the Windows/Linux union
(36 image and 29 video records), including Linux-only libc/cfg-if notices.
FFmpeg's exact `build-options.txt`, corresponding source and configuration survive
release staging unchanged. See the [SDK consumer and notice verification](
verification/2026-11-09-worker-sdk-media-consumers.md).

Custom archive suffixes reuse the already pinned `unicase` 2.10.0 in
`midnight-core` as well as `midnight-engine`. Its exact manifest and MIT notice
were inspected; MIT is the selected branch, and no optional features or additional
packages are enabled. Release notice generation already retains this package's
copyright and license files. The direct-dependant inventory includes both crates.

Viewer whole-word Find and text-comparison Find also reuse `unicase` 2.10.0
and `unicode-segmentation` 1.13.3 directly in `midnight-viewer-engine` and
`midnight-compare-engine`. The exact cached manifests and MIT license files were
inspected. Both use the existing MIT alternative without optional features;
there are no additional resolved packages, paid components or changed application
license. Existing distribution notice collection retains their license files.

Comparator tooltip compilation reuses the existing host adapter and its exact
`i-slint-compiler` 1.18.1 (`rust`), `spin_on` 0.1.1, `prettyplease` 0.2.37 and
`syn` 2.0.119 (`full`) build dependencies. Its geometry tests also directly use
`i-slint-core` 1.18.1 (`std`). These match the existing host selections. The cached
manifests and selected license texts were checked: Slint keeps the royalty-free
desktop alternative and its existing attribution, while the other three use MIT.
All 615 resolved package/version/source/checksum identities match the branch base;
only direct-dependant edges changed. Existing notice collection and the AboutSlint
attribution remain applicable. These helpers stay in the application, outside the
released plug-in API/SDK.
The native filesystem host adds only project-owned API dependencies and new
dependants of already pinned serde, toml and test-only tempfile. The application
graphs were recaptured on Windows and Linux on 8 November; other workspace
dependency selections are unchanged. Regeneration and `--check` retain the same
490 package/version/source records, 458 names and 60 direct records.

The subsequent Windows renderer correction enables Slint's `unstable-wgpu-30`
configuration API to select DX12 explicitly. WGPU 30.0.1 and rgb 0.8.53 were
already selected; their MIT licenses were checked. Resolution preserves all
610 existing lockfile package pins and adds only optional Android metadata:
`android-build` 0.1.4 (MIT) and `i-slint-backend-android-activity` 1.18.1 (the same
Slint Royalty-free 2.0 alternative). Their pinned license files were inspected.
Neither Android package nor Skia enters the Windows/Linux selected build graphs.
Earlier Browse/Read native Windows captures cover additional modules of pinned
windows-sys 0.61.2 under its MIT alternative, with the full notice retained.
The targeted prepared-Rename capture below updates that source selection.

The integrated prepared-Rename native Windows **0.2 source** adds runtime features
`Wdk_Foundation`, `Wdk_Storage_FileSystem`, `Win32_Globalization`,
`Win32_Security`, `Win32_Security_Cryptography` and `Win32_System_Threading`
to pinned windows-sys **0.61.2**, with defaults disabled. Its normal feature tree
has **19** namespace features; the test/compiled union has **20**, adding only
`Win32_System_LibraryLoader`. windows-link remains **0.2.1**. The selected **MIT**
alternative and complete retained notices are unchanged; no separate cryptographic
library or new third-party package/version is introduced.

The targeted Windows-host/Windows-target metadata/tree captures select windows-sys
and windows-link for `native-windows`, and no third-party package for
`native-linux`. The Linux-target libc **0.2.191** `std` declaration/checksum is
unchanged; no new Linux-host capture or Linux execution is claimed. Both plugins'
own 0.1-to-0.2 versions are project-owned and excluded. The targeted package/row
comparison preserves the generated package/license tables and their recorded
provenance: **646 records / 528 names / 66 direct records**. This is a feature
refresh, not a full 46-pair recapture or generator `--check`.

On 17 November 2026 `midnight-platform-windows` replaced `notify` with its own
`ReadDirectoryChangesW` service. notify was a `cfg(windows)` dependency of that crate,
so only the Windows application graph changes. A targeted Windows-host
`app`/`x86_64-pc-windows-msvc` metadata/tree capture (`--locked --offline`, edges
`normal,build,dev`), selected with the generator's own code, contains none of
notify 8.2.1, notify-types 2.2.0, walkdir 2.5.0, same-file 1.0.6 or winapi-util
0.1.11. The first four remain Linux application dependencies through
`midnight-platform-linux`; winapi-util leaves the inventory. Exactly those rows
changed (notify keeps only `midnight-platform-linux` as its declarer), giving
**645 records / 527 names / 66 direct records**; re-rendering the edited rows with
the generator reproduces all three tables byte for byte. The crate's two added
windows-sys features (`Win32_Foundation`, `Win32_System_WindowsProgramming`) leave
the application's windows-sys 0.61.2 feature union unchanged: notify had enabled
both, together with `Win32_System_Threading`, `Win32_Storage_FileSystem`,
`Win32_Security` and `Win32_System_IO`, which the crate already enables.
`Win32_Foundation` is also enabled by `midnight-elevation`, `midnight-worker-platform`
and other crates; `midnight-platform-windows` is now the only enabler of
`Win32_System_WindowsProgramming` (mio and socket2 are not in the Windows
application graph). No package, feature union or license selection changed. This
is a targeted refresh, not a full
46-pair recapture or generator `--check`; no Linux capture was taken, because the
removed edge was `cfg(windows)` only. Together with the `fs2`/`winapi` removal above,
the tables hold **643 records / 525 names / 65 direct records**: `cargo tree -i` on the
merged lockfile reaches neither `winapi` (gone from the lockfile) nor `winapi-util`
(still locked only as walkdir's Windows-only dependency, while walkdir itself is now
Linux-only) for the Windows or Linux target.

On 18 November 2026 the `midnight-ui-bindings` build script stopped formatting its
generated Rust with `syn`/`prettyplease` and writes the token stream itself
([record](verification/2026-11-18-slint-compile-time.md)). Its build dependencies
lose `prettyplease` 0.2.37 and `syn` 2.0.119 and gain a direct `proc-macro2` 1.0.107
edge (MIT OR Apache-2.0, already in the graph through `quote` and `i-slint-compiler`).
`prettyplease` leaves the application graph: the root lockfile still records it
for `bindgen` 0.72.1, which no application target selects, and Linux-host
`cargo tree --locked --offline -e normal,build,dev --target x86_64-unknown-linux-gnu
-i prettyplease@0.2.37` prints nothing. The same query with `-e features` shows the
`syn` 2.0.119 union (clone-impls, default, derive, extra-traits, full, parsing,
printing, proc-macro, visit, visit-mut) and the `proc-macro2` union (default,
proc-macro) unchanged. Exactly those rows changed, giving **642 records / 524 names /
64 direct records**. This is a targeted refresh, not a full 46-pair recapture or
generator `--check`: no Windows capture was taken, and the Windows result follows
from the lockfile, where `midnight-ui-bindings` and `bindgen` are the only packages
that name `prettyplease`. The build-graph notice staging below predates this change.

On 19 November 2026 `midnight-ui-bindings` also names `proc-macro2` 1.0.107 under
`[dev-dependencies]`, for the test target of its build script's token passes
(`tests/build_passes.rs`; [record](verification/2026-11-19-slint-build-speed.md)).
It is the package, version and default feature set (default, proc-macro) that the
crate already uses as a build dependency: no package, version, license or feature
changes, `Cargo.lock` is unchanged (`cargo build --locked` and `cargo test --locked`
accept it), and the record counts stay as above.

Also on 18 November 2026 the vendored FemtoVG 0.27.0 gained a selectable text fitting
([vendor notes](../vendor/README.md), [record](verification/2026-11-18-ui-sharpness.md)).
Two dependency edges change and no package, version, license or feature does:

- `femtovg` now depends directly on `skrifa` 0.44.0 (MIT OR Apache-2.0), optional
  and enabled by its existing `swash` feature, with default features off and
  `std`. swash 0.2.10 already brought that package into the application, so its
  row is unchanged; both lockfiles only add the name to the `femtovg` entry. The
  application graph already selects `skrifa`'s `default` feature, which includes
  `std` (Linux-host `cargo tree --locked --offline -e features -i skrifa@0.44.0
  -p midnight-explorer --target x86_64-unknown-linux-gnu` lists `default`, `std`
  and `autohint_shaping`), so the feature union is unchanged.
- `midnight-explorer` now names `femtovg` itself, to choose the fitting before its
  first window, with `default-features = false` and `features = ["swash"]`: the
  version and features Slint's renderer already requires, resolved by the same
  `[patch.crates-io]` entry to `vendor/femtovg`. The vendored package thereby
  becomes a direct record: **642 records / 524 names / 65 direct records**. The
  three generated tables were edited for that one row; as above this is a targeted
  refresh without a Windows capture or generator `--check`.

The local source remains under the upstream MIT OR Apache-2.0 choice and release
notices are unchanged: the collector already copies both FemtoVG license files.

On 19 November 2026 two manifest changes made the application build and a targeted
`cargo test -p midnight-ui` or `-p midnight-viewer-ui` resolve the same features
below Slint, so that Cargo can use one compiled Slint and one compiled copy of the
generated UI bindings for both. No package, version or license selection changes;
`Cargo.lock` gains three dependency edges of `midnight-ui-input` and nothing else.

- The workspace `slint` 1.18.1 dependency names `unstable-winit-030` and
  `unstable-wgpu-30` for every member. `midnight-explorer` already selected both,
  so the application's features are unchanged. A package selection without
  `midnight-explorer` or the `midnight-viewer-ui` tests now gains
  `unstable-wgpu-30` on `slint` and `i-slint-backend-selector` (and
  `unstable-winit-030` when only `midnight-ui-bindings` is selected); `i-slint-core`
  and `i-slint-backend-winit` had identical features in every selection before and
  keep them.
- `midnight-ui-input` declares `tracing-core` 0.1.36 (MIT), `indexmap` 2.14.2
  (Apache-2.0 OR MIT) and `lazy_static` 1.5.1 (MIT OR Apache-2.0) without using
  them. The first two carry their `default` feature (`tracing-core`: `default`,
  `std`, `once_cell`; `indexmap`: `default`, `std`), which `tracing-log` and `zip`
  already enable in the application; `lazy_static` has no feature and is already a
  normal application dependency through `sharded-slab`. The pinned license files of
  the three packages were inspected. All three were locked at these versions and
  are linked into the application already, so release notices are unchanged.

The three packages become direct records declared by `midnight-ui-input`; the
tables hold **642 records / 524 names** as before and list 69 direct rows (66
before this change, counted in the tables). Rendering the previous rows with the
generator's own `render` reproduces the three committed tables byte for byte, and
the same code wrote the three changed rows. A native Linux `app` capture
(`cargo metadata --locked --offline --filter-platform x86_64-unknown-linux-gnu` and
`cargo tree --locked --offline --workspace --target x86_64-unknown-linux-gnu --edges
normal,build,dev --prefix none`), selected with the generator's own code, lists
these three declarations, and all of its 400 third-party records are listed for
`app` and Linux (its eight other records are project-owned API/SDK crates, which a
complete run excludes). It also reports `midnight-work-budget` as a declarer of `tokio`
1.54.1 (its optional `resident-workers` dependency), which the direct-dependant
column does not name; that row is older than this change and was left as it is.
This is a targeted refresh, not a full 46-pair recapture or generator `--check`.
No Windows capture was taken: the Windows-only packages are not in this host's
offline cache, and the Windows rows follow from the three declarations being
unconditional. Whether Cargo then reuses the compiled units is a build result,
not a dependency fact, and is not recorded here.

On 18 November 2026 the dependency changes since commit `48ec78d` were rechecked
(`git diff 48ec78d..HEAD` of `Cargo.lock` and every `Cargo.toml`). Besides the two
removals above, they change only:

- project-owned path edges: `midnight-platform-windows` and `midnight-explorer` now
  depend on `midnight-worker-platform`, and `midnight-explorer` uses
  `midnight-plugin-host` as a normal instead of a development dependency;
- windows-sys 0.61.2 features already in each union: `Win32_System_Com` for
  `midnight-elevation`, and `Win32_System_IO`, `Win32_System_Ioctl` and
  `Win32_System_SystemServices` for the native Windows plugin's tests, which its
  normal declaration already selects (its 19 normal and 20 test/compiled features
  are unchanged);
- the `rustix` 1.1.6 `fs` declaration in `midnight-platform-common` described above;
- the archive plugin's own version (0.2.0), a project-owned package.

The Linux clipboard's eventfd wait and the Linux access-ACL reads use the `event`
and `fs` features that `midnight-platform-linux` already declared (and the native
Linux plugin's existing libc `std`), so no feature changed. Fresh `--locked --offline`
captures selected with the generator's own code reproduce the inventory for these
scopes: all 320 `app`/Windows records are listed for `app` and Windows (the five
other rows listing both come from the Linux application and other workspaces'
Windows graphs), every application crate's direct declarations match the
direct-dependant column, and the native Windows plugin's two records match. A
Windows-host capture for the Linux target, which is not equivalent to a native
Linux capture for host-dependent build dependencies, found no `app`/Linux record
missing from the inventory. No row changed. That recheck took no native Linux or
Darwin capture; the complete check that followed it is described next.

Later on 18 November all forty-six metadata/tree pairs were captured fresh for
integration tip `93d36411` with the build and test changes of the next review batch:
22 on Windows, 22 on native Linux (WSL Ubuntu 24.04) and the two macOS cross-target
pairs of `native-macos`. `scripts/dependency-inventory.py --check` passed against the
committed tables, so they hold **643 records / 525 names / 65 direct records**
([record](verification/2026-11-18-workspaces-and-opt-in-qualification.md#dependency-inventory)).
No `Cargo.toml`, `Cargo.lock`, `.cargo/config.toml` or toolchain file of any
workspace, and nothing under `vendor/`, changed between the recheck commit `5a0c23f2`,
`93d36411` and the review's integration tip `d0f44c55` (`git diff --stat 5a0c23f2
d0f44c55 -- '*Cargo.toml' '*Cargo.lock' '*.cargo/config.toml' 'rust-toolchain*'
vendor/` is empty), so the tables also describe `d0f44c55`.

That recheck covers Cargo package, version and feature selections. The vendored,
patched sources also changed since `48ec78d`, with unchanged versions and licenses:
the image-rs 0.25.10 AVIF patch now also ends the decoder's wait when an AV1 item
produces no displayable frame ([patch provenance](../vendor/image/MIDNIGHT_PATCH.md)),
and the FemtoVG 0.27.0 WGPU patch hardened its binding cache and upload staging
([vendor notes](../vendor/README.md)). Those two records describe the current
patches. The copy that the image viewer and the image thumbnailer distribute,
`plugins/image-decoder/notices/image-0.25.10/MIDNIGHT_PATCH.md`, is identical to the
vendored record, and the image viewer's `THIRD-PARTY.md` names both changes.

Windows/Linux native 0.2 source is integrated in D merge `5baee471` from C commit
`42d45def` (25 matching paths). The plugins are not deployed and engine/UI cutover
remains unimplemented; installed-provider/native-runtime qualification is separate. See
[provider status](ACTIVE_WORK.md#provider-increments-from-the-architect-review).

## Office contents preview host service — 17 November 2026

The new host-owned `midnight-office-preview` crate uses quick-xml **0.41.0**
(MIT), zip **8.6.0** (MIT), and flate2 **1.1.10** (MIT OR Apache-2.0; MIT selected).
It depends on host Core, not private independent-plugin implementation crates.
The tested XLSX backend service and approved six-format design do not establish support
for all six formats or a qualified app/distribution.

Build's `viewer-refresh-20261117/build/office-lock-integration/ready.json` records
locked offline metadata success: only local Office plus external zip8.6.0 and
typed-path0.12.4 were added; all prior versions/checksums stayed unchanged.
Office disables defaults on all three direct dependencies, requesting only
`zip/deflate-flate2` and `flate2/rust_backend`. Actual Windows workspace unification
selects ZIP `_deflate-any,deflate-flate2`, typed-path `default,std`, flate2
`any_impl,default,miniz_oxide,runtime_detection,rust_backend`, and quick-xml
`default` (an empty default set). ZIP AES/bzip2/deflate64/LZMA/PPMd/time/xz/zstd
features are not enabled. flate2's weak optional zlib-rs reference is not proof
that the lockfile's zlib-rs package is active.

The 14-package feature-audited registry subtree retains MIT alternatives except
foldhash0.2.0's Zlib license. Exact cached license texts were inspected; selected
copyright/permission texts are retained in the [host notice record](dependencies/OFFICE_PREVIEW_NOTICES.md),
with complete upstream files/hash pins staged under the private review artifacts.
The existing release collector must include the actual host-selected dependency
closure under `licenses/`; old archive-plugin attribution alone is insufficient.
No mandatory fee, subscription or royalty is required by these inspected licenses;
no project-license change or paid alternative is selected.

Security review remains open: RustSec extraction advisories
[RUSTSEC-2026-0429](https://rustsec.org/advisories/RUSTSEC-2026-0429.html),
[0369](https://rustsec.org/advisories/RUSTSEC-2026-0369.html),
[0368](https://rustsec.org/advisories/RUSTSEC-2026-0368.html) and
[0367](https://rustsec.org/advisories/RUSTSEC-2026-0367.html) list fixes in 9.0.0.
The cached8.6 extraction allocation matches0429; current Office source uses a
captured ZIP reader without an `extract()` call. That narrow observation is not
complete reachability clearance. Prior 0168 is patched from 2.3.0. A complete
transitive maintenance/advisory review, selected Cargo-tree captures, Linux Office
feature capture and actual packaged-notice verification remain pending. The
generated direct/complete/CSV inventories below receive a bounded Office Windows
delta from Build's locked metadata and an actual locked/offline workspace Cargo
tree (normal/build/dev), not metadata's optional-feature union. The selected tree
excludes zlib-rs. Older scope/target captures remain retained; no fresh Linux Office
or complete forty-six-capture refresh is claimed.

## Private resident-worker dependency handoff — 17 November 2026

This records the private migration proposal, not an integrated dependency graph
or implementation pass. The current canonical `midnight-work-budget` manifest has
no Tokio dependency; its private manifest adds optional `tokio = { workspace =
true, optional = true }`, activated by `resident-workers = ["dep:tokio"]`.

| Private edge / feature | Selected version and scope |
| --- | --- |
| `midnight-work-budget` → Tokio | Existing lockfile version **1.54.1**, upstream **MIT**; optional until `resident-workers` is enabled |
| Engine → work-budget | Enable first-party `resident-workers` feature |
| Viewer Engine → work-budget | Add/enable first-party dependency with `resident-workers` |
| Compare Engine → work-budget | Enable first-party `resident-workers` feature |

The optional edge inherits the existing workspace features `rt-multi-thread`,
`sync`, `time` and `macros`; Tokio's pinned manifest declares `default = []`.
Thus no default Tokio functionality is enabled by default. The proposal does
not spell `default-features = false`; it inherits the workspace declaration,
whose default feature set is empty. Existing consumers can unify additional
features, so these four are this edge's selection, not a claim about the whole
application's resolved feature union.

The actual cached 1.54.1 `Cargo.toml` and `LICENSE` were inspected: the license is
MIT, with Tokio Contributors' copyright and permission notice retained for
distribution. The existing release collector copies that source license to
`licenses/tokio-1.54.1/`; this edge requires no changed license/notice bytes.
It introduces no third-party version, mandatory fee, application-license change
or public plugin API/SDK dependency. First-party crates remain excluded from
third-party package counts.

The private proposal originally left the generated rows unchanged. The
19 November full capture/regeneration now records `midnight-work-budget` as a
Tokio direct declarer, alongside the integrated Find/tooltip and Media edges.
Package/source/license declarations are unchanged for those existing records;
retain their existing notices. The private proposal's earlier review alone did
not establish a full graph refresh.

Upstream maintenance and the RustSec Tokio package record were reviewed on
17 November 2026. Version 1.54.1 is within the patched ranges of
[RUSTSEC-2026-0384](https://rustsec.org/advisories/RUSTSEC-2026-0384.html) and
[RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html);
[its upstream release](https://github.com/tokio-rs/tokio/releases/tag/tokio-1.54.1)
records the timer repair. This is a pinned-package review, not a fresh complete
workspace advisory scan or implementation/native qualification.

## Main functional libraries

Optional executable-size diagnostics use `cargo-bloat` **0.12.1** (MIT), installed
only into `.tools/cargo-bloat`; its build output stays under `_build/tools/`.
It is not an application dependency or a distributed runtime component. The
[diagnostic tool inventory](dependencies/CARGO_BLOAT.md) records its pinned Windows
dependency graph separately from the application/plugin inventories above.

This table is a guide; the linked inventories above contain every selected crate.

| Library | Purpose | Declared license |
| --- | --- | --- |
| Slint 1.18.1 | UI, compiler and render integration | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0; royalty-free desktop branch selected |
| Tokio 1.54.1 | Async backend orchestration | MIT |
| serde 1.0.229 / serde_json 1.0.154 / toml 0.9.12 | Data/settings/manifest serialization | MIT OR Apache-2.0 |
| sha2 0.10.9 | SHA-256 copy/readback verification, protected Windows plugin runtimes and standalone SHA-256/SHA-512 checksum backend | MIT OR Apache-2.0 |
| md-5 0.10.6 / sha1 0.10.6 | MD5/SHA-1 compatibility calculations in the standalone checksum backend; not authentication or copy verification | MIT OR Apache-2.0; MIT branch selected |
| crc32fast 1.5.4 | SFV/CRC32 compatibility calculations and verification of private spool reads; also an existing PNG/compression dependency | MIT OR Apache-2.0; MIT branch selected |
| notify 8.2.1 | Linux filesystem event watching (Windows uses its own `ReadDirectoryChangesW` service) | CC0-1.0 |
| image 0.25.10 / png 0.18.1 | Icons, isolated image thumbnails and the independent image viewer | MIT OR Apache-2.0 |
| zip 8.6.0 | Read-only ZIP archive provider and host Office package parsing | MIT |
| quick-xml 0.41.0 | Host Office XML parsing; existing transitive consumers | MIT |
| hayro 0.9.0 and its parser/interpreter crates | Isolated PDF rendering and first-page PDF thumbnails | Apache-2.0 OR MIT; pinned notices and embedded font/CMap licenses accompany the PDF viewer and image thumbnail packages |
| similar 2.7.0 | Bounded text comparison | Apache-2.0 |
| sevenz-rust2 0.23.0 / lzma-rust2 0.21.0 | Read-only 7z and XZ decoding | Apache-2.0 |
| bzip2 0.6.1 / libbz2-rs-sys 0.2.6 | Pure Rust BZIP2 stream decoding | MIT OR Apache-2.0 / bzip2-1.0.6 |
| ruzstd 0.9.1 / twox-hash 2.1.5 | Zstandard decoding and frame checksums | MIT |
| tar 0.4.46 / flate2 1.1.10 and 1.1.11 | TAR/GZIP archive decoding, host Office DEFLATE, SDK packaging and image codec dependencies | MIT OR Apache-2.0 |
| blake3 1.8.7 | Thumbnail cache identities and integrity records | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception |
| ffmpeg-next 9.0.0 / ffmpeg-sys-next 9.0.1 | Required isolated video thumbnail and playback decoder bindings with software scaling and resampling | WTFPL |
| bindgen 0.73.2 | Build-time native video bindings | BSD-3-Clause |
| regex 1.13.1 | Matching and the source viewer's custom lexer | MIT OR Apache-2.0 |
| pulldown-cmark 0.13.6 | Markdown parsing | MIT |
| parley 0.11.1 / swash 0.2.10 | Unicode text layout and glyph rendering | Apache-2.0 OR MIT |
| ICU casemap / locale core 2.4.0 | Unicode case mapping and locale data/types | Unicode-3.0 |
| minicbor 2.3.0 | Bounded plugin wire messages | BlueOak-1.0.0 |
| windows 0.62.2 / windows-sys 0.61.2 | Windows API bindings | MIT OR Apache-2.0 |
| rustix 1.1.6 | Unix native calls | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| arboard 3.7.0 | Native clipboard integration | MIT OR Apache-2.0 |
| cbindgen 0.29.4 | API C-header generation tool | MPL-2.0 |

The checksum backend pins the digest-0.10-compatible hash releases and enables
only `std`. Optional assembly, OID, compression and force-soft features are not
enabled. Exact released source archives and both license texts were inspected;
MIT copyright/license notices accompany distribution. SHA-1 0.10.7 was also
inspected: its only patch change disables an optional assembly feature that this
project does not enable. The Ctrl+F8 window is implemented and has separate
[UI qualification](verification/2026-11-07-checksum-window.md); native hashing and
manifest evidence is in [backend verification](verification/2026-11-07-checksum-backend.md).

SFV checksum support reuses the already locked `crc32fast` 1.5.4 as a direct
engine dependency with only `std` enabled. Its released manifest and full MIT
license were inspected; MIT is the selected branch. The only runtime dependency
is the already selected `cfg-if` 1.0.5. The release notice collector retains the
existing `crc32fast` copyright and license files under `licenses/crc32fast-1.5.4/`;
no new package, paid alternative or nightly feature is introduced.
The Windows and native Linux application graphs were recaptured and the complete
inventory regenerated and checked: 490 package records and 458 names remain,
with 61 direct package/version records after promoting `crc32fast` to a direct
dependency. `midnight-worker-platform` uses the same locked version with the same
`std` feature to verify private spool reads (2026-11-20); this adds a dependency edge,
not a package, version, feature or license.

The native Browse/Read API and author SDK use only `std`. Independent Windows
and Linux filesystem packages reuse windows-sys 0.61.2/windows-link 0.2.1 and
libc 0.2.191 under their inspected MIT alternatives. Windows runtime features are
Foundation/Storage_FileSystem; standalone dynamic-loader tests additionally use
System_LibraryLoader. Linux enables only libc `std`. Each package includes the
complete selected MIT notice; no dependency version or mandatory fee was added.

The independent Network Neighborhood plugin adds the `network` inventory scope.
Windows reuses windows-sys 0.61.2 (defaults disabled; Foundation and WNet with their
parent features) and windows-link 0.2.1 under MIT. Linux lazily loads GIO through
libloading 0.8.9 (ISC) and cfg-if 1.0.5 (MIT), without optional features. Their
exact pinned manifests and license texts were inspected; all four already exist
in the inventory. Native-host Windows/Linux metadata and Cargo tree captures
record the separate target graphs. Promoting libloading to a direct dependency
raises the direct count to 62; the complete inventory remains 490 records and 458
names. Full selected license texts and runtime notes
ship inside the package's [THIRD-PARTY.md](../plugins/midnight-filesystem-network/THIRD-PARTY.md).
Optional GLib/GIO and GVfs/backends/FUSE are system-provided and are not bundled.
GVfs Windows discovery also uses the optional system `wsdd` helper. Ubuntu
`wsdd` 2:0.7.1-5 (upstream 0.7.1) and its Debian packaging were inspected under
MIT, with system Python 3 as the only declared dependency and no optional build
features. It is not bundled or added to the Cargo inventory; its full runtime
provenance is recorded in the network package's THIRD-PARTY.md.
The inspected GLib package is LGPL-2.1-or-later with per-file notices; GVfs 1.54.4
COPYING is GNU Library GPL v2, with inspected backend/client headers allowing later
versions. Any future native runtime bundle needs its own full license, backend
dependency and corresponding-source audit. Current fixture checks do not qualify
live SMB/FUSE interoperability.

The independent native macOS package also uses libc 0.2.191 with only `std` and
the public, std-only filesystem API/SDK. Its exact cached manifest/features and
full MIT license were inspected; retain the package's
[THIRD-PARTY.md](../plugins/midnight-filesystem-native-macos/THIRD-PARTY.md) with
every distribution. Its separate Darwin x86-64 and arm64 captures add the
`native-macos` scope and direct dependant to the existing libc record, without
adding a package/version or changing license choices. The existing Windows/Linux
captures are preserved. These target-filtered metadata/tree captures were made
on Windows; this graph has no additional build or procedural-macro dependencies.
They do not establish Darwin linking, runtime behavior or main-application support;
see the package's [qualification limits](../plugins/midnight-filesystem-native-macos/VERIFICATION.md).

The archive provider supports bounded ZIP/7z/TAR and GZIP/XZ/BZIP2/Zstandard streams,
including compressed TAR variants, with read-only browsing and host-verified copy-out;
its concrete limits are in [the filesystem API](developer/FILESYSTEM_API.md).
The source viewer currently uses custom lexical
profiles with `regex`, not a comprehensive grammar engine such as syntect or
Tree-sitter. Media preview choices and implementation status are in the
[thumbnail design](superpowers/specs/2026-11-05-panel-views-thumbnails-design.md).

## Native libraries, runtime and data outside Cargo

Linux settings Browse uses the desktop FileChooser portal through zbus 5.19.0
(MIT), futures-lite 2.6.1 (MIT OR Apache-2.0) and Tokio 1.54.1 (MIT). The Tokio-backed
portal enables socket2 0.6.5 (MIT OR Apache-2.0) and bytes 1.12.1 (MIT). These
licenses were checked in the pinned Cargo manifests; no paid service is involved.
Windows Browse uses the existing Windows Common Item Dialog bindings. Runtime
distribution notices are generated from the target's selected dependency graph.

These are distinct from the Cargo inventory. Most are supplied by the user's OS;
versions, transitive dependencies and graphics drivers vary by installation.

| Component | Use | Upstream license family / provenance |
| --- | --- | --- |
| FFmpeg 9.0.2 shared libraries | Shared by the isolated video thumbnail and playback workers | LGPL-2.1-or-later in the supplied configuration: GPL, nonfree, version3 and network disabled. Matching source, configuration and notices accompany the five runtime libraries |
| LLVM/libclang 18.1.1 wheel or system development package (Ubuntu 18.1.3 in this Debug build) | Build-time video binding generation, not shipped as application runtime | Apache-2.0 WITH LLVM-exception; retained upstream license or distribution copyright files |
| GnuPG system package (2.4.4 in this build container) | Build-time FFmpeg detached-signature verification in an isolated keyring; not bundled with ME | GPL-3.0-or-later; retain distribution package notices when redistributing the build environment |
| Fontconfig | Linux font discovery; direct ELF import | Permissive MIT/X11-style notices; distribution copyright file |
| FreeType | Linux font stack | FreeType License OR GPL-2.0; retain FTL credit when choosing FTL ([terms](https://freetype.org/license.html)) |
| GNU libc (`libc`, `libm`, loader) | Linux runtime | LGPL-2.1-or-later with component exceptions; distribution copyright files ([upstream](https://sourceware.org/glibc/)) |
| GCC `libgcc_s` | Linux unwind runtime | GPL-3.0-or-later WITH GCC-exception-3.1 ([runtime terms](https://gcc.gnu.org/onlinedocs/libstdc++/manual/license.html)) |
| X11/XCB libraries | Linux desktop/window integration | MIT/X11/BSD-family, per component ([X.Org notices](https://xorg.freedesktop.org/archive/current/doc/xorg-docs/License.html)) |
| Wayland client libraries | Native Wayland integration | MIT-family; retain per-source notices ([upstream](https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/COPYING)) |
| libxkbcommon | Linux keyboard handling | Combined permissive MIT/ISC/X11-style terms ([license](https://raw.githubusercontent.com/xkbcommon/libxkbcommon/master/LICENSE)) |
| libudev | Linux device discovery | LGPL-2.1-or-later ([systemd source](https://raw.githubusercontent.com/systemd/systemd/main/README)) |
| Vulkan loader | Linux graphics backend | Apache-2.0 with more-permissive exceptions ([license](https://raw.githubusercontent.com/KhronosGroup/Vulkan-Loader/main/LICENSE.txt)) |
| Windows system DLLs, UCRT, VC runtime | Windows shell, storage, UI, DirectWrite and graphics | Microsoft platform/runtime terms; not open source merely because the Rust bindings are MIT/Apache |
| Darwin/libSystem | Independent native macOS filesystem package prerequisite; not bundled | Apple platform terms; the libc Rust binding's MIT license does not relicense macOS |
| Rust standard library/runtime | Application platforms and independent macOS filesystem package | MIT/Apache-2.0 plus component-specific notices; releases include `licenses/rust/COPYRIGHT-library.html` |
| Unicode case-fold data in viewer packages | Unicode-insensitive search | Unicode-3.0; each package retains `UNICODE-LICENSE.txt` |

Read-only import inspection of the tested release found Linux `me` directly imports
`libfontconfig.so.1`, `libgcc_s.so.1`, `libm.so.6` and `libc.so.6`; the three viewer
libraries import `libgcc_s`, `libc` and the ELF loader. Window/graphics libraries can
also be loaded dynamically, so `readelf` alone is not a complete runtime inventory.
Windows imports include Kernel32/User32, Shell32, OLE, GDI, DirectWrite, DXGI,
OpenGL32, UCRT API sets and `VCRUNTIME140.dll`. Host fonts/icons are not bundled
third-party font/icon packs. OS drivers and installed external editors are not
pinned Cargo dependencies of this project.

## Distribution notices

Each release includes `THIRD-PARTY.json`/`THIRD-PARTY.txt` and copied dependency
license/notice files from the exact packaged graph. Independent viewers include
their own `LICENSE`, `THIRD-PARTY.md`, Unicode notice and generated dependency notices.
The package generator refuses missing required plugin notices. This development
inventory supplements those per-artifact notices, rather than replacing them.
Midnight Explorer's own application/API distribution license has not been selected;
these dependency reports do not grant a new license to project-owned code.

The About dialog's Licenses tab is built from the [machine-readable inventory](dependencies/cargo-licenses.csv)
when the application is compiled: the records of the running platform's target whose
scopes a release ships (application, viewer, filesystem, archive, network and
thumbnail packages; not renderer tests, the ABI conformance project, the public
API/SDK workspaces or the macOS filesystem package), one row per package version
naming the components that use it. Like the release notice, it conservatively
includes build- and test-time libraries. It adds the bundled components outside
Cargo recorded here: the Rust standard library, FFmpeg, dav1d, CEF, the Noto Sans
fonts, the Unicode case-folding data, the PDF viewer's embedded Foxit fonts and
Adobe CMaps, and the four palette references. Regenerating the inventory therefore
updates the tab; a `midnight-ui` unit test fails when an inventory record is no
longer pinned by the lockfile of a component that ships it, and another checks the
bundled versions against these records. Opening a library (View license for the
selected row, the open mark at the end of a row, a double-click or Enter) asks a
backend worker for its notices. Release builds read only the `licenses` folder and
`THIRD-PARTY.json` beside the program (debug builds may also read a developer's
Cargo registry); links are never followed, every read is capped before it starts
and one library shows at most 128 KiB of notice text. Authors come from the package
manifests that `THIRD-PARTY.json` records, otherwise from concrete copyright lines
of its notices: lines that start with "Copyright" or © and carry a year, © or (c),
or start with "(c)" and a year, without license templates such as
`Copyright [yyyy] [name of copyright owner]`, license prose such as Apache's
"copyright license to reproduce", or repeats that differ only in spacing or case.
The dialog lists one author per line. Notices that ship inside a plugin package are
named there instead.

Video decoding currently enables VP8/VP9, Theora, FFV1 and raw video with
Matroska/WebM, AVI and Ogg containers. This is not a promise of MP4/H.264 support.
The video package retains FFmpeg's corresponding source archive and the exact
configuration needed to rebuild its shared libraries. `ffmpeg-sys-next` 9.0.1
declares WTFPL but its published source has no LICENSE file; distribution retains
the canonical WTFPL text, the pinned declaration and provenance rather than
claiming an upstream bundled license file exists.

The native playback output leaf uses Windows bindings 0.62.2 under MIT OR Apache-2.0.
The enabled Windows binding features are Media Audio, COM Structured Storage and
Variant, with their inherited features. Linux uses libloading 0.8.9 under ISC to
open a supported absolute system libasound.so.2. System ALSA retains the distribution
license and is not shipped as part of the application. FFmpeg targets stay isolated
from native output, and the worker SDK stays at its existing 0.2.0 pin.

## Regenerating the inventory

Use Python 3.11+ and the pinned Rust toolchain. Fetch dependencies and obtain the
released API/SDK bundle following the [plugin quick start](developer/QUICK_START.md).
The independent plugins use that bundle's `.cargo/config.toml`; they do not use
private host crates. Use an existing released bundle matching plugin lockfiles.
All capture and generated build output belongs under `_build/`.

Capture JSON and tree text for each scope below, using Windows for
`x86_64-pc-windows-msvc` and Linux for `x86_64-unknown-linux-gnu`, except
`native-macos`, which requires only `x86_64-apple-darwin` and
`aarch64-apple-darwin`. Do not add Darwin captures for the application or other
workspaces. Name the outputs
`<scope>-<target>.json` and `<scope>-<target>.tree` in `_build/verification/dependency-inventory/`.

| Scope | Manifest |
| --- | --- |
| app | `Cargo.toml` |
| renderer-tests | `vendor/femtovg/Cargo.toml` |
| api | `api/Cargo.toml` |
| basic | `plugins/midnight-viewer-basic/Cargo.toml` |
| source | `plugins/midnight-viewer-source/Cargo.toml` |
| markdown | `plugins/midnight-viewer-markdown/Cargo.toml` |
| conformance | `plugins/viewer-conformance/Cargo.toml` |
| filesystem-api | `filesystem-api/Cargo.toml` |
| native-windows | `plugins/midnight-filesystem-native-windows/Cargo.toml` |
| native-linux | `plugins/midnight-filesystem-native-linux/Cargo.toml` |
| native-macos | `plugins/midnight-filesystem-native-macos/Cargo.toml` |
| network | `plugins/midnight-filesystem-network/Cargo.toml` |
| worker-api | `worker-api/Cargo.toml` |
| thumbnail-api | `thumbnail-api/Cargo.toml` |
| image-viewer-api | `image-viewer-api/Cargo.toml` |
| web-viewer-api | `web-viewer-api/Cargo.toml` |
| html-viewer | `plugins/midnight-viewer-html/Cargo.toml` |
| image-viewer | `plugins/midnight-viewer-image/Cargo.toml` |
| image-decoder | `plugins/image-decoder/Cargo.toml` |
| pdf-viewer | `plugins/midnight-viewer-pdf/Cargo.toml` |
| archive | `plugins/midnight-fs-archive/Cargo.toml` |
| thumbnail-image | `plugins/midnight-thumbnail-image/Cargo.toml` |
| thumbnail-video | `plugins/midnight-thumbnail-video/Cargo.toml` |

Example PowerShell capture for one scope/target; repeat using the table. Omit
`--config $sdkConfig` for application, API, native and network filesystem workspaces.
These filesystem packages use only the checked-in public API/SDK. Viewer plugins use
the released viewer bundle, except `source` and `markdown`, which use the
configuration that their `scripts/prepare-sdk.ps1`/`prepare-sdk.py` derives from it
(as their CI does); archive, image, PDF and thumbnail plugins use the separate
public worker bundle described in [the worker SDK](developer/WORKER_SDK.md), and the
HTML viewer uses the web SDK configuration that `web-viewer-api/scripts/package.py`
prints. Supply the
matching bundle's absolute config path; plugin lockfiles pin its package checksums.

```powershell
$scope = 'app'
$manifest = 'Cargo.toml'
$target = 'x86_64-pc-windows-msvc'
$captures = '_build/verification/dependency-inventory'
New-Item -ItemType Directory -Force $captures | Out-Null
./scripts/cargo.ps1 metadata --locked --format-version 1 --manifest-path $manifest `
    --filter-platform $target | Set-Content -Encoding utf8 "$captures/$scope-$target.json"
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
./scripts/cargo.ps1 tree --locked --workspace --manifest-path $manifest `
    --target $target --edges 'normal,build,dev' --prefix none `
    | Set-Content -Encoding utf8 "$captures/$scope-$target.tree"
if ($LASTEXITCODE -ne 0) { throw 'Cargo tree failed' }
python -B scripts/dependency-inventory.py --metadata-dir $captures
```

For `renderer-tests`, append `--no-default-features --features wgpu` to both
Cargo commands. Its independent lockfile and development edges are part of the
capture.

On Linux use `sh scripts/cargo.sh` and UTF-8 shell redirection instead of
PowerShell's `Set-Content`; use `python3` where Python is installed under that name.
`cargo metadata`/`tree` do not compile native code. Capture each target on its
native host: build dependencies/procedural macros follow the host, so a Windows
cross-target query is not equivalent to the native Linux graph.
The native macOS package's two added captures are an explicit cross-target
exception: its current libc/public-API/SDK graph has no additional build or
procedural-macro dependencies. Recapture on a Mac if that graph gains such
dependencies; do not extrapolate its graph to the application. Metadata/tree
success never establishes native runtime support.
The generator requires forty-six metadata/tree pairs across its twenty-three
scopes, including `image-decoder`; it excludes project-owned
API/SDK crates, and writes deterministic Markdown/CSV. Explicitly marked vendored
third-party workspace members remain included. Recapture after every dependency
or feature change. `--check` verifies tables against the supplied captures; it
does not prove that those captures are current. Current source hashes are not
used as a substitute for capture provenance. The 18 November inputs were not
retained. The complete **19 November** input set is now retained in
`_build/qa/accessibility-feature-20261119/inventory-captures/`, together with the
[92-artifact/native-host verification](../_build/qa/accessibility-feature-20261119/license-inventory/capture-verification.json)
and [successful generation/check commands](../_build/qa/accessibility-feature-20261119/license-inventory/regeneration-results.json).
All original manifest and lock hashes pass before/after capture. SDK configuration
files are QA-only registry-path projections of the existing exact bundles;
shared configs, dependencies and lockfiles were not edited by capture.

The nested D checkout makes the excluded renderer test package discover an outer
workspace. Only that scope uses an exact QA copy of the vendored package and
lockfile, appending `[workspace]` with explicit `resolver = "2"`, matching the
edition-2021 standalone policy. All original source hashes and the copied lock
remain unchanged; parsed dependency/feature tables match, and both captured
renderer roots select exactly `wgpu`. The projected manifest is explicitly **not
byte-identical** to the original. The native macOS exception still selects only
`libc` as a third party, with no added build/procedural-macro dependency. These
metadata-only captures compile no code and establish no native runtime result.

The generator emits package/license/scope records, not enabled-feature columns.
Retain the affected package's `resolve.nodes[].features` with each metadata
capture. The older [feature/license CSV](third-party-inventory.csv) belongs to the
[dated main-branch audit](verification/main-audit.md), as the [docs index](README.md)
explains; preserve that historical snapshot rather than inserting later feature changes.

```sh
python3 -B -m unittest discover -s scripts/tests -v
python3 -B scripts/dependency-inventory.py --metadata-dir _build/verification/dependency-inventory --check
```

The 11 November MEDIA-01 follow-up advances only the first-party
`midnight-filesystem-api` package from 0.3.0 to 0.4.0 for negotiated cooperative
archive staging cancellation. The independent archive plugin consumes its new
immutable public SDK package; its lockfile changes only that package/version and
checksum. No third-party dependency, feature or license selection changed. This
note supplements the historical dependency captures above; it is not a new full
inventory capture.

## Host notice-source staging — 18 November 2026

The current completion-v3 stage resolves the eleven omitted license-source
packages for the selected 359-package normal/build graph. It contains 662 staged
files and 61 reviewed override files, with zero missing license inputs. Root
independently checked the exact pinned crate checksums/VCS commits and 27 saved
official manifest/license responses and copy hashes. Dependencies, the project's
license and license selections are unchanged; the historical blocked stage and
its missing-input findings remain preserved.

This is qualified notice-source staging. The later normal validation-input2
producer now matches package/source/version/checksum identities against the 359
selected tuples and locked graph; the stage's662 files were verified. Explicit
copy of the 662 notices into the new C9297 validation bundle is now complete and
included in its exact2,487-file freeze; normal-only registration preserved the
inventory. Root reviewed that complete input separately. This remains validation-
input assembly/graph qualification, without runtime, final distribution or native
acceptance. The later test-helper source epoch9c25 does not rebind that C9297 bundle.
See the [normal validation-input2 graph qualification](../_build/verification/viewer-refresh-20261117/build/normal-host-validation-input-2/qualification.json).
See the [completed C9297 validation-input review](../_build/verification/viewer-refresh-20261117/root-new-c-validation-bundle-9297-review.json).
See the [Root source/provenance review](../_build/verification/viewer-refresh-20261117/root-host-notices-staging-review-v1.json)
and [completion-v3 qualification](../_build/verification/viewer-refresh-20261117/build/host-notices-stage/completion-v3/stage-qualification.json).
