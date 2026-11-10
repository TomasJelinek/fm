# Cargo-bloat Windows executable measurement

Measured on 8 November 2026 with cargo-bloat 0.12.1. This is a size
investigation; renderer availability, panic containment and release
optimization policy were not changed.

## Existing release at the start of the investigation

`_build/release/me.exe`: **44,860,416 bytes (42.7822 MiB)**.
SHA-256: `cd78439033841ba74ff45340fe738794ab05e0ecfd6a4de712199e3cc2b83b39`.

These PE section lengths are exact occupied file bytes, including alignment:

| Section | File bytes | Share of executable |
| --- | ---: | ---: |
| `.text` | 31,825,408 | 70.943% |
| `.rdata` | 11,377,152 | 25.361% |
| `.data` | 6,144 | 0.014% |
| `.pdata` | 1,415,680 | 3.156% |
| `.rsrc` | 105,472 | 0.235% |
| `.reloc` | 129,536 | 0.289% |
| Headers | 1,024 | 0.002% |

The sections and headers account for the complete file; there is no appended payload.
The separate PDB is 5,328,896 bytes and is not included in the EXE.

## Optimized diagnostic build

Stock cargo-bloat has no analyze-existing-file switch. On MSVC it forces
debug information on and stripping off to obtain names and lengths from PDB.
It was therefore run against an immutable snapshot of the current working
sources in an isolated target directory. The existing release was not replaced.
Fat LTO, one codegen unit, overflow checks, unwinding and the generated UI
package's size optimization remain enabled. This snapshot is newer than the
existing release; the difference is not a controlled optimization comparison.

- Diagnostic EXE: **48,181,248 bytes (45.9492 MiB)**.
- SHA-256: `fb120f90af9ec5779012115c4ef77865e26c9828829bb560c19087377948a1c8`.
- Separate diagnostic PDB: 440,569,856 bytes; not part of the EXE.
- Source manifest SHA-256: `610dc2e7a82ce900e89a05cbad6ccb643d99b9d0423a2556d992726e87c277ba`.

Commands (from the frozen source directory, with the workspace Cargo cache/toolchain
on PATH and offline mode enabled; `$analysisTarget` is an absolute `_build/.../target`):

```powershell
cargo bloat --release --frozen -p midnight-explorer --bin me --crates -n 0 `
    --message-format json --target-dir $analysisTarget -j 2
cargo bloat --release --frozen -p midnight-explorer --bin me -n 0 `
    --message-format json --target-dir $analysisTarget -j 2
```

## Code attributed by cargo-bloat

Sizes below are function bodies attributed to crates by symbol names.
LTO, generic specialization and folded aliases cross crate boundaries; these
are not exact standalone dependency footprints or predicted removal savings.
They exclude read-only tables, unwind metadata, resources and other sections.

| Crate bucket | Function bytes | MiB | Share of `.text` |
| --- | ---: | ---: | ---: |
| `std` | 8,826,450 | 8.418 | 25.67% |
| `midnight_ui_bindings` | 5,991,191 | 5.714 | 17.42% |
| `i_slint_core` | 1,794,233 | 1.711 | 5.22% |
| `naga` | 1,691,951 | 1.614 | 4.92% |
| `tokio` | 1,427,005 | 1.361 | 4.15% |
| `midnight_engine` | 1,244,784 | 1.187 | 3.62% |
| `midnight_ui` | 1,164,474 | 1.111 | 3.39% |
| `wgpu_core` | 796,932 | 0.760 | 2.32% |
| `midnight_viewer_ui` | 777,671 | 0.742 | 2.26% |
| `wgpu_hal` | 743,076 | 0.709 | 2.16% |
| `usvg` | 537,524 | 0.513 | 1.56% |
| `hashbrown` | 470,435 | 0.449 | 1.37% |
| `zeno` | 470,110 | 0.448 | 1.37% |
| `skrifa` | 428,587 | 0.409 | 1.25% |
| `harfrust` | 414,357 | 0.395 | 1.21% |
| `regex_automata` | 405,465 | 0.387 | 1.18% |
| `i_slint_renderer_software` | 334,574 | 0.319 | 0.97% |
| `midnight_compare_ui` | 333,402 | 0.318 | 0.97% |
| `midnight_platform_common` | 321,416 | 0.307 | 0.93% |
| `midnight_platform_windows` | 300,429 | 0.287 | 0.87% |
| `zune_jpeg` | 296,406 | 0.283 | 0.86% |
| `i_slint_backend_winit` | 244,926 | 0.234 | 0.71% |
| `me` | 226,487 | 0.216 | 0.66% |
| `femtovg` | 223,883 | 0.214 | 0.65% |
| `tiny_skia` | 219,780 | 0.210 | 0.64% |
| Remaining crate buckets | 4,510,396 | 4.301 | 13.12% |

Total named function bodies: **34,195,944 bytes** across
**42,330 functions**, against **34,382,848 bytes**
in the `.text` file allocation. The difference includes alignment and code not
represented by the available function records; it is not assigned arbitrarily.

The [complete per-crate CSV](2026-11-08-cargo-bloat-crates.csv) preserves all
214 reported crate buckets and their exact reported byte counts.

## Largest individual functions

The `std` bucket is not 8.83 MB of independent standard-library overhead.
**10,884** functions in that bucket have names containing `midnight_ui_bindings`
and total **4,370,019 bytes**. For example, the main-window destructor below is
classified as `std` because its symbol is `core::ptr::drop_glue<...>`.
Together with the generated UI's own bucket, these disjoint reported records
associate **10,361,210 bytes** with generated main UI types (30.13% of `.text`).
This is symbol association, not a precise feature-removal saving; LTO can inline
other code there and UI-related specializations also appear in other buckets.

| Function | Bytes |
| --- | ---: |
| midnight_engine::impl$4::run::async_fn$0 | 261,939 |
| midnight_ui_bindings::slint_generatedMainWindow::impl$1960::ensure_instantiated | 177,158 |
| midnight_ui::install | 168,482 |
| core::ptr::drop_glue&lt;midnight_ui_bindings::slint_generatedMainWindow::InnerMainWindow&gt; | 123,150 |
| midnight_ui::refresh_properties | 121,425 |
| midnight_ui_bindings::slint_generatedMainWindow::impl$2550::default | 107,104 |
| midnight_viewer_engine::worker::run | 102,934 |
| wgpu_core::command::render::encode_render_pass | 90,626 |
| midnight_ui_bindings::slint_generatedMainWindow::impl$1558::ensure_instantiated | 84,558 |
| me::run_window | 81,980 |
| midnight_plugin_host::dispatch::run | 69,753 |
| resvg::filter::apply_inner | 63,461 |
| i_slint_core::callbacks::impl$1::set_handler::closure$0&lt;tuple$&lt;i32,bool&gt;,tuple$&lt;&gt;,midnight_ui_bindings::slint_generatedMainWindow::impl$1961::on_view_file::closure_env$0&lt;me::viewers::install::closure_env$1&gt; &gt; | 62,112 |
| midnight_engine::operations::run::async_fn$0 | 59,708 |
| midnight_viewer_ui::generated::slint_generatedViewerWindow::InnerViewerWindow::ensure_instantiated | 59,374 |
| usvg::text::flatten::flatten | 58,644 |

## Reproducibility and limits

The diagnostic build took **21 minutes 24 seconds** with two Cargo jobs. The
final full-debug fat-LTO compiler reached approximately 25.9 GB of committed
memory during an observed sample. Further parallel compilation was held during
that phase. This is compiler memory, not the application's runtime requirement.
This is the cost of the one-off symbol-rich analysis, not a debug
iteration benchmark; subsequent unchanged analysis builds took 0.45 seconds.

The frozen snapshot precedes the follow-up shared-toolbar implementation and
PDF/Markdown polish in this chat. The measurements identify this exact artifact;
they do not claim to measure those later source edits.

The first optimization targets are generated UI creation/drop specializations,
large UI callback installation/refresh functions, and the large engine async
dispatcher (261,939 bytes). Shared components and smaller callable boundaries
should be measured with an equivalent source/configuration comparison before
claiming savings. The graphics/shader stack is another substantial contributor;
removing backend features requires support/performance qualification. The PE
resource section, including the executable icon and manifest, occupies only
105,472 bytes in the original executable. This is not a complete inventory of
Slint assets: embedded images and generated SVG data can also reside in read-only
data or generated code. Their individual footprint was not measured here.
Unwinding and overflow
checks remain required; symbol stripping does not remove runtime unwind tables.

Raw per-crate and complete per-function JSON, PE measurements, source input
hashes and build logs are under `_build/qa/cargo-bloat-20261108/`.
The final analysis verifies the original release hash is unchanged and that
the per-function and per-crate attributed totals agree. The source snapshot
contains the existing work in progress; this is not a claim of release qualification.

The optional tool and pinned dependencies are listed in
[the diagnostic license inventory](../dependencies/CARGO_BLOAT.md).
