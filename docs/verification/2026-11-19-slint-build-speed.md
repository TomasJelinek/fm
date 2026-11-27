# Slint UI build speed — 19 November 2026

## Scope

Where the remaining compile time of `midnight-ui-bindings` (the Rust generated from
`crates/midnight-ui/ui/main.slint`) goes after the
[18 November change](2026-11-18-slint-compile-time.md), and two changes to its build
script that need no profile change and do not change what the code does. Baseline is
main `e7e0f390`; the standalone measurements used the generated file of `22fe65fa`
(four `.slint` files differ, 30,562,340 and 30,569,039 bytes). Linux only; nothing
here was built or measured on Windows.

## Result

Debug profile, Docker build image (Ubuntu 24.04, Rust 1.99.1), `jobs = 4` from
`.cargo/config.toml`, four cores with their SMT siblings of a 16-core host.
`cargo build --locked --offline -p midnight-explorer --timings` after touching
`main.slint`, so that only the bindings and what depends on them are rebuilt:

| | main `e7e0f390` | build passes (1, 2) | with the source changes (5, 6) |
| --- | ---: | ---: | ---: |
| Build script (Slint to Rust) | 11.1 s | 11.4 s | 10.1 s |
| `rustc` on the bindings | 187.2 s | 119.2 s | 95.1 s |
| Whole rebuild as reported by Cargo | 202 s | 134 s | 108 s |

The last column is main `c2e62c10`, measured twice with the same result to within
0.1 s.

The first build after the change also recompiled `midnight-ui` beside the bindings,
because their metadata changed: bindings 128.1 s, `midnight-ui` 71.8 s, 147 s in all.

The bindings compiled alone with the dev-profile flags (`rustc` outside Cargo,
opt-level 1, 16 codegen units, no debug info), pinned to four physical cores, runs
back to back:

| Generated file | Wall | User | Peak memory | Object code | Machine code |
| --- | ---: | ---: | ---: | ---: | ---: |
| main | 175.8 to 179.4 s (4 runs) | 222 to 227 s | 5.87 GB | 99.7 MB | 17.53 MB |
| component structs in 16 submodules | 114.4 s | 243.1 s | 4.85 GB | 112.6 MB | 18.90 MB |
| the same without the repeated arms | 110.1 s | 244.3 s | 4.85 GB | 112.6 MB | 18.90 MB |

The split was measured eleven more times in its survey on other cores, with 8, 16 and
32 submodules and several ways of grouping: 113.4 to 120.5 s, mean 116.5 s.

## Causes

### 1. One code generation unit held most of the crate

`rustc` places a method in the code generation unit of the module that defines its
`Self` type, merges the smallest units until `codegen-units` remain, and never
divides a module. The Slint generator writes all 591 components into one module, so
one of the 16 units held 55.8 MB of the 99.7 MB of object code and LLVM optimised it
on one thread: after 68 s of single-threaded front end, that unit alone needed about
117 s (optimisation 35 s, thin local LTO 29 s, machine code 45 s) while the other 15
units needed 51 s of CPU together. More code generation units (64, 256: 177 to 185 s,
still one 55.8 MB object) and more CPUs (eight logical: 181 to 183 s) therefore
changed nothing, and one unit was slower (214 to 225 s).

Fix: `crates/midnight-ui-bindings/build/partition.rs` moves each generated component
struct (`struct Inner…`) into one of 16 private submodules, round-robin. The `impl`
blocks stay in place, because their unit follows the struct. A struct or field
without a visibility gets `pub(super)`, the scope it was private to, and the module
re-exports every part with a glob, so no path, expression or public item changes.
The pass fails the build on a module item it does not know instead of rearranging it.
The largest unit is now 13.5 MB.

### 2. Unreachable arms in the accessibility lookups

Where an element sets an accessible role or action on an instance of a
sub-component, the generator writes two arms with the same item index into
`accessible_role` and `supported_accessibility_actions`: the element's value and
then the sub-component's. Only the first can match. `rustc` examines each
unreachable arm against the source text before it looks at the lint level, which the
generated code allows; in a 30 MB module that is about 9 ms per arm. `check_match`
took 5.3 s, 4.95 s of it in those two functions of each component.

Fix: `crates/midnight-ui-bindings/build/arms.rs` removes an arm whose whole pattern
is a literal that an earlier arm of the same `match index` already matches without a
guard: 569 arms, 77,661 bytes. A match with an arm of any other form than
`literal =>`, `literal ..= literal =>` or `_ =>` is left as it is. Measured in the
survey on the generated text: front end 40.2 to 40.4 s instead of 45.3 to 45.9 s,
full compile 169.0 and 169.7 s instead of 174.8 s, machine code unchanged apart from
the embedded source path.

## Equivalence

- An independent parse of the generated file before and after both passes (with
  `syn`, by a reviewer who did not write them): exactly 569 arms are gone, each an
  unguarded literal arm after an equal unguarded literal arm of the same match, 301
  forwarding `accessible_role(0)` and 268 `supported_accessibility_actions(0)`; all
  591 component structs are in the submodules and equal to the originals apart from
  12,309 `pub(super)` insertions (419 structs, 11,890 fields); the other 3,062 items
  of the module are identical and in the same order, as is everything outside it.
- From outside the generated module a `pub(super)` struct or field stays as
  unreachable as a private one was (checked with `rustc` on a model of the output),
  and the field-offset derive gives its helpers the visibility of the struct and its
  fields.
- With one code generation unit at opt-level s (the release settings of this crate,
  without the final LTO link) the split gives the same 75,458 functions and
  11,205,529 function bytes as the unsplit file of `22fe65fa`.
- In the dev profile the split spreads functions over units differently, so inlining
  can differ; the machine code is 1.4 MB larger (drop glue and generic
  instantiations repeated per unit) and the object files 12.9 MB larger.
- Both passes, run through a copy of the build script, give byte-for-byte the output
  of the survey's prototypes that the standalone measurements were taken on.
- The split refuses a module item it does not know, a component struct that names
  `self` or `super`, and output without component structs (live preview, which this
  workspace does not enable) with a build error.

## Targeted tests and their helpers

Two further changes, on main `e7757bd5`. They change manifests and test code only.

### 3. Targeted UI tests compiled their own Slint and bindings

Cargo unifies features per command. `cargo test -p midnight-ui` selected Slint
without `unstable-wgpu-30` and three shared dependencies with fewer features than
the application (`tracing-core` and `indexmap` without their defaults, `lazy_static`
only as a build dependency), so it compiled Slint, the bindings and `midnight-ui` a
second time, and `cargo test -p midnight-viewer-ui` its own Slint. The workspace
`slint` entry now names the two Slint features the application selects, and
`midnight-ui-input`, which every UI crate builds, names the three dependencies as
the application resolves them.

In the resolved unit graph of eleven package selections the bindings were six
distinct units and are four, and Slint was eight and is six. Built in one target
directory after `cargo build -p midnight-explorer -p midnight-elevation`:

| Command | Reported fresh, same files as the application build |
| --- | --- |
| `cargo test -p midnight-ui --lib --tests --no-run` | Slint, the bindings, `midnight-ui-input`, the `midnight-ui` library |
| `cargo test -p midnight-viewer-ui --lib --tests --no-run` | Slint, `midnight-ui-input`, the `midnight-viewer-ui` library |

Still separate units: `cargo test --workspace` (tokio `test-util` and x11rb `xtest`
come from development dependencies), and `cargo build -p midnight-ui`,
`cargo test -p midnight-compare-ui` and `cargo test -p midnight-ui-bindings` on their
own, which select no platform crate, so that zbus, fontdb and icu_collections resolve
differently. The application's units are unchanged except `midnight-ui-input` and
the crates that depend on it. No package, version or license changes
([third-party record](../third-party.md)).

### 4. Test helpers were compiled once per test module

The `midnight-ui` integration binary compiled `tests/support/ui.rs` 115 times and
`modeless.rs` 51 times, once per including file. Both are now modules of the test
crate that the test modules import; support files that include `modeless.rs`
themselves keep their copy, so 21 of those remain and one of `ui.rs`. Three test
fixtures (`error_notice`, `color_picker`,
`path_elision`) export `Theme`, so that their bindings read the palette at run time
instead of each holding a copy of its constants.

Type-check of the integration target with the host compiler, CPU time and peak
memory: 25.2 s and 2.19 GB before, 23.0 s and 2.01 GB with the shared helpers,
17.4 s and 1.57 GB with the fixtures as well. The 640 `#[test]` functions and the
766 test names of the binary are the same before and after; the name lists were
compared.

Not done: sharing `support/tree_filters.rs`. It includes a `midnight-explorer`
source file whose eight unit tests the binary therefore runs once per including
module; sharing would reduce 766 test names to 654.

## Smaller generated code

Changes to the `.slint` sources, on main `816436b5`. The plain generator output of
the main UI, before the two build passes, is 30.57 MB on that commit; sizes move by a
few KB with the checkout path, which the generated code embeds.

| | Before | After 5 | After 5 and 6 |
| --- | ---: | ---: | ---: |
| Main UI, generated Rust | 30.57 MB | 28.05 MB | 26.97 MB |
| Main UI, generated components (conditional or repeated) | 591 (419) | 577 (407) | 523 (353) |
| Viewer UI, generated Rust | 3.11 MB | 2.99 MB | 2.93 MB |
| Comparator UI, generated Rust | 2.58 MB | 2.46 MB | 2.45 MB |
| Tooltip popup components | 27 | 27 | 8 |

### 5. A shared control is paid for at every use

Every item inside a shared component is repeated in the item tree, item array,
geometry and accessibility tables of each enclosing item tree, and each public
property left at its default costs a `set_constant` per use.

- **Scroll views.** A child with a size but no position enters its parent's layout
  info through eight synthesized properties; for the `Flickable` of `ScrollView`
  and `ScrollStrip` they only ever held the defaults, in 562 binding closures. It now
  has an explicit position. Two scroll views that their non-layout parents then
  merged with their own 50px minimum are positioned too, so those parents merge
  nothing, as before.
- **One-bar scroll views.** A scroll bar whose policy is constantly `always-off` is
  still an instance with 12 items at every use. `VScrollView` and `HScrollView`
  (`ui/scroll-view.slint`) leave that bar out, and `ui/vlist-view.slint` exports a
  `ListView` over `VScrollView`; 18 `ScrollView` and 9 `ListView` sites lose the
  horizontal bar, 8 the vertical one, and the command prompt strip is a
  `ScrollStrip`. Scroll bar instances 112 to 66; the 14 virtualized lists stay
  virtualized.
- **ActionButton.** Each of about 130 buttons held an invisible stock `Button` for
  its focus and keyboard behaviour, with a hidden text and layout. It is replaced by
  a `FocusScope` with the geometry, enabled expression and Space/Return handling of
  the Fluent button's own scope.
- **Inherited controls.** A component that inherits a hinted control is compiled as
  another copy of the stock widget under it. `ArchiveExtensionInput` is written at
  its single use, and `ChecksumOutputPicker` contains a `HintedComboBox` instead of
  inheriting it; one `LineEdit` and one `ComboBox` copy are gone.

Not taken: creating the scroll bar's body only while the bar shows. It removes
0.46 MB from the main UI but adds a conditional component, and 38 KB, to the viewer
and comparator UIs, and the bar's hover and drag state would restart on each
appearance. `WrappingCheckBox` keeps its hidden stock check box, whose keyboard and
accessibility behaviour the component exists to keep.

### 6. Conditional components

Each `if`, `for` and popup is its own generated component: about 0.25 s of `rustc`
and 15 KB of machine code. Without changing layout, accessibility or what a test can
observe:

- `Tip` owns the `text != ""` condition that ten hosts wrapped around it, and 19
  sites that declared a `Tooltip` directly use `Tip`, so they share its popup
  component (27 tooltip popups to 8). Two direct sites remain. A `Tip`'s text is now
  evaluated when its host is created, not when the tooltip opens.
- Neighbouring conditionals with the same condition are merged where the merged
  cells cannot grow or shrink (seven of nine candidate groups).
- Four `for` loops over a constant count are written out.
- Eleven elements that only paint or block input, sit outside layouts and carry no
  accessible role use `visible`, or `enabled` for the three input gates, instead of
  `if`. Eight further candidates stay conditional: a test counts them, they carry an
  accessible role, they would enter a layout, or they would be instantiated in every
  file row.

## Measured and not changed

Bindings compiled alone as above, against 176 s:

- **Thin local LTO off** (`-C lto=off`): 135 to 143 s (seven runs); with the split
  92.1 and 92.3 s. Still not adopted for the reason recorded on 18 November: Cargo
  sets `lto` only for a whole profile, and the renderer and text dependencies built
  at opt-level 2 would lose inlining across their units. Their run-time cost was not
  measured.
- **opt-level `"z"` for the bindings in dev**: 147 to 157 s (three runs) and 42% less
  machine code; `"s"` 186 s. Run-time speed of debug builds and UI tests not measured.
- **opt-level 0**: 72 s, 72% more machine code; not adopted, as on 18 November.
- **Optimising the Slint compiler for the build script**: the script runs in 2.3 s
  with every dependency at opt-level 2, 4.1 s with `i-slint-compiler`, `proc-macro2`,
  `quote`, `rowan`, `smol_str` and `hashbrown` only, 7.5 s with the compiler crate
  only, against 10.9 s; the generator's dependency tree then takes 55 s, 38 s and
  39 s to build instead of 19.5 s.
- **Debug assertions and overflow checks off** for the bindings: 166 and 168 s, 8%
  less machine code. Not adopted: the release profile keeps overflow checks.
- **Release**: 16 code generation units for the bitcode-only compile of the bindings
  take 107.7 s instead of 167.9 s (one run each). Not adopted: the effect on the
  final LTO link and the executable was not measured, and the Windows size and resize
  qualification would have to be repeated.
- The parallel front end (`-Z threads=4`: front end 23 s instead of 46 s, split
  compile 84 s) is unstable and not available on the pinned toolchain.

## Tests

Docker build image, Linux, debug profile, headless software renderer.

`cargo test --locked --offline -p midnight-ui-bindings` and
`cargo test --locked --offline -p midnight-ui -p midnight-ui-input -p midnight-explorer
--lib --bins --tests --no-fail-fast`, on the tree of this change:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-ui-bindings` `build_passes` (the two passes) | 9 | 0 | 0 |
| `midnight-ui` library | 1,232 | 0 | 2 |
| `midnight-ui` integration | 765 | 0 | 1 |
| `midnight-ui-input` library and `placement` | 32 | 0 | 0 |
| `midnight-explorer` binary | 123 | 0 | 17 |

`cargo clippy --locked --offline -p midnight-ui-bindings --all-targets -- -D warnings`
and `rustfmt --check` on the changed files pass.

Run on its own, `cargo test -p midnight-ui-bindings` selects fewer Slint features
than the application and therefore compiled Slint and the bindings a second time
(3 min 12 s); a build that already has the bindings for that feature set compiles
only the test target.

With changes 3 and 4,
`cargo test --locked --offline -p midnight-ui -p midnight-ui-input -p midnight-explorer
-p midnight-viewer-ui -p midnight-compare-ui --lib --bins --tests --no-fail-fast`:

| Target | Passed | Failed | Ignored |
| --- | ---: | ---: | ---: |
| `midnight-ui` library | 1,232 | 0 | 2 |
| `midnight-ui` integration | 765 | 0 | 1 |
| `midnight-ui-input` library and `placement` | 32 | 0 | 0 |
| `midnight-explorer` binary | 123 | 0 | 17 |
| `midnight-viewer-ui` library and integration | 197 | 0 | 0 |
| `midnight-compare-ui` library and integration | 40 | 0 | 0 |

`cargo clippy --locked --offline -p midnight-ui -p midnight-ui-input --all-targets --
-D warnings` passes.

The same command passed with the same counts after change 5 and again after
changes 5 and 6 together; they change `.slint` files only.

## Not verified

- Windows builds and the release executable: the release profile compiles this crate
  as one unit, where the split made no measurable difference, but no release build
  was linked.
- Frame, resize and test run time of debug builds with the split.
- The per-arm cost of unreachable patterns on any compiler other than Rust 1.99.1.
- The Windows dependency graph for change 3 (it could not be resolved offline on the
  Linux host) and a Windows build of the test code of change 4.
- The build time of the integration test target with change 4 inside Cargo; only its
  type-check was timed.
- The running application with changes 5 and 6: hover, painting, scroll bars and
  focus were not looked at, because the application's smoke test renders only a
  320 by 200 window on this host. The evidence is the test suites and comparisons of
  the generated layout, accessibility and item-tree code.
- Run-time cost of change 6's earlier tooltip-text evaluation (one string comparison
  per `Tip` host, including tree rows created while scrolling).
