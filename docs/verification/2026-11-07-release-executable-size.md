# Windows release executable size inspection

This is a read-only inspection of the completed 2026-11-07 release rebuild and
its MSVC linker map. The rebuild includes the archive/settings/review fixes.
Measurement did not change the renderer, dependencies or release profile. The
linker map was requested only for the final application crate.

## Artifact and PE measurements

- File: `_build/release/me.exe`, x86-64 PE.
- Length: **41,606,144 bytes (39.6787 MiB)**.
- Last-write time: **2026-11-07T07:02:56.0986221Z**.
- SHA-256: `36e6157bd39e9fba5d429c398d829bc0892d51208994106a8163b973d9a2c0f9`.
- Map: `_build/qa/executable-size/me.map`, 44,703,303 bytes. Its timestamp
  `6aeecd1f` and load address match the PE; all parsed section/address pairs match.

The previous artifact was 41,167,872 bytes, timestamp 2026-11-07 02:53:19 UTC,
SHA-256 `512fb1b9e28db83c6fc0a7643dec85999101cb772f78daafd90def9a7d72be20`.
The new artifact is **438,272 bytes larger (1.06%)**. This includes application
changes; it is not a controlled size-optimization comparison.

| Section | File bytes | Share | Purpose |
| --- | ---: | ---: | --- |
| `.text` | 29,777,920 | 71.57% | Compiled instructions |
| `.rdata` | 10,413,568 | 25.03% | Read-only tables, constants, pointers, strings and embedded data |
| `.data` | 5,632 | 0.01% | Initialized writable data |
| `.pdata` | 1,198,592 | 2.88% | Windows x64 function/unwind lookup data |
| `.rsrc` | 105,472 | 0.25% | Windows icon and manifest resources |
| `.reloc` | 103,936 | 0.25% | Relocations |
| Headers | 1,024 | <0.01% | PE headers/alignment |

The last section ends at the file length: there is no appended payload/overlay.
The COFF symbol count is zero. The PE debug directory has three records whose
payloads total only **907 bytes**; this is not an executable containing megabytes
of debug information. The adjacent `me.pdb` is **4,681,728 bytes** and is a separate
file, not part of the executable size.

The map identifies **4,527,684 bytes of `.xdata` unwind/exception metadata inside
`.rdata`**, or 43.48% of that section. Together with `.pdata`, this accounts for
**5,726,276 bytes (5.46 MiB)**. Parsing the unwind records independently confirms
2,075,204 bytes of fixed headers/code slots/trailers; language-specific handler
payloads account for additional data in the mapped `.xdata` contribution. These
are runtime structures, not debug symbols that another stripping pass removes.

## Measured function-body attribution

The map contains 203,283 symbol records. Exact map addresses match the starts of
**99,863 of 99,865** PE runtime-function entries, covering **28,871,551 bytes**,
or **96.96% of `.text`**. The two unmatched entries total 669 bytes. The remaining
906,369 bytes of `.text` include functions without unwind entries, alignment and
other bytes not attributed by this method. No size was inferred from a gap
between public symbols.

Rust names were demangled with the installed toolchain's existing demangler.
Explicitly named MSVC cleanup funclets were associated with the Rust parent
encoded in their names. Each physical function range is counted once. A bucket
contains the union of component references in all aliases at that address;
cross-component references are retained in a separate mixed bucket.

| Disjoint symbol-associated function-body bucket | Bytes |
| --- | ---: |
| Generated main-window UI | 8,135,915 |
| GPU backends and shader stack | 4,370,664 |
| Application logic and hosts | 3,055,231 |
| Fonts and Unicode | 1,887,140 |
| SVG and image stack | 1,786,357 |
| Runtime, serialization and diagnostics | 1,643,812 |
| Slint and FemtoVG | 1,567,441 |
| Viewer UI | 615,652 |
| Windowing | 280,093 |
| Mixed component references | 4,564,588 |
| Other/runtime/unclassified | 964,658 |
| **Total measured function bodies** | **28,871,551** |

Within the mixed bucket, **1,522,389 bytes** reference generated main UI and Slint,
**1,390,149 bytes** reference application/host and runtime specializations, and
**229,497 bytes** reference viewer UI and Slint. Those bytes are already counted
above and must not be added again to the total.

These are bodies associated with symbol names, **not isolated crate sizes or
predicted savings from removing a feature**. Fat LTO inlines dependency code
inside these bodies; generic specializations and folded aliases cross crate
boundaries. The table does not attribute `.rdata` tables or unwind metadata to
individual components.

Some large exact function bodies make the first investigation targets concrete:

| Function | Bytes |
| --- | ---: |
| Generated `InnerMainWindow` drop glue | 142,049 |
| Generated `InnerMainWindow::ensure_instantiated` | 125,701 |
| Generated `InnerOptionsBody_root_1135::ensure_instantiated` | 117,792 |
| Generated `InnerMainWindow::default` | 115,676 |
| `midnight_engine::Engine::run` closure | 103,928 |
| `midnight_ui::refresh_properties` | 69,809 |
| `midnight_ui::install` | 68,957 |
| `wgpu_core::command::render::encode_render_pass` | 59,418 |
| `wgpu_hal::vulkan::Instance::expose_adapter` | 56,090 |
| `naga::valid::Validator::validate_impl` | 49,803 |

## Assets and generated UI

The release main-window generated source has 41 distinct `include_bytes!`
inputs: the 64,308-byte window PNG plus 40 SVG icons. Those source assets total
**73,103 bytes**, and exact byte matching finds all 41 in `.rdata`. The same PNG
also occurs in `.rsrc`, inside the Windows icon resource. Counting all `.rsrc`
plus these `.rdata` assets gives **178,575 bytes (0.43% of the executable)**.
Asset compression or removing the duplicate PNG cannot explain a large saving.
The 1,199,358-byte `moon-master.png` source artwork is not embedded.

Neither generated UI source embeds font files or registers an embedded font.
System-font discovery, shaping and glyph rasterization code is compiled in;
that code must not be confused with a bundled font-file payload.

| Current release generated file | Source bytes | Last-write time (UTC) |
| --- | ---: | --- |
| `build/midnight-ui-bindings-06a87a069e1db62a/out/main.rs` | 23,100,350 | 2026-11-07 06:57:09 |
| `build/midnight-viewer-ui-8b0f1b4de7f761fb/out/viewer.rs` | 1,331,826 | 2026-11-07 06:57:13 |

Paths in this table are relative to `_build/release/`. These are Rust source
lengths, **not executable contributions**. The compiler optimizes, inlines and
deduplicates across the final fat-LTO object. Existing investigations of control
and popup reuse are in [the Slint generation report](slint-code-generation-size.md);
its historical checkpoints must not be presented as measurements of this binary.

## Active feature graph and limits of attribution

The inspected workspace release profile already uses `opt-level = 3`, fat LTO,
one codegen unit, no incremental compilation, no debug information and symbol
stripping. Overflow checks and unwinding remain enabled intentionally; removing
them would change integrity checks and worker-panic reporting.

The Windows application's normal dependency graph includes:

- Slint 1.18.1 with the Winit backend, FemtoVG/WGPU renderer and software renderer.
  Software rendering is an exposed `--software` option and a qualified fallback.
- WGPU 30 with Direct3D 12, Vulkan and GLES features, and Naga with HLSL, SPIR-V
  and GLSL output support. Windows selects Direct3D by default; explicit backend
  overrides remain supported. The cached Slint FemtoVG package enables WGPU's
  default features. Adding a direct dependency with `default-features = false`
  would not subtract features already enabled upstream.
- SVG rendering/decoding and text shaping/font discovery: resvg/usvg,
  Parley/Fontique/Swash and Skrifa. The viewer and Slint use unified versions of
  the shared font stack. Removing complex-script shaping would regress text.
- The app's `image` dependency has JPEG/PNG decoding enabled, not its entire
  format catalog. Independent archive, image and media worker implementations
  are not linked into this executable. FFmpeg is not its size explanation.

The lockfile contains packages that are not in this target's normal dependency
graph, including Skia; lockfile presence is not linked-size evidence. Likewise,
WGPU feature names for other target operating systems are not proof that those
platform implementations are compiled into the Windows executable.

The first artifact's PDB contained only 511 named Rust procedure records, all
from `std`, `core` or `alloc`. The final-crate map supplies the much stronger
coverage above despite the application/dependencies occupying one fat-LTO object.

## Next measurement and safe boundaries

The first useful experiment is reducing generated control/popup duplication
while preserving behavior. Generated main UI alone is the largest measured
bucket, and its creation/default/drop functions are the four largest individual
bodies. Compare the resulting executable and function ranges using the same map
method; source shrinkage alone is insufficient. Smaller retained component
representations and shared dialog/control bodies are candidates, not proven
savings, and must retain focus, accessibility, menus, tooltips and input behavior.

A separate experiment could qualify a platform-only graphics-backend
configuration if the upstream feature boundary permits it. The map proves that
Vulkan code is present, including its 56,090-byte adapter function; it does not
measure the savings of removing Vulkan or prove that doing so preserves support.
Keep the hardware renderer and software fallback. Changes to backend availability
need driver/startup smoke tests and existing scrolling/resize latency checks on
both supported operating systems. Size-oriented optimization profiles also need
those performance comparisons; `opt-level = "s"`/`"z"` is not a free improvement.
The 5.46 MiB unwind metadata is not an invitation to switch to `panic = "abort"`:
worker-panic containment is an explicit contract. No saving estimate is justified
before controlled builds are measured. No additional optimization build was run.

Diagnostic scripts and results are under `_build/qa/executable-size/`:
`analyze_map.py`, `group_functions.py`, `demangle.rs`, `me-analysis.json`,
`component-analysis.json`, and `demangled.tsv`. The Rust diagnostic uses only the
existing sysroot demangler; no dependency was added to the application or fetched.
The release used final-crate `-C link-arg=/MAP:...`, without global `RUSTFLAGS`.

Other measurements used PowerShell file metadata/SHA-256, Python standard-library
PE/MSF-PDB parsing through existing WSL Python, existing generated source/asset
files, and `cargo tree -p midnight-explorer --target x86_64-pc-windows-msvc -e normal
-f '{p} [{f}]' --locked --offline` with the cached registry configuration. The
coordinator verified the rebuilt native window was visible/responsive with empty
stderr. This investigation changed only this report and ignored diagnostics.
