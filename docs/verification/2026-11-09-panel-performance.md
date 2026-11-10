# Panel scrolling and retained previews — 2026-11-09

This records the scoped PERF-02 / MEDIA-03 fixes and their qualification. It does
not claim 60 Hz rendering or a complete native traversal decoding all cloud
images. Evidence and diagnostic sources are under
`_build/qa/panel-performance/`; no user preferences or desktop shortcut changed.

## Native System32 measurement

The final baseline diagnostic compiles the actual application source and its
coherent cached dependency graph, including Engine, installed filesystem
provider, native icons, and thumbnail bridge. Only the smoke-test lifecycle is
extended: preferences remain in memory, a timer supplies Ctrl+F11 and real Slint
pointer events to the actual Fluent scrollbar, and cadence is logged. It is not
a compositor scanout or physical mouse-to-photon measurement.

The private diagnostic runtime is
`C:\Users\tjelinek\MidnightExplorer-20261109-panel-probe`. The original coherent
candidate at `MidnightExplorer-20261109-provider-candidate` remains intact.
Run under its ordinary owner identity; the sandbox account cannot authorize a
different owner's private plugin installation. An initial run that contained
only a parent row after listing failure is excluded.

Matched workload: `C:\Windows\System32` in both panels, active left pane zoomed
with Ctrl+F11, 5,104 displayed rows including parent, native GPU renderer,
225% scaling, 2,700×1,800 physical window, 200 pointer moves across the scrollbar
and back. First visible row spans 0–5,077. The requested timer period is 16 ms;
its delayed callbacks measure how rendering/event-loop work limits progress.
For the final matched pair, root, archive and reviewer agents explicitly paused
builds and app probes. A 250 ms monitor recorded a maximum of zero Cargo, rustc
and linker processes throughout both runs. Both diagnostic processes exited
before the reserved window was released.

| Configuration | Median interval | p95 interval | Median pointer dispatch | p95 dispatch |
| --- | ---: | ---: | ---: | ---: |
| Corrected coherent v2 baseline | 61.139 ms | 86.787 ms | 1.102 ms | 1.776 ms |
| Same source/dependencies, native renderer policy only | 43.949 ms | 56.754 ms | 1.069 ms | 1.599 ms |

Files: `system32-app-quiet-baseline.*` and `system32-app-quiet-flags.*`, including
JSON summaries with `compilerMax: 0`; script `run-app-pair.ps1`. Pointer dispatch
is consistently brief, while the remaining interval is still above 16 ms.

Excluded trials are retained for audit. The first full-app baseline measured
86.858/125.135 ms while 3/2 compiler processes were present at its boundaries.
It also used a diagnostic bridge reconstructed from Git HEAD, accidentally
omitting the existing working-tree hidden-startup fix. That copied bridge was
corrected before v2; production source already retained the fix. A first v2
repeat overlapped the tail of a diagnostic link (`compilerMax: 1`) and was also
rejected. Neither is used in the final matched table.

Earlier exploratory `probe.rs` used the production Presenter/widgets but a
background `std::fs` listing instead of Engine/provider composition. Its quiet
matched intervals were 69.213/100.442 ms median/p95 versus 40.395/61.404 ms with
the two native debugging flags disabled. Those are supporting measurements,
not the full-application baseline above. Compiler-loaded trials and an early
trial that never moved the visible row are not comparison evidence.

The renderer change removes only native DEBUG/VALIDATION defaults, reapplies
explicit WGPU environment controls, and always retains VALIDATION_INDIRECT_CALL.
An actual-source child-process test first failed with the old default and then
passed for both the ordinary default and explicit debugging opt-in. Explicit
backend selection remains available. This does not disable application content
or source-identity validation.

An owned-process instruction sampler resolved addresses using the actual linker
MAP. Of 222 sampled application main-thread addresses after warmup, 111 were in
font/text work, 56 in GPU code, 38 in UI/layout and 17 elsewhere. These are sparse
instruction samples, not call stacks or wall-time percentages. `read-fonts`,
`font-types` and `zerotrie` were unoptimized leaves despite their optimized
callers. Those three existing dependencies now use development opt-level 2;
versions/features/dependencies are unchanged. These are development-build
measurements; release dependencies were already optimized.

After the integrated build, a second reserved quiet window compared the frozen
renderer-only control with the new candidate. Both again used 200 samples,
5,104 rows and the same complete scrollbar traversal/window geometry, with
`compilerMax: 0` throughout:

| Final comparison | Median interval | p95 interval | Maximum interval | Median/p95 dispatch |
| --- | ---: | ---: | ---: | ---: |
| Frozen renderer-only control | 50.723 ms | 68.061 ms | 76.287 ms | 1.102 / 1.998 ms |
| Integrated candidate | 40.394 ms | 58.504 ms | 68.047 ms | 1.080 / 1.928 ms |

Files: `system32-final-control.*`, `system32-final-integrated.*`, script
`run-integrated-pair.ps1`. This qualifies the integrated candidate; it does not
isolate font optimization from the intervening archive/thumbnail UI changes.
Control variation between quiet windows also prevents assigning every measured
millisecond to one change. The isolated renderer comparison remains the earlier
v2 pair. Remaining redraw intervals are above 16 ms.

## Thumbnail retention and repaint

The requested Screenshots folder contains 2,219 PNG files. The old presentation
record limit was 1,024 regardless of pixel size; the separate decoded-pixel
budget is 54 MiB. A real Presenter test using 2,219 96×48 previews reproduced
eviction despite fitting the pixel budget. A second test reproduced disappearing
pixels when a same-folder refresh increments navigation intent while the old
successful listing remains displayed.

Changes now in source:

- Pixel identities follow the exact displayed flat listing generation and
  EntryId. Input identities retain current navigation/search intent; tree
  identities retain their stricter publication identity. Hover uses the actual
  input identity independently. A committed replacement rejects old completions.
- The record limit is 4,096; the 54 MiB pixel budget is unchanged. Both static
  record overflow and six-frame animation overflow remain covered. Repeated
  touches reuse existing bookkeeping keys.
- The 100 ms composition timer no longer refreshes the whole application for a
  changed visible range. Frame completions and animation transitions notify only
  matching visible rows. Explicit cache clear and service notices retain normal
  full refresh behavior. No UI filesystem or decoding work was introduced.

Evidence: initial retention RED had 5 passing / 2 failing tests; staged GREEN
had 9 passing thumbnail tests. The actual Rows/ModelNotify repaint RED notified
all five visible rows instead of the one completed row; GREEN also verifies
empty, offscreen and stale updates issue no notifications or model reset.
Integrated shared-Cargo `midnight-ui --lib thumbnail` passes 13 tests. The root's
reviewed integrated UI gate subsequently passes 595 units plus five main-menu
and eleven shared-toolbar integration tests (`_build/delete-followups/archive-size-reviewed-ui.log`).

A bounded native baseline check uses the exact Screenshots folder (2,219 PNGs,
one desktop.ini, and the parent row), real installed image worker and native GPU
window. Its diagnostic capture factory permits only the explicitly selected
resident PNG, and uses an owned temporary cache. The decoded 96×54 image remains
available immediately after scrolling away and returning, before the next
100 ms visibility timer; it stays unchanged for two seconds. There is one
accepted/uploaded frame even though the backend captures the source twice on
departure/return. The GPU screenshot was inspected. Files:
`screenshots-resident-baseline.{out,csv,png}`. This demonstrates existing
single-image retention, not complete-folder cache retention or the integrated
new repaint path. All other preview content was deliberately excluded to avoid
bulk cloud hydration. The integrated build passes the same check with the new
native DLL and row-specific presentation bridge. Its GPU screenshot was also
inspected: the completed preview remains displayed. Integrated evidence:
`screenshots-resident-integrated.{out,csv,png}`. This is deliberately bounded
native coverage; the 2,219-image retention stress remains a real Presenter test
using admitted pixel replies rather than bulk reading the cloud folder.

## Salamander source review

Read the local `D:\experiments\salamander-main` sources requested by the user:

- `src/icncache.h` and `src/icncache.cpp` (especially GetIconsAndThumbsFrom around
  lines 605–644): directory-owned raw pixel buffers, transferred completed images
  rather than destroying and rebuilding them with each listing.
- `src/fileswn1.cpp` around 539–645: visible entries before surrounding entries,
  visibility-version restart and skipping already completed work; around
  1015–1055: item-specific completion notification.
- `src/fileswnb.cpp` around 689–770: RepaintIconOnly for the completed item and
  batching during repaint suppression.
- `src/plugins/pictview/thumbs.cpp` around 812–917: bounded preview extraction and
  fast embedded preview before more expensive quality work.

The applicable patterns are retained completed images, stable old-display
adoption and item-specific repaint. No source code or assets were copied. The
old cache's name/size/time matching, ANSI paths and MAX_PATH assumptions were not
adopted; captured source identity/version remains authoritative here.

## Exposed cloud placeholder robustness

The first source-only suspicion that the application refused every OneDrive
image was disproved: a selected resident image already opens through the
unmodified provider under Windows' default placeholder-disguise compatibility
mode. This is not established as the current flicker cause.

An explicit test-thread exposed-placeholder mode reproduces a narrower failure:
the public provider table rejects that same resident PNG as a reparse point.
The fix checks FileAttributeTagInfo on the retained OPEN_REPARSE_POINT handle,
admits only the Cloud tag family without the name-surrogate bit, and rejects
directories, links/junctions, devices and unrelated reparse providers. Existing
volume/file ID, length, write/change timestamp validation remains in place.

`cloud-exposed-red.log` records Unsupported before the fix;
`cloud-exposed-green.log` records a 24-byte PNG header and ValidateRead success.
Only one explicitly selected resident image is used for this acceptance; no
broad placeholder hydration is part of qualification. Owned fixture tests reject
direct junction reads and stale captured-name replacement with a junction, and
verify unrelated target bytes remain unchanged.

Native plugin suite: 16 pass, one resident acceptance ignored by default and
separately passed; all-target clippy with `-D warnings` passes. The ordinary
dynamic DLL integration also passes. Updated installed-provider resident
acceptance also passed: a separate private runtime loaded the original DLL and
returned Unsupported, then loaded the tested DLL and read exactly 24 bytes
through InstalledProvider with retained revision and revalidation. The selected
resident PNG is 494,686 bytes, 1,905×1,064 pixels. Logs:
`installed-cloud-red.log` and `installed-cloud-green.log`. Ordinary plugin trust
checks remained enabled; baseline and Markdown diagnostic runtimes were not
modified.

Tested DLL:
`_build/goal/native-plugin/native-windows/debug/deps/midnight_filesystem_native_windows.dll`
(601,600 bytes), SHA256
`21E82F4DEA4588E06D2B6F372DFE54AAE3527D454825A949F37B96C08FCA9EAC`.
The coherent baseline runtime deliberately retains its earlier native DLL.

Windows references: [placeholder compatibility mode](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/content/ntifs/nf-ntifs-rtlsetprocessplaceholdercompatibilitymode),
[reparse tags](https://learn.microsoft.com/en-us/windows/win32/fileio/reparse-point-tags),
and [name-surrogate semantics](https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-isreparsetagnamesurrogate).

## Frozen candidate and final gates

The private candidate is
`C:\Users\tjelinek\MidnightExplorer-20261109-panels-candidate`. The desktop
shortcut and user preferences were not changed. The original provider candidate
and renderer-only diagnostic remain separate. Frozen application source,
dependency graph and hashes are retained under
`_build/qa/panel-performance/app-integrated-*` and `integrated/`.

| Artifact | Bytes | SHA256 |
| --- | ---: | --- |
| `me.exe` | 85,954,560 | `31906E875EBEC4125F3B33A92425A7741B1C52A678478616BA5F1F8C3A2CCCC9` |
| Native Windows provider DLL | 601,600 | `21E82F4DEA4588E06D2B6F372DFE54AAE3527D454825A949F37B96C08FCA9EAC` |
| Archive worker | 941,568 | `069B835F2CFE37150137BBCC14DD38D9E8CD0E2F6DCDADFAAB0A8C600347C922` |

The existing compatible viewer/thumbnail packages and their notices were copied
from the coherent provider candidate. Later media package replacement remains
owned by the separate media/SDK work; this report does not claim those packages
were rebuilt here. `integrated/deployment.json` records the installed hashes.
The optional `panel-integrated.exe` diagnostic supports in-memory smoke
preferences and a 15-minute hold when `MIDNIGHT_PANEL_HOLD=1`; it is not a
production diagnostic protocol or shortcut target.

Final gates use toolchain 1.99.1, locked/offline dependencies, shared `_build`,
debug info 0 and incremental compilation disabled:

- Real app thumbnail timer/startup/failure test: one pass
  (`app-thumbnail-tests.log`).
- App native renderer default/explicit-opt-in child-process test: one pass
  (`app-renderer-tests.log`).
- `cargo clippy -p midnight-ui -p midnight-explorer --all-targets --no-deps
  --locked --offline -- -D warnings`: pass (`scoped-clippy-green.log`).
  The broader attempt also linted engine dependencies and failed on 13 existing
  unused tree-action-capture items. No suppressions or unrelated engine changes
  were applied; this is not a claim of workspace-wide warning freedom.
- `cargo build -p midnight-explorer --bin me --locked --offline`: pass
  (`integrated-app-build.log`), then the exact artifact/dependency graph was frozen.
- Native plugin suite and strict all-target clippy: pass as recorded above.
- Independent scoped review of cache/repaint/renderer/cloud changes found no
  actionable issues. Final lint changes only simplify the same animation
  condition and borrow test slices.

The bounded native Screenshots check and final System32 run exercise the frozen
integrated build. Full-folder cloud decoding, arbitrary media/provider coverage,
60 Hz redraw and an isolated numerical font-only claim remain outside this
qualification. TREE-01 also remains open for the already documented captured
action consumers; these presentation/performance changes grant no new mutation
authority.
