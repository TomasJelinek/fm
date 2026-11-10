# Third-party dependencies

This inventory describes `main` at `5e04723319e341abcbd33adfad15b569b2f1c9b0`,
audited on 10 November 2026. It lists **561 registry packages** pinned by
Cargo.lock, including **26 direct workspace dependencies**. The eight local
workspace packages are not third-party libraries. Independent viewer/filesystem
plugins are planned in this snapshot and do not yet contribute dependencies.

The [dependency policy](DEPENDENCY_POLICY.md) applies to every dependency and
feature. Each row links to the exact packaged source version; the expressions
below come from that package's Cargo metadata, not an unpinned project homepage.
Legacy `/` expressions are upstream declarations of license alternatives.
No paid license alternative is selected, and this document does not assign a
license to Midnight Explorer itself.

## Scope and reproducibility

The full lockfile includes platform-specific and optional packages which may not
be compiled or shipped. Windows/Linux columns indicate membership in Cargo's
target-filtered **workspace build/test dependency graph**, not proof that a
package is linked into a release binary. Windows has 344 graph nodes and Linux
449, including local crates. Features and packaged root notice filenames are in
[the machine-readable inventory](third-party-inventory.csv). Empty features can
mean a present package with no named features; use the graph columns to tell.

Regenerate metadata with the pinned toolchain and unchanged lockfile:

```sh
cargo metadata --locked --offline --format-version 1
cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-pc-windows-msvc
cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-unknown-linux-gnu
```

Capture these files under `_build/qa/`, inspect each `packages` entry and the
target-filtered `resolve.nodes` feature lists, and compare the inventory before
accepting dependency changes. Metadata may include build/test feature unification;
use compiler artifacts from the actual release build to assemble distribution
contents. An absent root notice filename does not mean a package is unlicensed:
some crates retain notices in source headers or only in their pinned upstream
repository. Fetch and preserve those notices before distribution.

## Slint

The 1.18.1 Slint packages declare GPL-3.0-only, Slint Royalty-free 2.0, and
Slint Software 3.0 as alternatives. This desktop project uses the
[pinned Slint Royalty-free 2.0 terms](https://github.com/slint-ui/slint/blob/v1.18.1/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md),
which permit desktop/mobile/web applications without a mandatory payment.
This is a source-available free desktop alternative, not a claim that these
terms are a permissive open-source license. The paid alternative is not used.

Preserve Slint copyright notices and the required attribution. The application
already includes Slint's `AboutSlint` widget in its About dialog, reachable from
the application menu. Retain it when changing navigation or packaging. The
royalty-free terms restrict embedded use, standalone redistribution of Slint,
and exposing Slint APIs; future API/plugin changes must respect those terms.

For ordinary `MIT OR Apache-2.0` packages either free alternative can apply;
retain the selected text and copyright notices. `AND` expressions require both
sets of obligations. Unicode, BSD, ISC, Zlib, BSL and CC0 notices must likewise
be retained as required by their actual terms. `r-efi` offers MIT/Apache
alternatives; no LGPL-only dependency or paid-only package was identified in
this lockfile's declared license expressions.

## Maintenance and distribution

Cargo-audit 0.22.2 against RustSec revision
`28d73fc95e5972af3403225a217def57dcaba3c4` (10 November 2026) reported
**zero known vulnerabilities** and six unmaintained-package advisories:

| Package | Advisory | Target graph |
| --- | --- | --- |
| bincode 2.0.1 | RUSTSEC-2025-0141 | Windows, Linux |
| pin-utils 0.1.1 | RUSTSEC-2026-0441 | Windows, Linux |
| ttf-parser 0.25.1 | RUSTSEC-2026-0192 | Linux |
| zbus_names 4.3.5 | RUSTSEC-2026-0371 | Linux |
| zvariant 5.15.1 | RUSTSEC-2026-0370 | Linux |
| zvariant_derive 5.15.1 | RUSTSEC-2026-0372 | Linux |

These are transitive maintenance risks, not suppressed findings or evidence of
an exploitable defect. Follow the parent libraries' migrations and re-run the
audit after upgrades. The scan covers the lockfile, not OS libraries or every
native artifact downloaded during a build. Recheck at release time.

Before distributing: collect the actual shipped crates' and bundled native
libraries' full licenses/copyright/NOTICE texts; include Slint's selected terms;
verify About attribution in the built application. External OS tools and system
libraries have their own licenses and are not automatically redistributed by
Cargo. Build/test-only tools need their own license checks even when not shipped.
The project distribution license remains an explicit unresolved project decision;
this audit does not silently choose one.

## Locked package inventory

| Package | Version | Direct | Pinned license expression | Windows | Linux |
| --- | --- | --- | --- | --- | --- |
| [ab_glyph](https://docs.rs/crate/ab_glyph/0.2.32/source/) | 0.2.32 |  | Apache-2.0 |  | Yes |
| [ab_glyph_rasterizer](https://docs.rs/crate/ab_glyph_rasterizer/0.1.10/source/) | 0.1.10 |  | Apache-2.0 |  | Yes |
| [adler2](https://docs.rs/crate/adler2/2.0.1/source/) | 2.0.1 |  | 0BSD OR MIT OR Apache-2.0 | Yes | Yes |
| [ahash](https://docs.rs/crate/ahash/0.8.12/source/) | 0.8.12 |  | MIT OR Apache-2.0 |  | Yes |
| [aho-corasick](https://docs.rs/crate/aho-corasick/1.1.5/source/) | 1.1.5 |  | Unlicense OR MIT | Yes | Yes |
| [allocator-api2](https://docs.rs/crate/allocator-api2/0.2.21/source/) | 0.2.21 |  | MIT OR Apache-2.0 | Yes | Yes |
| [android_system_properties](https://docs.rs/crate/android_system_properties/0.1.6/source/) | 0.1.6 |  | MIT OR Apache-2.0 |  |  |
| [android-activity](https://docs.rs/crate/android-activity/0.6.1/source/) | 0.6.1 |  | MIT OR Apache-2.0 |  |  |
| [android-properties](https://docs.rs/crate/android-properties/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [annotate-snippets](https://docs.rs/crate/annotate-snippets/0.12.17/source/) | 0.12.17 |  | MIT OR Apache-2.0 | Yes | Yes |
| [anstyle](https://docs.rs/crate/anstyle/1.0.14/source/) | 1.0.14 |  | MIT OR Apache-2.0 | Yes | Yes |
| [arboard](https://docs.rs/crate/arboard/3.7.0/source/) | 3.7.0 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [arrayref](https://docs.rs/crate/arrayref/0.3.9/source/) | 0.3.9 |  | BSD-2-Clause | Yes | Yes |
| [arrayvec](https://docs.rs/crate/arrayvec/0.7.8/source/) | 0.7.8 |  | MIT OR Apache-2.0 | Yes | Yes |
| [as-raw-xcb-connection](https://docs.rs/crate/as-raw-xcb-connection/1.0.1/source/) | 1.0.1 |  | MIT OR Apache-2.0 |  | Yes |
| [ash](https://docs.rs/crate/ash/0.38.0+1.3.281/source/) | 0.38.0+1.3.281 |  | MIT OR Apache-2.0 | Yes | Yes |
| [async-broadcast](https://docs.rs/crate/async-broadcast/0.7.2/source/) | 0.7.2 |  | MIT OR Apache-2.0 |  | Yes |
| [async-channel](https://docs.rs/crate/async-channel/2.5.0/source/) | 2.5.0 |  | Apache-2.0 OR MIT |  | Yes |
| [async-executor](https://docs.rs/crate/async-executor/1.14.1/source/) | 1.14.1 |  | Apache-2.0 OR MIT |  | Yes |
| [async-io](https://docs.rs/crate/async-io/2.6.0/source/) | 2.6.0 |  | Apache-2.0 OR MIT |  | Yes |
| [async-lock](https://docs.rs/crate/async-lock/3.5.0/source/) | 3.5.0 |  | Apache-2.0 OR MIT |  | Yes |
| [async-process](https://docs.rs/crate/async-process/2.5.0/source/) | 2.5.0 |  | Apache-2.0 OR MIT |  | Yes |
| [async-recursion](https://docs.rs/crate/async-recursion/1.2.0/source/) | 1.2.0 |  | MIT OR Apache-2.0 |  | Yes |
| [async-signal](https://docs.rs/crate/async-signal/0.2.14/source/) | 0.2.14 |  | Apache-2.0 OR MIT |  | Yes |
| [async-task](https://docs.rs/crate/async-task/4.7.1/source/) | 4.7.1 |  | Apache-2.0 OR MIT |  | Yes |
| [async-trait](https://docs.rs/crate/async-trait/0.1.92/source/) | 0.1.92 |  | MIT OR Apache-2.0 |  | Yes |
| [atomic-waker](https://docs.rs/crate/atomic-waker/1.1.2/source/) | 1.1.2 |  | Apache-2.0 OR MIT |  | Yes |
| [autocfg](https://docs.rs/crate/autocfg/1.5.1/source/) | 1.5.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [base64](https://docs.rs/crate/base64/0.23.1/source/) | 0.23.1 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [bincode](https://docs.rs/crate/bincode/2.0.1/source/) | 2.0.1 |  | MIT | Yes | Yes |
| [bindgen](https://docs.rs/crate/bindgen/0.72.1/source/) | 0.72.1 |  | BSD-3-Clause | Yes | Yes |
| [bit-set](https://docs.rs/crate/bit-set/0.10.0/source/) | 0.10.0 |  | Apache-2.0 OR MIT | Yes | Yes |
| [bit-vec](https://docs.rs/crate/bit-vec/0.9.1/source/) | 0.9.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [bitflags](https://docs.rs/crate/bitflags/1.3.2/source/) | 1.3.2 |  | MIT/Apache-2.0 |  |  |
| [bitflags](https://docs.rs/crate/bitflags/2.13.2/source/) | 2.13.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [block2](https://docs.rs/crate/block2/0.5.1/source/) | 0.5.1 |  | MIT |  |  |
| [block2](https://docs.rs/crate/block2/0.6.2/source/) | 0.6.2 |  | MIT |  |  |
| [blocking](https://docs.rs/crate/blocking/1.7.0/source/) | 1.7.0 |  | Apache-2.0 OR MIT |  | Yes |
| [borsh](https://docs.rs/crate/borsh/1.8.1/source/) | 1.8.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [bumpalo](https://docs.rs/crate/bumpalo/3.20.3/source/) | 3.20.3 |  | MIT OR Apache-2.0 |  |  |
| [by_address](https://docs.rs/crate/by_address/1.2.1/source/) | 1.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [bytemuck](https://docs.rs/crate/bytemuck/1.25.2/source/) | 1.25.2 |  | Zlib OR Apache-2.0 OR MIT | Yes | Yes |
| [bytemuck_derive](https://docs.rs/crate/bytemuck_derive/1.12.1/source/) | 1.12.1 |  | Zlib OR Apache-2.0 OR MIT | Yes | Yes |
| [byteorder-lite](https://docs.rs/crate/byteorder-lite/0.1.0/source/) | 0.1.0 |  | Unlicense OR MIT | Yes | Yes |
| [bytes](https://docs.rs/crate/bytes/1.12.1/source/) | 1.12.1 |  | MIT | Yes | Yes |
| [calloop](https://docs.rs/crate/calloop/0.13.0/source/) | 0.13.0 |  | MIT |  | Yes |
| [calloop](https://docs.rs/crate/calloop/0.14.5/source/) | 0.14.5 |  | MIT |  | Yes |
| [calloop-wayland-source](https://docs.rs/crate/calloop-wayland-source/0.3.0/source/) | 0.3.0 |  | MIT |  | Yes |
| [cc](https://docs.rs/crate/cc/1.11.0/source/) | 1.11.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [cexpr](https://docs.rs/crate/cexpr/0.6.0/source/) | 0.6.0 |  | Apache-2.0/MIT | Yes | Yes |
| [cfg_aliases](https://docs.rs/crate/cfg_aliases/0.2.2/source/) | 0.2.2 |  | MIT | Yes | Yes |
| [cfg-if](https://docs.rs/crate/cfg-if/1.0.5/source/) | 1.0.5 |  | MIT OR Apache-2.0 | Yes | Yes |
| [cgl](https://docs.rs/crate/cgl/0.3.2/source/) | 0.3.2 |  | MIT / Apache-2.0 |  |  |
| [chrono](https://docs.rs/crate/chrono/0.4.45/source/) | 0.4.45 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [clang-sys](https://docs.rs/crate/clang-sys/1.9.1/source/) | 1.9.1 |  | Apache-2.0 | Yes | Yes |
| [clipboard-win](https://docs.rs/crate/clipboard-win/5.4.3/source/) | 5.4.3 |  | BSL-1.0 | Yes |  |
| [clru](https://docs.rs/crate/clru/0.6.3/source/) | 0.6.3 |  | MIT | Yes | Yes |
| [codespan-reporting](https://docs.rs/crate/codespan-reporting/0.13.1/source/) | 0.13.1 |  | Apache-2.0 | Yes | Yes |
| [color_quant](https://docs.rs/crate/color_quant/1.1.0/source/) | 1.1.0 |  | MIT | Yes | Yes |
| [combine](https://docs.rs/crate/combine/4.6.9/source/) | 4.6.9 |  | MIT |  |  |
| [concurrent-queue](https://docs.rs/crate/concurrent-queue/2.5.0/source/) | 2.5.0 |  | Apache-2.0 OR MIT |  | Yes |
| [const-field-offset](https://docs.rs/crate/const-field-offset/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [const-field-offset-macro](https://docs.rs/crate/const-field-offset-macro/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [convert_case](https://docs.rs/crate/convert_case/0.10.0/source/) | 0.10.0 |  | MIT | Yes | Yes |
| [core-foundation](https://docs.rs/crate/core-foundation/0.9.4/source/) | 0.9.4 |  | MIT OR Apache-2.0 |  |  |
| [core-foundation-sys](https://docs.rs/crate/core-foundation-sys/0.8.7/source/) | 0.8.7 |  | MIT OR Apache-2.0 |  |  |
| [core-graphics](https://docs.rs/crate/core-graphics/0.23.2/source/) | 0.23.2 |  | MIT OR Apache-2.0 |  |  |
| [core-graphics-types](https://docs.rs/crate/core-graphics-types/0.1.3/source/) | 0.1.3 |  | MIT OR Apache-2.0 |  |  |
| [countme](https://docs.rs/crate/countme/3.0.1/source/) | 3.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [crc32fast](https://docs.rs/crate/crc32fast/1.5.4/source/) | 1.5.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [critical-section](https://docs.rs/crate/critical-section/1.2.0/source/) | 1.2.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [crossbeam-channel](https://docs.rs/crate/crossbeam-channel/0.5.17/source/) | 0.5.17 |  | MIT OR Apache-2.0 | Yes |  |
| [crossbeam-deque](https://docs.rs/crate/crossbeam-deque/0.8.8/source/) | 0.8.8 |  | MIT OR Apache-2.0 | Yes | Yes |
| [crossbeam-epoch](https://docs.rs/crate/crossbeam-epoch/0.9.21/source/) | 0.9.21 |  | MIT OR Apache-2.0 | Yes | Yes |
| [crossbeam-utils](https://docs.rs/crate/crossbeam-utils/0.8.23/source/) | 0.8.23 |  | MIT OR Apache-2.0 | Yes | Yes |
| [crunchy](https://docs.rs/crate/crunchy/0.2.4/source/) | 0.2.4 |  | MIT |  |  |
| [ctor](https://docs.rs/crate/ctor/0.10.1/source/) | 0.10.1 |  | Apache-2.0 OR MIT |  | Yes |
| [cursor-icon](https://docs.rs/crate/cursor-icon/1.2.0/source/) | 1.2.0 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [data-url](https://docs.rs/crate/data-url/0.3.2/source/) | 0.3.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [derive_more](https://docs.rs/crate/derive_more/2.1.1/source/) | 2.1.1 |  | MIT | Yes | Yes |
| [derive_more-impl](https://docs.rs/crate/derive_more-impl/2.1.1/source/) | 2.1.1 |  | MIT | Yes | Yes |
| [dispatch](https://docs.rs/crate/dispatch/0.2.0/source/) | 0.2.0 |  | MIT |  |  |
| [dispatch2](https://docs.rs/crate/dispatch2/0.3.1/source/) | 0.3.1 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [displaydoc](https://docs.rs/crate/displaydoc/0.2.7/source/) | 0.2.7 |  | MIT OR Apache-2.0 | Yes | Yes |
| [dlib](https://docs.rs/crate/dlib/0.5.3/source/) | 0.5.3 |  | MIT |  | Yes |
| [document-features](https://docs.rs/crate/document-features/0.2.12/source/) | 0.2.12 |  | MIT OR Apache-2.0 | Yes | Yes |
| [downcast-rs](https://docs.rs/crate/downcast-rs/1.2.1/source/) | 1.2.1 |  | MIT/Apache-2.0 |  | Yes |
| [dpi](https://docs.rs/crate/dpi/0.1.2/source/) | 0.1.2 |  | Apache-2.0 AND MIT | Yes | Yes |
| [drm](https://docs.rs/crate/drm/0.14.1/source/) | 0.14.1 |  | MIT |  | Yes |
| [drm](https://docs.rs/crate/drm/0.15.0/source/) | 0.15.0 |  | MIT |  | Yes |
| [drm-ffi](https://docs.rs/crate/drm-ffi/0.9.1/source/) | 0.9.1 |  | MIT |  | Yes |
| [drm-fourcc](https://docs.rs/crate/drm-fourcc/2.2.0/source/) | 2.2.0 |  | MIT |  | Yes |
| [drm-sys](https://docs.rs/crate/drm-sys/0.8.1/source/) | 0.8.1 |  | MIT |  | Yes |
| [dtor](https://docs.rs/crate/dtor/0.8.1/source/) | 0.8.1 |  | Apache-2.0 OR MIT |  | Yes |
| [either](https://docs.rs/crate/either/1.19.0/source/) | 1.19.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [endi](https://docs.rs/crate/endi/1.1.1/source/) | 1.1.1 |  | MIT |  | Yes |
| [enumflags2](https://docs.rs/crate/enumflags2/0.7.12/source/) | 0.7.12 |  | MIT OR Apache-2.0 |  | Yes |
| [enumflags2_derive](https://docs.rs/crate/enumflags2_derive/0.7.12/source/) | 0.7.12 |  | MIT OR Apache-2.0 |  | Yes |
| [equivalent](https://docs.rs/crate/equivalent/1.0.3/source/) | 1.0.3 |  | Apache-2.0 OR MIT | Yes | Yes |
| [errno](https://docs.rs/crate/errno/0.3.14/source/) | 0.3.14 |  | MIT OR Apache-2.0 |  | Yes |
| [error-code](https://docs.rs/crate/error-code/3.4.0/source/) | 3.4.0 |  | BSL-1.0 | Yes |  |
| [euclid](https://docs.rs/crate/euclid/0.22.14/source/) | 0.22.14 |  | MIT OR Apache-2.0 | Yes | Yes |
| [event-listener](https://docs.rs/crate/event-listener/5.4.2/source/) | 5.4.2 |  | Apache-2.0 OR MIT |  | Yes |
| [event-listener-strategy](https://docs.rs/crate/event-listener-strategy/0.5.4/source/) | 0.5.4 |  | Apache-2.0 OR MIT |  | Yes |
| [fastrand](https://docs.rs/crate/fastrand/2.5.0/source/) | 2.5.0 |  | Apache-2.0 OR MIT | Yes | Yes |
| [fdeflate](https://docs.rs/crate/fdeflate/0.3.7/source/) | 0.3.7 |  | MIT OR Apache-2.0 | Yes | Yes |
| [femtovg](https://docs.rs/crate/femtovg/0.27.0/source/) | 0.27.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [field-offset](https://docs.rs/crate/field-offset/0.3.6/source/) | 0.3.6 |  | MIT OR Apache-2.0 | Yes | Yes |
| [filetime](https://docs.rs/crate/filetime/0.2.29/source/) | 0.2.29 |  | MIT/Apache-2.0 | Yes | Yes |
| [find-msvc-tools](https://docs.rs/crate/find-msvc-tools/0.1.15/source/) | 0.1.15 |  | MIT OR Apache-2.0 | Yes | Yes |
| [fixed_decimal](https://docs.rs/crate/fixed_decimal/0.7.3/source/) | 0.7.3 |  | Unicode-3.0 | Yes | Yes |
| [fixedbitset](https://docs.rs/crate/fixedbitset/0.5.7/source/) | 0.5.7 |  | MIT OR Apache-2.0 |  | Yes |
| [flate2](https://docs.rs/crate/flate2/1.1.10/source/) | 1.1.10 |  | MIT OR Apache-2.0 | Yes | Yes |
| [float-cmp](https://docs.rs/crate/float-cmp/0.9.0/source/) | 0.9.0 |  | MIT | Yes | Yes |
| [fnv](https://docs.rs/crate/fnv/1.0.7/source/) | 1.0.7 |  | Apache-2.0 / MIT | Yes | Yes |
| [foldhash](https://docs.rs/crate/foldhash/0.1.5/source/) | 0.1.5 |  | Zlib |  | Yes |
| [foldhash](https://docs.rs/crate/foldhash/0.2.0/source/) | 0.2.0 |  | Zlib | Yes | Yes |
| [font-types](https://docs.rs/crate/font-types/0.12.7/source/) | 0.12.7 |  | MIT OR Apache-2.0 | Yes | Yes |
| [fontdb](https://docs.rs/crate/fontdb/0.24.0/source/) | 0.24.0 | Yes | MIT | Yes | Yes |
| [fontique](https://docs.rs/crate/fontique/0.11.1/source/) | 0.11.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [foreign-types](https://docs.rs/crate/foreign-types/0.5.0/source/) | 0.5.0 |  | MIT/Apache-2.0 |  |  |
| [foreign-types-macros](https://docs.rs/crate/foreign-types-macros/0.2.4/source/) | 0.2.4 |  | MIT/Apache-2.0 |  |  |
| [foreign-types-shared](https://docs.rs/crate/foreign-types-shared/0.3.1/source/) | 0.3.1 |  | MIT/Apache-2.0 |  |  |
| [form_urlencoded](https://docs.rs/crate/form_urlencoded/1.2.2/source/) | 1.2.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [fs2](https://docs.rs/crate/fs2/0.4.3/source/) | 0.4.3 | Yes | MIT/Apache-2.0 | Yes | Yes |
| [futures](https://docs.rs/crate/futures/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-channel](https://docs.rs/crate/futures-channel/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-core](https://docs.rs/crate/futures-core/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-executor](https://docs.rs/crate/futures-executor/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-io](https://docs.rs/crate/futures-io/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-lite](https://docs.rs/crate/futures-lite/2.6.1/source/) | 2.6.1 |  | Apache-2.0 OR MIT |  | Yes |
| [futures-macro](https://docs.rs/crate/futures-macro/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-sink](https://docs.rs/crate/futures-sink/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-task](https://docs.rs/crate/futures-task/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [futures-util](https://docs.rs/crate/futures-util/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 |  | Yes |
| [gethostname](https://docs.rs/crate/gethostname/1.1.0/source/) | 1.1.0 |  | Apache-2.0 |  | Yes |
| [getrandom](https://docs.rs/crate/getrandom/0.3.4/source/) | 0.3.4 |  | MIT OR Apache-2.0 |  | Yes |
| [getrandom](https://docs.rs/crate/getrandom/0.4.3/source/) | 0.4.3 |  | MIT OR Apache-2.0 | Yes | Yes |
| [gif](https://docs.rs/crate/gif/0.14.2/source/) | 0.14.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [gl_generator](https://docs.rs/crate/gl_generator/0.14.0/source/) | 0.14.0 |  | Apache-2.0 | Yes | Yes |
| [glob](https://docs.rs/crate/glob/0.3.4/source/) | 0.3.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [glow](https://docs.rs/crate/glow/0.17.0/source/) | 0.17.0 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [glow](https://docs.rs/crate/glow/0.18.0/source/) | 0.18.0 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [glutin](https://docs.rs/crate/glutin/0.32.3/source/) | 0.32.3 |  | Apache-2.0 | Yes | Yes |
| [glutin_egl_sys](https://docs.rs/crate/glutin_egl_sys/0.7.1/source/) | 0.7.1 |  | Apache-2.0 | Yes | Yes |
| [glutin_glx_sys](https://docs.rs/crate/glutin_glx_sys/0.6.1/source/) | 0.6.1 |  | Apache-2.0 |  | Yes |
| [glutin_wgl_sys](https://docs.rs/crate/glutin_wgl_sys/0.6.1/source/) | 0.6.1 |  | Apache-2.0 | Yes |  |
| [glutin-winit](https://docs.rs/crate/glutin-winit/0.5.0/source/) | 0.5.0 |  | MIT | Yes | Yes |
| [gpu-allocator](https://docs.rs/crate/gpu-allocator/0.28.0/source/) | 0.28.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [grid](https://docs.rs/crate/grid/1.0.1/source/) | 1.0.1 |  | MIT | Yes | Yes |
| [half](https://docs.rs/crate/half/2.7.1/source/) | 2.7.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [harfrust](https://docs.rs/crate/harfrust/0.12.0/source/) | 0.12.0 |  | MIT | Yes | Yes |
| [hashbrown](https://docs.rs/crate/hashbrown/0.14.5/source/) | 0.14.5 |  | MIT OR Apache-2.0 | Yes | Yes |
| [hashbrown](https://docs.rs/crate/hashbrown/0.15.5/source/) | 0.15.5 |  | MIT OR Apache-2.0 |  | Yes |
| [hashbrown](https://docs.rs/crate/hashbrown/0.16.1/source/) | 0.16.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [hashbrown](https://docs.rs/crate/hashbrown/0.17.1/source/) | 0.17.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [heck](https://docs.rs/crate/heck/0.5.0/source/) | 0.5.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [hermit-abi](https://docs.rs/crate/hermit-abi/0.3.9/source/) | 0.3.9 |  | MIT OR Apache-2.0 |  |  |
| [hermit-abi](https://docs.rs/crate/hermit-abi/0.5.3/source/) | 0.5.3 |  | MIT OR Apache-2.0 |  |  |
| [hex](https://docs.rs/crate/hex/0.4.3/source/) | 0.4.3 |  | MIT OR Apache-2.0 |  | Yes |
| [htmlparser](https://docs.rs/crate/htmlparser/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [i-slint-backend-linuxkms](https://docs.rs/crate/i-slint-backend-linuxkms/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 |  | Yes |
| [i-slint-backend-selector](https://docs.rs/crate/i-slint-backend-selector/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-backend-testing](https://docs.rs/crate/i-slint-backend-testing/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-backend-winit](https://docs.rs/crate/i-slint-backend-winit/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-common](https://docs.rs/crate/i-slint-common/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-compiler](https://docs.rs/crate/i-slint-compiler/1.18.1/source/) | 1.18.1 | Yes | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-core](https://docs.rs/crate/i-slint-core/1.18.1/source/) | 1.18.1 | Yes | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-core-macros](https://docs.rs/crate/i-slint-core-macros/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-renderer-femtovg](https://docs.rs/crate/i-slint-renderer-femtovg/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-renderer-skia](https://docs.rs/crate/i-slint-renderer-skia/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [i-slint-renderer-software](https://docs.rs/crate/i-slint-renderer-software/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [iana-time-zone](https://docs.rs/crate/iana-time-zone/0.1.65/source/) | 0.1.65 |  | MIT OR Apache-2.0 |  | Yes |
| [iana-time-zone-haiku](https://docs.rs/crate/iana-time-zone-haiku/0.1.2/source/) | 0.1.2 |  | MIT OR Apache-2.0 |  |  |
| [icu_collections](https://docs.rs/crate/icu_collections/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_decimal](https://docs.rs/crate/icu_decimal/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_decimal_data](https://docs.rs/crate/icu_decimal_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_locale_core](https://docs.rs/crate/icu_locale_core/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_locale_fallback](https://docs.rs/crate/icu_locale_fallback/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_locale_fallback_data](https://docs.rs/crate/icu_locale_fallback_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_normalizer](https://docs.rs/crate/icu_normalizer/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_normalizer_data](https://docs.rs/crate/icu_normalizer_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_plurals](https://docs.rs/crate/icu_plurals/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_plurals_data](https://docs.rs/crate/icu_plurals_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_properties](https://docs.rs/crate/icu_properties/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_properties_data](https://docs.rs/crate/icu_properties_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_provider](https://docs.rs/crate/icu_provider/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_segmenter](https://docs.rs/crate/icu_segmenter/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [icu_segmenter_data](https://docs.rs/crate/icu_segmenter_data/2.4.0/source/) | 2.4.0 |  | Unicode-3.0 | Yes | Yes |
| [idna](https://docs.rs/crate/idna/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [idna_adapter](https://docs.rs/crate/idna_adapter/1.2.2/source/) | 1.2.2 |  | Apache-2.0 OR MIT | Yes | Yes |
| [image](https://docs.rs/crate/image/0.25.10/source/) | 0.25.10 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [image-webp](https://docs.rs/crate/image-webp/0.2.4/source/) | 0.2.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [imagesize](https://docs.rs/crate/imagesize/0.15.0/source/) | 0.15.0 |  | MIT | Yes | Yes |
| [imgref](https://docs.rs/crate/imgref/1.12.3/source/) | 1.12.3 |  | CC0-1.0 OR Apache-2.0 | Yes | Yes |
| [indexmap](https://docs.rs/crate/indexmap/2.14.2/source/) | 2.14.2 |  | Apache-2.0 OR MIT | Yes | Yes |
| [inotify](https://docs.rs/crate/inotify/0.11.5/source/) | 0.11.5 |  | ISC |  | Yes |
| [inotify-sys](https://docs.rs/crate/inotify-sys/0.1.8/source/) | 0.1.8 |  | ISC |  | Yes |
| [input](https://docs.rs/crate/input/0.10.0/source/) | 0.10.0 |  | MIT |  | Yes |
| [input-sys](https://docs.rs/crate/input-sys/1.19.0/source/) | 1.19.0 |  | MIT |  | Yes |
| [io-lifetimes](https://docs.rs/crate/io-lifetimes/1.0.11/source/) | 1.0.11 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [itertools](https://docs.rs/crate/itertools/0.13.0/source/) | 0.13.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [itertools](https://docs.rs/crate/itertools/0.15.0/source/) | 0.15.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [itoa](https://docs.rs/crate/itoa/1.0.18/source/) | 1.0.18 |  | MIT OR Apache-2.0 | Yes | Yes |
| [jni](https://docs.rs/crate/jni/0.22.4/source/) | 0.22.4 |  | MIT OR Apache-2.0 |  |  |
| [jni-macros](https://docs.rs/crate/jni-macros/0.22.4/source/) | 0.22.4 |  | MIT OR Apache-2.0 |  |  |
| [jni-sys](https://docs.rs/crate/jni-sys/0.3.1/source/) | 0.3.1 |  | MIT OR Apache-2.0 |  |  |
| [jni-sys](https://docs.rs/crate/jni-sys/0.4.1/source/) | 0.4.1 |  | MIT OR Apache-2.0 |  |  |
| [jni-sys-macros](https://docs.rs/crate/jni-sys-macros/0.4.1/source/) | 0.4.1 |  | MIT OR Apache-2.0 |  |  |
| [jobserver](https://docs.rs/crate/jobserver/0.1.35/source/) | 0.1.35 |  | MIT OR Apache-2.0 | Yes | Yes |
| [js-sys](https://docs.rs/crate/js-sys/0.3.107/source/) | 0.3.107 |  | MIT OR Apache-2.0 |  |  |
| [keyboard-types](https://docs.rs/crate/keyboard-types/0.7.0/source/) | 0.7.0 |  | MIT OR Apache-2.0 | Yes |  |
| [khronos_api](https://docs.rs/crate/khronos_api/3.1.0/source/) | 3.1.0 |  | Apache-2.0 | Yes | Yes |
| [khronos-egl](https://docs.rs/crate/khronos-egl/6.0.0/source/) | 6.0.0 |  | MIT/Apache-2.0 | Yes | Yes |
| [kqueue](https://docs.rs/crate/kqueue/1.2.1/source/) | 1.2.1 |  | MIT |  |  |
| [kqueue-sys](https://docs.rs/crate/kqueue-sys/1.1.2/source/) | 1.1.2 |  | MIT |  |  |
| [kurbo](https://docs.rs/crate/kurbo/0.13.1/source/) | 0.13.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [lazy_static](https://docs.rs/crate/lazy_static/1.5.1/source/) | 1.5.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [libc](https://docs.rs/crate/libc/0.2.191/source/) | 0.2.191 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [libloading](https://docs.rs/crate/libloading/0.8.9/source/) | 0.8.9 |  | ISC | Yes | Yes |
| [libm](https://docs.rs/crate/libm/0.2.16/source/) | 0.2.16 |  | MIT | Yes | Yes |
| [libredox](https://docs.rs/crate/libredox/0.1.25/source/) | 0.1.25 |  | MIT |  |  |
| [libseat](https://docs.rs/crate/libseat/0.2.4/source/) | 0.2.4 |  | MIT |  | Yes |
| [libseat-sys](https://docs.rs/crate/libseat-sys/0.2.0/source/) | 0.2.0 |  | MIT |  | Yes |
| [libudev-sys](https://docs.rs/crate/libudev-sys/0.1.4/source/) | 0.1.4 |  | MIT |  | Yes |
| [linebender_resource_handle](https://docs.rs/crate/linebender_resource_handle/0.1.1/source/) | 0.1.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [linux-raw-sys](https://docs.rs/crate/linux-raw-sys/0.12.1/source/) | 0.12.1 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [linux-raw-sys](https://docs.rs/crate/linux-raw-sys/0.4.15/source/) | 0.4.15 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [linux-raw-sys](https://docs.rs/crate/linux-raw-sys/0.9.4/source/) | 0.9.4 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [litemap](https://docs.rs/crate/litemap/0.8.3/source/) | 0.8.3 |  | Unicode-3.0 | Yes | Yes |
| [litrs](https://docs.rs/crate/litrs/1.0.1/source/) | 1.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [lock_api](https://docs.rs/crate/lock_api/0.4.14/source/) | 0.4.14 |  | MIT OR Apache-2.0 | Yes | Yes |
| [log](https://docs.rs/crate/log/0.4.35/source/) | 0.4.35 |  | MIT OR Apache-2.0 | Yes | Yes |
| [lyon_algorithms](https://docs.rs/crate/lyon_algorithms/1.0.22/source/) | 1.0.22 |  | MIT OR Apache-2.0 | Yes | Yes |
| [lyon_extra](https://docs.rs/crate/lyon_extra/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [lyon_geom](https://docs.rs/crate/lyon_geom/1.0.20/source/) | 1.0.20 |  | MIT OR Apache-2.0 | Yes | Yes |
| [lyon_path](https://docs.rs/crate/lyon_path/1.0.19/source/) | 1.0.19 |  | MIT OR Apache-2.0 | Yes | Yes |
| [matchers](https://docs.rs/crate/matchers/0.2.0/source/) | 0.2.0 |  | MIT | Yes | Yes |
| [memchr](https://docs.rs/crate/memchr/2.8.3/source/) | 2.8.3 |  | Unlicense OR MIT | Yes | Yes |
| [memmap2](https://docs.rs/crate/memmap2/0.9.12/source/) | 0.9.12 |  | MIT OR Apache-2.0 | Yes | Yes |
| [memoffset](https://docs.rs/crate/memoffset/0.9.1/source/) | 0.9.1 |  | MIT | Yes | Yes |
| [minimal-lexical](https://docs.rs/crate/minimal-lexical/0.2.1/source/) | 0.2.1 |  | MIT/Apache-2.0 | Yes | Yes |
| [miniz_oxide](https://docs.rs/crate/miniz_oxide/0.8.9/source/) | 0.8.9 |  | MIT OR Zlib OR Apache-2.0 | Yes | Yes |
| [miniz_oxide](https://docs.rs/crate/miniz_oxide/0.9.1/source/) | 0.9.1 |  | MIT OR Zlib OR Apache-2.0 | Yes | Yes |
| [mio](https://docs.rs/crate/mio/1.2.4/source/) | 1.2.4 |  | MIT |  | Yes |
| [moxcms](https://docs.rs/crate/moxcms/0.8.1/source/) | 0.8.1 |  | BSD-3-Clause OR Apache-2.0 | Yes | Yes |
| [muda](https://docs.rs/crate/muda/0.19.3/source/) | 0.19.3 |  | Apache-2.0 OR MIT | Yes |  |
| [naga](https://docs.rs/crate/naga/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [naga-types](https://docs.rs/crate/naga-types/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [natord](https://docs.rs/crate/natord/1.0.9/source/) | 1.0.9 |  | MIT | Yes | Yes |
| [ndk](https://docs.rs/crate/ndk/0.9.0/source/) | 0.9.0 |  | MIT OR Apache-2.0 |  |  |
| [ndk-context](https://docs.rs/crate/ndk-context/0.1.1/source/) | 0.1.1 |  | MIT OR Apache-2.0 |  |  |
| [ndk-sys](https://docs.rs/crate/ndk-sys/0.6.0+11769913/source/) | 0.6.0+11769913 |  | MIT OR Apache-2.0 |  |  |
| [nix](https://docs.rs/crate/nix/0.31.3/source/) | 0.31.3 |  | MIT |  | Yes |
| [nom](https://docs.rs/crate/nom/7.1.3/source/) | 7.1.3 |  | MIT | Yes | Yes |
| [nom](https://docs.rs/crate/nom/8.0.0/source/) | 8.0.0 |  | MIT |  | Yes |
| [notify](https://docs.rs/crate/notify/8.2.1/source/) | 8.2.1 | Yes | CC0-1.0 | Yes | Yes |
| [notify-types](https://docs.rs/crate/notify-types/2.2.0/source/) | 2.2.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [nu-ansi-term](https://docs.rs/crate/nu-ansi-term/0.50.3/source/) | 0.50.3 |  | MIT | Yes | Yes |
| [num_enum](https://docs.rs/crate/num_enum/0.7.6/source/) | 0.7.6 |  | BSD-3-Clause OR MIT OR Apache-2.0 | Yes | Yes |
| [num_enum_derive](https://docs.rs/crate/num_enum_derive/0.7.6/source/) | 0.7.6 |  | BSD-3-Clause OR MIT OR Apache-2.0 | Yes | Yes |
| [num-traits](https://docs.rs/crate/num-traits/0.2.19/source/) | 0.2.19 |  | MIT OR Apache-2.0 | Yes | Yes |
| [objc-sys](https://docs.rs/crate/objc-sys/0.3.5/source/) | 0.3.5 |  | MIT |  |  |
| [objc2](https://docs.rs/crate/objc2/0.5.3/source/) | 0.5.3 |  | MIT |  |  |
| [objc2](https://docs.rs/crate/objc2/0.6.5/source/) | 0.6.5 |  | MIT |  |  |
| [objc2-app-kit](https://docs.rs/crate/objc2-app-kit/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-app-kit](https://docs.rs/crate/objc2-app-kit/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-cloud-kit](https://docs.rs/crate/objc2-cloud-kit/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-contacts](https://docs.rs/crate/objc2-contacts/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-core-data](https://docs.rs/crate/objc2-core-data/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-core-foundation](https://docs.rs/crate/objc2-core-foundation/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-core-graphics](https://docs.rs/crate/objc2-core-graphics/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-core-image](https://docs.rs/crate/objc2-core-image/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-core-location](https://docs.rs/crate/objc2-core-location/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-core-text](https://docs.rs/crate/objc2-core-text/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-encode](https://docs.rs/crate/objc2-encode/4.1.0/source/) | 4.1.0 |  | MIT |  |  |
| [objc2-foundation](https://docs.rs/crate/objc2-foundation/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-foundation](https://docs.rs/crate/objc2-foundation/0.3.2/source/) | 0.3.2 |  | MIT |  |  |
| [objc2-io-surface](https://docs.rs/crate/objc2-io-surface/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-link-presentation](https://docs.rs/crate/objc2-link-presentation/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-metal](https://docs.rs/crate/objc2-metal/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-metal](https://docs.rs/crate/objc2-metal/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-quartz-core](https://docs.rs/crate/objc2-quartz-core/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-quartz-core](https://docs.rs/crate/objc2-quartz-core/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-symbols](https://docs.rs/crate/objc2-symbols/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-ui-kit](https://docs.rs/crate/objc2-ui-kit/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-ui-kit](https://docs.rs/crate/objc2-ui-kit/0.3.2/source/) | 0.3.2 |  | Zlib OR Apache-2.0 OR MIT |  |  |
| [objc2-uniform-type-identifiers](https://docs.rs/crate/objc2-uniform-type-identifiers/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [objc2-user-notifications](https://docs.rs/crate/objc2-user-notifications/0.2.2/source/) | 0.2.2 |  | MIT |  |  |
| [once_cell](https://docs.rs/crate/once_cell/1.21.4/source/) | 1.21.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [orbclient](https://docs.rs/crate/orbclient/0.3.55/source/) | 0.3.55 |  | MIT |  |  |
| [ordered-float](https://docs.rs/crate/ordered-float/5.5.1/source/) | 5.5.1 |  | MIT | Yes | Yes |
| [ordered-stream](https://docs.rs/crate/ordered-stream/0.2.0/source/) | 0.2.0 |  | MIT OR Apache-2.0 |  | Yes |
| [os_pipe](https://docs.rs/crate/os_pipe/1.2.3/source/) | 1.2.3 |  | MIT |  | Yes |
| [owned_ttf_parser](https://docs.rs/crate/owned_ttf_parser/0.25.1/source/) | 0.25.1 |  | Apache-2.0 |  | Yes |
| [parking](https://docs.rs/crate/parking/2.2.1/source/) | 2.2.1 |  | Apache-2.0 OR MIT |  | Yes |
| [parking_lot](https://docs.rs/crate/parking_lot/0.12.5/source/) | 0.12.5 |  | MIT OR Apache-2.0 | Yes | Yes |
| [parking_lot_core](https://docs.rs/crate/parking_lot_core/0.9.12/source/) | 0.9.12 |  | MIT OR Apache-2.0 | Yes | Yes |
| [parlance](https://docs.rs/crate/parlance/0.1.1/source/) | 0.1.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [parley](https://docs.rs/crate/parley/0.11.1/source/) | 0.11.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [parley_data](https://docs.rs/crate/parley_data/0.11.1/source/) | 0.11.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [percent-encoding](https://docs.rs/crate/percent-encoding/2.3.2/source/) | 2.3.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [petgraph](https://docs.rs/crate/petgraph/0.8.3/source/) | 0.8.3 |  | MIT OR Apache-2.0 |  | Yes |
| [pico-args](https://docs.rs/crate/pico-args/0.5.0/source/) | 0.5.0 |  | MIT | Yes | Yes |
| [pin-project](https://docs.rs/crate/pin-project/1.1.13/source/) | 1.1.13 |  | Apache-2.0 OR MIT | Yes | Yes |
| [pin-project-internal](https://docs.rs/crate/pin-project-internal/1.1.13/source/) | 1.1.13 |  | Apache-2.0 OR MIT | Yes | Yes |
| [pin-project-lite](https://docs.rs/crate/pin-project-lite/0.2.17/source/) | 0.2.17 |  | Apache-2.0 OR MIT | Yes | Yes |
| [pin-utils](https://docs.rs/crate/pin-utils/0.1.1/source/) | 0.1.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [pin-weak](https://docs.rs/crate/pin-weak/1.1.0/source/) | 1.1.0 |  | MIT | Yes | Yes |
| [piper](https://docs.rs/crate/piper/0.2.5/source/) | 0.2.5 |  | MIT OR Apache-2.0 |  | Yes |
| [pkg-config](https://docs.rs/crate/pkg-config/0.3.34/source/) | 0.3.34 |  | MIT OR Apache-2.0 | Yes | Yes |
| [plain](https://docs.rs/crate/plain/0.2.3/source/) | 0.2.3 |  | MIT/Apache-2.0 |  |  |
| [png](https://docs.rs/crate/png/0.18.1/source/) | 0.18.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [polling](https://docs.rs/crate/polling/3.11.0/source/) | 3.11.0 |  | Apache-2.0 OR MIT |  | Yes |
| [polycool](https://docs.rs/crate/polycool/0.4.0/source/) | 0.4.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [portable-atomic](https://docs.rs/crate/portable-atomic/1.15.0/source/) | 1.15.0 |  | Apache-2.0 OR MIT | Yes | Yes |
| [portable-atomic-util](https://docs.rs/crate/portable-atomic-util/0.2.8/source/) | 0.2.8 |  | Apache-2.0 OR MIT |  |  |
| [potential_utf](https://docs.rs/crate/potential_utf/0.1.6/source/) | 0.1.6 |  | Unicode-3.0 | Yes | Yes |
| [presser](https://docs.rs/crate/presser/0.3.1/source/) | 0.3.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [prettyplease](https://docs.rs/crate/prettyplease/0.2.37/source/) | 0.2.37 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [proc-macro-crate](https://docs.rs/crate/proc-macro-crate/3.5.0/source/) | 3.5.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [proc-macro2](https://docs.rs/crate/proc-macro2/1.0.107/source/) | 1.0.107 |  | MIT OR Apache-2.0 | Yes | Yes |
| [profiling](https://docs.rs/crate/profiling/1.0.18/source/) | 1.0.18 |  | MIT OR Apache-2.0 | Yes | Yes |
| [pulldown-cmark](https://docs.rs/crate/pulldown-cmark/0.13.6/source/) | 0.13.6 |  | MIT | Yes | Yes |
| [pxfm](https://docs.rs/crate/pxfm/0.1.30/source/) | 0.1.30 |  | BSD-3-Clause OR Apache-2.0 | Yes | Yes |
| [quick-error](https://docs.rs/crate/quick-error/2.0.1/source/) | 2.0.1 |  | MIT/Apache-2.0 | Yes | Yes |
| [quick-xml](https://docs.rs/crate/quick-xml/0.41.0/source/) | 0.41.0 |  | MIT |  | Yes |
| [quote](https://docs.rs/crate/quote/1.0.47/source/) | 1.0.47 |  | MIT OR Apache-2.0 | Yes | Yes |
| [r-efi](https://docs.rs/crate/r-efi/5.3.0/source/) | 5.3.0 |  | MIT OR Apache-2.0 OR LGPL-2.1-or-later |  |  |
| [r-efi](https://docs.rs/crate/r-efi/6.0.0/source/) | 6.0.0 |  | MIT OR Apache-2.0 OR LGPL-2.1-or-later |  |  |
| [range-alloc](https://docs.rs/crate/range-alloc/0.1.5/source/) | 0.1.5 |  | MIT OR Apache-2.0 | Yes |  |
| [raw-window-handle](https://docs.rs/crate/raw-window-handle/0.6.2/source/) | 0.6.2 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [raw-window-metal](https://docs.rs/crate/raw-window-metal/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 |  |  |
| [rayon](https://docs.rs/crate/rayon/1.12.0/source/) | 1.12.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [rayon-core](https://docs.rs/crate/rayon-core/1.13.0/source/) | 1.13.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [read-fonts](https://docs.rs/crate/read-fonts/0.41.0/source/) | 0.41.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [redox_syscall](https://docs.rs/crate/redox_syscall/0.4.1/source/) | 0.4.1 |  | MIT |  |  |
| [redox_syscall](https://docs.rs/crate/redox_syscall/0.5.18/source/) | 0.5.18 |  | MIT |  |  |
| [redox_syscall](https://docs.rs/crate/redox_syscall/0.9.5/source/) | 0.9.5 |  | MIT |  |  |
| [regex](https://docs.rs/crate/regex/1.13.1/source/) | 1.13.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [regex-automata](https://docs.rs/crate/regex-automata/0.4.18/source/) | 0.4.18 |  | MIT OR Apache-2.0 | Yes | Yes |
| [regex-syntax](https://docs.rs/crate/regex-syntax/0.8.11/source/) | 0.8.11 |  | MIT OR Apache-2.0 | Yes | Yes |
| [renderdoc-sys](https://docs.rs/crate/renderdoc-sys/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [resvg](https://docs.rs/crate/resvg/0.48.1/source/) | 0.48.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [rgb](https://docs.rs/crate/rgb/0.8.53/source/) | 0.8.53 |  | MIT | Yes | Yes |
| [rowan](https://docs.rs/crate/rowan/0.17.0/source/) | 0.17.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [roxmltree](https://docs.rs/crate/roxmltree/0.21.2/source/) | 0.21.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [rspolib](https://docs.rs/crate/rspolib/0.1.2/source/) | 0.1.2 |  | MIT | Yes | Yes |
| [rustc_version](https://docs.rs/crate/rustc_version/0.4.1/source/) | 0.4.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [rustc-hash](https://docs.rs/crate/rustc-hash/1.1.0/source/) | 1.1.0 |  | Apache-2.0/MIT | Yes | Yes |
| [rustc-hash](https://docs.rs/crate/rustc-hash/2.1.3/source/) | 2.1.3 |  | Apache-2.0 OR MIT | Yes | Yes |
| [rustix](https://docs.rs/crate/rustix/0.38.44/source/) | 0.38.44 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [rustix](https://docs.rs/crate/rustix/1.1.6/source/) | 1.1.6 | Yes | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  | Yes |
| [rustversion](https://docs.rs/crate/rustversion/1.0.23/source/) | 1.0.23 |  | MIT OR Apache-2.0 | Yes | Yes |
| [same-file](https://docs.rs/crate/same-file/1.0.6/source/) | 1.0.6 |  | Unlicense/MIT | Yes | Yes |
| [scoped-tls](https://docs.rs/crate/scoped-tls/1.0.1/source/) | 1.0.1 |  | MIT/Apache-2.0 |  | Yes |
| [scoped-tls-hkt](https://docs.rs/crate/scoped-tls-hkt/0.1.5/source/) | 0.1.5 |  | MIT/Apache-2.0 | Yes | Yes |
| [scopeguard](https://docs.rs/crate/scopeguard/1.2.0/source/) | 1.2.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [sctk-adwaita](https://docs.rs/crate/sctk-adwaita/0.10.1/source/) | 0.10.1 |  | MIT |  | Yes |
| [semver](https://docs.rs/crate/semver/1.0.28/source/) | 1.0.28 |  | MIT OR Apache-2.0 | Yes | Yes |
| [serde](https://docs.rs/crate/serde/1.0.229/source/) | 1.0.229 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [serde_core](https://docs.rs/crate/serde_core/1.0.229/source/) | 1.0.229 |  | MIT OR Apache-2.0 | Yes | Yes |
| [serde_derive](https://docs.rs/crate/serde_derive/1.0.229/source/) | 1.0.229 |  | MIT OR Apache-2.0 | Yes | Yes |
| [serde_json](https://docs.rs/crate/serde_json/1.0.154/source/) | 1.0.154 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [serde_repr](https://docs.rs/crate/serde_repr/0.1.21/source/) | 0.1.21 |  | MIT OR Apache-2.0 |  | Yes |
| [serde_spanned](https://docs.rs/crate/serde_spanned/1.1.2/source/) | 1.1.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [sharded-slab](https://docs.rs/crate/sharded-slab/0.1.7/source/) | 0.1.7 |  | MIT | Yes | Yes |
| [shlex](https://docs.rs/crate/shlex/1.3.0/source/) | 1.3.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [shlex](https://docs.rs/crate/shlex/2.0.1/source/) | 2.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [signal-hook-registry](https://docs.rs/crate/signal-hook-registry/1.4.8/source/) | 1.4.8 |  | MIT OR Apache-2.0 |  | Yes |
| [simd_cesu8](https://docs.rs/crate/simd_cesu8/1.2.0/source/) | 1.2.0 |  | Apache-2.0 OR MIT |  |  |
| [simd-adler32](https://docs.rs/crate/simd-adler32/0.3.10/source/) | 0.3.10 |  | MIT | Yes | Yes |
| [simdutf8](https://docs.rs/crate/simdutf8/0.1.5/source/) | 0.1.5 |  | MIT OR Apache-2.0 |  |  |
| [simplecss](https://docs.rs/crate/simplecss/0.2.2/source/) | 0.2.2 |  | Apache-2.0 OR MIT | Yes | Yes |
| [siphasher](https://docs.rs/crate/siphasher/1.0.4/source/) | 1.0.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [skia-bindings](https://docs.rs/crate/skia-bindings/0.153.3/source/) | 0.153.3 |  | MIT | Yes | Yes |
| [skia-safe](https://docs.rs/crate/skia-safe/0.153.3/source/) | 0.153.3 |  | MIT | Yes | Yes |
| [skrifa](https://docs.rs/crate/skrifa/0.44.0/source/) | 0.44.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [slab](https://docs.rs/crate/slab/0.4.12/source/) | 0.4.12 |  | MIT | Yes | Yes |
| [slint](https://docs.rs/crate/slint/1.18.1/source/) | 1.18.1 | Yes | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [slint-build](https://docs.rs/crate/slint-build/1.18.1/source/) | 1.18.1 | Yes | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [slint-macros](https://docs.rs/crate/slint-macros/1.18.1/source/) | 1.18.1 |  | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | Yes | Yes |
| [slotmap](https://docs.rs/crate/slotmap/1.1.1/source/) | 1.1.1 |  | Zlib | Yes | Yes |
| [smallvec](https://docs.rs/crate/smallvec/1.16.4/source/) | 1.16.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [smithay-client-toolkit](https://docs.rs/crate/smithay-client-toolkit/0.19.2/source/) | 0.19.2 |  | MIT |  | Yes |
| [smol_str](https://docs.rs/crate/smol_str/0.2.2/source/) | 0.2.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [smol_str](https://docs.rs/crate/smol_str/0.3.7/source/) | 0.3.7 |  | MIT OR Apache-2.0 | Yes | Yes |
| [snafu](https://docs.rs/crate/snafu/0.8.9/source/) | 0.8.9 |  | MIT OR Apache-2.0 | Yes | Yes |
| [snafu-derive](https://docs.rs/crate/snafu-derive/0.8.9/source/) | 0.8.9 |  | MIT OR Apache-2.0 | Yes | Yes |
| [softbuffer](https://docs.rs/crate/softbuffer/0.4.8/source/) | 0.4.8 |  | MIT OR Apache-2.0 | Yes | Yes |
| [spin_on](https://docs.rs/crate/spin_on/0.1.1/source/) | 0.1.1 | Yes | Apache-2.0 OR MIT | Yes | Yes |
| [spirv](https://docs.rs/crate/spirv/0.4.0+sdk-1.4.341.0/source/) | 0.4.0+sdk-1.4.341.0 |  | Apache-2.0 | Yes | Yes |
| [stable_deref_trait](https://docs.rs/crate/stable_deref_trait/1.2.1/source/) | 1.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [static_assertions](https://docs.rs/crate/static_assertions/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [strict-num](https://docs.rs/crate/strict-num/0.1.1/source/) | 0.1.1 |  | MIT | Yes | Yes |
| [strum](https://docs.rs/crate/strum/0.28.0/source/) | 0.28.0 |  | MIT | Yes | Yes |
| [strum_macros](https://docs.rs/crate/strum_macros/0.28.0/source/) | 0.28.0 |  | MIT | Yes | Yes |
| [svgtypes](https://docs.rs/crate/svgtypes/0.16.1/source/) | 0.16.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [swash](https://docs.rs/crate/swash/0.2.10/source/) | 0.2.10 |  | Apache-2.0 OR MIT | Yes | Yes |
| [syn](https://docs.rs/crate/syn/2.0.119/source/) | 2.0.119 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [syn](https://docs.rs/crate/syn/3.0.9/source/) | 3.0.9 |  | MIT OR Apache-2.0 | Yes | Yes |
| [synstructure](https://docs.rs/crate/synstructure/0.14.0/source/) | 0.14.0 |  | MIT | Yes | Yes |
| [sys-locale](https://docs.rs/crate/sys-locale/0.3.2/source/) | 0.3.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [taffy](https://docs.rs/crate/taffy/0.10.1/source/) | 0.10.1 |  | MIT | Yes | Yes |
| [tar](https://docs.rs/crate/tar/0.4.46/source/) | 0.4.46 |  | MIT OR Apache-2.0 | Yes | Yes |
| [tempfile](https://docs.rs/crate/tempfile/3.27.0/source/) | 3.27.0 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [termcolor](https://docs.rs/crate/termcolor/1.4.1/source/) | 1.4.1 |  | Unlicense OR MIT | Yes | Yes |
| [text-size](https://docs.rs/crate/text-size/1.1.1/source/) | 1.1.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [thiserror](https://docs.rs/crate/thiserror/1.0.69/source/) | 1.0.69 |  | MIT OR Apache-2.0 |  | Yes |
| [thiserror](https://docs.rs/crate/thiserror/2.0.21/source/) | 2.0.21 |  | MIT OR Apache-2.0 | Yes | Yes |
| [thiserror-impl](https://docs.rs/crate/thiserror-impl/1.0.69/source/) | 1.0.69 |  | MIT OR Apache-2.0 |  | Yes |
| [thiserror-impl](https://docs.rs/crate/thiserror-impl/2.0.21/source/) | 2.0.21 |  | MIT OR Apache-2.0 | Yes | Yes |
| [thread_local](https://docs.rs/crate/thread_local/1.1.10/source/) | 1.1.10 |  | MIT OR Apache-2.0 | Yes | Yes |
| [tiny-skia](https://docs.rs/crate/tiny-skia/0.11.4/source/) | 0.11.4 |  | BSD-3-Clause |  | Yes |
| [tiny-skia](https://docs.rs/crate/tiny-skia/0.12.0/source/) | 0.12.0 |  | BSD-3-Clause | Yes | Yes |
| [tiny-skia-path](https://docs.rs/crate/tiny-skia-path/0.11.4/source/) | 0.11.4 |  | BSD-3-Clause |  | Yes |
| [tiny-skia-path](https://docs.rs/crate/tiny-skia-path/0.12.0/source/) | 0.12.0 |  | BSD-3-Clause | Yes | Yes |
| [tiny-xlib](https://docs.rs/crate/tiny-xlib/0.2.5/source/) | 0.2.5 |  | MIT OR Apache-2.0 OR Zlib |  | Yes |
| [tinystr](https://docs.rs/crate/tinystr/0.8.5/source/) | 0.8.5 |  | Unicode-3.0 | Yes | Yes |
| [tinyvec](https://docs.rs/crate/tinyvec/1.13.3/source/) | 1.13.3 |  | Zlib OR Apache-2.0 OR MIT | Yes | Yes |
| [tokio](https://docs.rs/crate/tokio/1.54.1/source/) | 1.54.1 | Yes | MIT | Yes | Yes |
| [tokio-macros](https://docs.rs/crate/tokio-macros/2.7.2/source/) | 2.7.2 |  | MIT | Yes | Yes |
| [toml](https://docs.rs/crate/toml/1.2.0+spec-1.1.0/source/) | 1.2.0+spec-1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [toml_datetime](https://docs.rs/crate/toml_datetime/1.1.2+spec-1.1.0/source/) | 1.1.2+spec-1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [toml_edit](https://docs.rs/crate/toml_edit/0.25.18+spec-1.1.0/source/) | 0.25.18+spec-1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [toml_parser](https://docs.rs/crate/toml_parser/1.1.5+spec-1.1.0/source/) | 1.1.5+spec-1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [toml_writer](https://docs.rs/crate/toml_writer/1.2.0+spec-1.1.0/source/) | 1.2.0+spec-1.1.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [tracing](https://docs.rs/crate/tracing/0.1.44/source/) | 0.1.44 | Yes | MIT | Yes | Yes |
| [tracing-attributes](https://docs.rs/crate/tracing-attributes/0.1.31/source/) | 0.1.31 |  | MIT | Yes | Yes |
| [tracing-core](https://docs.rs/crate/tracing-core/0.1.36/source/) | 0.1.36 |  | MIT | Yes | Yes |
| [tracing-log](https://docs.rs/crate/tracing-log/0.2.0/source/) | 0.2.0 |  | MIT | Yes | Yes |
| [tracing-subscriber](https://docs.rs/crate/tracing-subscriber/0.3.23/source/) | 0.3.23 | Yes | MIT | Yes | Yes |
| [tree_magic_mini](https://docs.rs/crate/tree_magic_mini/3.2.2/source/) | 3.2.2 |  | MIT |  | Yes |
| [ttf-parser](https://docs.rs/crate/ttf-parser/0.25.1/source/) | 0.25.1 |  | MIT OR Apache-2.0 |  | Yes |
| [typed-index-collections](https://docs.rs/crate/typed-index-collections/3.5.0/source/) | 3.5.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [udev](https://docs.rs/crate/udev/0.9.3/source/) | 0.9.3 |  | MIT |  | Yes |
| [uds_windows](https://docs.rs/crate/uds_windows/1.2.1/source/) | 1.2.1 |  | MIT |  |  |
| [unicase](https://docs.rs/crate/unicase/2.10.0/source/) | 2.10.0 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [unicode-bidi](https://docs.rs/crate/unicode-bidi/0.3.18/source/) | 0.3.18 |  | MIT OR Apache-2.0 | Yes | Yes |
| [unicode-ident](https://docs.rs/crate/unicode-ident/1.0.27/source/) | 1.0.27 |  | (MIT OR Apache-2.0) AND Unicode-3.0 | Yes | Yes |
| [unicode-linebreak](https://docs.rs/crate/unicode-linebreak/0.1.5/source/) | 0.1.5 |  | Apache-2.0 | Yes | Yes |
| [unicode-script](https://docs.rs/crate/unicode-script/0.5.9/source/) | 0.5.9 |  | MIT OR Apache-2.0 | Yes | Yes |
| [unicode-segmentation](https://docs.rs/crate/unicode-segmentation/1.13.3/source/) | 1.13.3 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [unicode-vo](https://docs.rs/crate/unicode-vo/0.1.0/source/) | 0.1.0 |  | MIT/Apache-2.0 | Yes | Yes |
| [unicode-width](https://docs.rs/crate/unicode-width/0.2.3/source/) | 0.2.3 |  | MIT OR Apache-2.0 | Yes | Yes |
| [unicode-xid](https://docs.rs/crate/unicode-xid/0.2.6/source/) | 0.2.6 |  | MIT OR Apache-2.0 | Yes | Yes |
| [unty](https://docs.rs/crate/unty/0.0.4/source/) | 0.0.4 |  | MIT OR Apache-2.0 | Yes | Yes |
| [url](https://docs.rs/crate/url/2.5.8/source/) | 2.5.8 |  | MIT OR Apache-2.0 | Yes | Yes |
| [usvg](https://docs.rs/crate/usvg/0.48.1/source/) | 0.48.1 |  | Apache-2.0 OR MIT | Yes | Yes |
| [utf_types](https://docs.rs/crate/utf_types/1.0.0/source/) | 1.0.0 |  | Apache-2.0 OR MIT | Yes | Yes |
| [utf8_iter](https://docs.rs/crate/utf8_iter/1.1.0/source/) | 1.1.0 |  | Apache-2.0 OR MIT | Yes | Yes |
| [uuid](https://docs.rs/crate/uuid/1.29.0/source/) | 1.29.0 |  | Apache-2.0 OR MIT |  | Yes |
| [valuable](https://docs.rs/crate/valuable/0.1.1/source/) | 0.1.1 |  | MIT |  |  |
| [version_check](https://docs.rs/crate/version_check/0.9.5/source/) | 0.9.5 |  | MIT/Apache-2.0 | Yes | Yes |
| [vtable](https://docs.rs/crate/vtable/0.5.0/source/) | 0.5.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [vtable-macro](https://docs.rs/crate/vtable-macro/0.5.0/source/) | 0.5.0 |  | MIT OR Apache-2.0 | Yes | Yes |
| [walkdir](https://docs.rs/crate/walkdir/2.5.0/source/) | 2.5.0 |  | Unlicense/MIT | Yes | Yes |
| [wasi](https://docs.rs/crate/wasi/0.11.1+wasi-snapshot-preview1/source/) | 0.11.1+wasi-snapshot-preview1 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  |  |
| [wasip2](https://docs.rs/crate/wasip2/1.0.4+wasi-0.2.12/source/) | 1.0.4+wasi-0.2.12 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  |  |
| [wasm-bindgen](https://docs.rs/crate/wasm-bindgen/0.2.130/source/) | 0.2.130 |  | MIT OR Apache-2.0 |  |  |
| [wasm-bindgen-futures](https://docs.rs/crate/wasm-bindgen-futures/0.4.80/source/) | 0.4.80 |  | MIT OR Apache-2.0 |  |  |
| [wasm-bindgen-macro](https://docs.rs/crate/wasm-bindgen-macro/0.2.130/source/) | 0.2.130 |  | MIT OR Apache-2.0 |  |  |
| [wasm-bindgen-macro-support](https://docs.rs/crate/wasm-bindgen-macro-support/0.2.130/source/) | 0.2.130 |  | MIT OR Apache-2.0 |  |  |
| [wasm-bindgen-shared](https://docs.rs/crate/wasm-bindgen-shared/0.2.130/source/) | 0.2.130 |  | MIT OR Apache-2.0 |  |  |
| [wayland-backend](https://docs.rs/crate/wayland-backend/0.3.17/source/) | 0.3.17 |  | MIT |  | Yes |
| [wayland-client](https://docs.rs/crate/wayland-client/0.31.15/source/) | 0.31.15 |  | MIT |  | Yes |
| [wayland-csd-frame](https://docs.rs/crate/wayland-csd-frame/0.3.0/source/) | 0.3.0 |  | MIT |  | Yes |
| [wayland-cursor](https://docs.rs/crate/wayland-cursor/0.31.14/source/) | 0.31.14 |  | MIT |  | Yes |
| [wayland-protocols](https://docs.rs/crate/wayland-protocols/0.32.13/source/) | 0.32.13 |  | MIT |  | Yes |
| [wayland-protocols-plasma](https://docs.rs/crate/wayland-protocols-plasma/0.3.12/source/) | 0.3.12 |  | MIT |  | Yes |
| [wayland-protocols-wlr](https://docs.rs/crate/wayland-protocols-wlr/0.3.12/source/) | 0.3.12 |  | MIT |  | Yes |
| [wayland-scanner](https://docs.rs/crate/wayland-scanner/0.31.11/source/) | 0.31.11 |  | MIT |  | Yes |
| [wayland-sys](https://docs.rs/crate/wayland-sys/0.31.11/source/) | 0.31.11 |  | MIT |  | Yes |
| [web-sys](https://docs.rs/crate/web-sys/0.3.107/source/) | 0.3.107 |  | MIT OR Apache-2.0 |  |  |
| [web-time](https://docs.rs/crate/web-time/1.1.0/source/) | 1.1.0 |  | MIT OR Apache-2.0 |  |  |
| [webbrowser](https://docs.rs/crate/webbrowser/1.2.6/source/) | 1.2.6 |  | MIT OR Apache-2.0 | Yes | Yes |
| [weezl](https://docs.rs/crate/weezl/0.1.12/source/) | 0.1.12 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu](https://docs.rs/crate/wgpu/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu-core](https://docs.rs/crate/wgpu-core/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu-core-deps-apple](https://docs.rs/crate/wgpu-core-deps-apple/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 |  |  |
| [wgpu-core-deps-emscripten](https://docs.rs/crate/wgpu-core-deps-emscripten/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 |  |  |
| [wgpu-core-deps-wasm](https://docs.rs/crate/wgpu-core-deps-wasm/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 |  |  |
| [wgpu-core-deps-windows-linux-android](https://docs.rs/crate/wgpu-core-deps-windows-linux-android/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu-hal](https://docs.rs/crate/wgpu-hal/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu-naga-bridge](https://docs.rs/crate/wgpu-naga-bridge/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [wgpu-types](https://docs.rs/crate/wgpu-types/30.0.1/source/) | 30.0.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [winapi](https://docs.rs/crate/winapi/0.3.9/source/) | 0.3.9 |  | MIT/Apache-2.0 | Yes |  |
| [winapi-i686-pc-windows-gnu](https://docs.rs/crate/winapi-i686-pc-windows-gnu/0.4.0/source/) | 0.4.0 |  | MIT/Apache-2.0 |  |  |
| [winapi-util](https://docs.rs/crate/winapi-util/0.1.11/source/) | 0.1.11 |  | Unlicense OR MIT | Yes |  |
| [winapi-x86_64-pc-windows-gnu](https://docs.rs/crate/winapi-x86_64-pc-windows-gnu/0.4.0/source/) | 0.4.0 |  | MIT/Apache-2.0 |  |  |
| [windows](https://docs.rs/crate/windows/0.62.2/source/) | 0.62.2 | Yes | MIT OR Apache-2.0 | Yes | Yes |
| [windows_aarch64_gnullvm](https://docs.rs/crate/windows_aarch64_gnullvm/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_aarch64_gnullvm](https://docs.rs/crate/windows_aarch64_gnullvm/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_aarch64_msvc](https://docs.rs/crate/windows_aarch64_msvc/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_aarch64_msvc](https://docs.rs/crate/windows_aarch64_msvc/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_i686_gnu](https://docs.rs/crate/windows_i686_gnu/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_i686_gnu](https://docs.rs/crate/windows_i686_gnu/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_i686_gnullvm](https://docs.rs/crate/windows_i686_gnullvm/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_i686_msvc](https://docs.rs/crate/windows_i686_msvc/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_i686_msvc](https://docs.rs/crate/windows_i686_msvc/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_gnu](https://docs.rs/crate/windows_x86_64_gnu/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_gnu](https://docs.rs/crate/windows_x86_64_gnu/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_gnullvm](https://docs.rs/crate/windows_x86_64_gnullvm/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_gnullvm](https://docs.rs/crate/windows_x86_64_gnullvm/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_msvc](https://docs.rs/crate/windows_x86_64_msvc/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows_x86_64_msvc](https://docs.rs/crate/windows_x86_64_msvc/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 | Yes |  |
| [windows-collections](https://docs.rs/crate/windows-collections/0.3.2/source/) | 0.3.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-core](https://docs.rs/crate/windows-core/0.62.2/source/) | 0.62.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-future](https://docs.rs/crate/windows-future/0.3.2/source/) | 0.3.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-implement](https://docs.rs/crate/windows-implement/0.60.2/source/) | 0.60.2 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-interface](https://docs.rs/crate/windows-interface/0.59.3/source/) | 0.59.3 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-link](https://docs.rs/crate/windows-link/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-numerics](https://docs.rs/crate/windows-numerics/0.3.1/source/) | 0.3.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-result](https://docs.rs/crate/windows-result/0.4.1/source/) | 0.4.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-strings](https://docs.rs/crate/windows-strings/0.5.1/source/) | 0.5.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [windows-sys](https://docs.rs/crate/windows-sys/0.48.0/source/) | 0.48.0 |  | MIT OR Apache-2.0 |  |  |
| [windows-sys](https://docs.rs/crate/windows-sys/0.52.0/source/) | 0.52.0 |  | MIT OR Apache-2.0 | Yes |  |
| [windows-sys](https://docs.rs/crate/windows-sys/0.59.0/source/) | 0.59.0 |  | MIT OR Apache-2.0 |  |  |
| [windows-sys](https://docs.rs/crate/windows-sys/0.61.2/source/) | 0.61.2 | Yes | MIT OR Apache-2.0 | Yes |  |
| [windows-targets](https://docs.rs/crate/windows-targets/0.48.5/source/) | 0.48.5 |  | MIT OR Apache-2.0 |  |  |
| [windows-targets](https://docs.rs/crate/windows-targets/0.52.6/source/) | 0.52.6 |  | MIT OR Apache-2.0 | Yes |  |
| [windows-threading](https://docs.rs/crate/windows-threading/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 | Yes | Yes |
| [winit](https://docs.rs/crate/winit/0.30.13/source/) | 0.30.13 |  | Apache-2.0 | Yes | Yes |
| [winnow](https://docs.rs/crate/winnow/1.0.4/source/) | 1.0.4 |  | MIT | Yes | Yes |
| [wit-bindgen](https://docs.rs/crate/wit-bindgen/0.57.1/source/) | 0.57.1 |  | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |  |  |
| [wl-clipboard-rs](https://docs.rs/crate/wl-clipboard-rs/0.9.5/source/) | 0.9.5 |  | MIT OR Apache-2.0 |  | Yes |
| [write-fonts](https://docs.rs/crate/write-fonts/0.50.0/source/) | 0.50.0 |  | MIT OR Apache-2.0 |  |  |
| [writeable](https://docs.rs/crate/writeable/0.6.4/source/) | 0.6.4 |  | Unicode-3.0 | Yes | Yes |
| [x11-dl](https://docs.rs/crate/x11-dl/2.21.0/source/) | 2.21.0 |  | MIT |  | Yes |
| [x11rb](https://docs.rs/crate/x11rb/0.13.2/source/) | 0.13.2 |  | MIT OR Apache-2.0 |  | Yes |
| [x11rb-protocol](https://docs.rs/crate/x11rb-protocol/0.13.2/source/) | 0.13.2 |  | MIT OR Apache-2.0 |  | Yes |
| [xattr](https://docs.rs/crate/xattr/1.7.0/source/) | 1.7.0 |  | MIT OR Apache-2.0 |  | Yes |
| [xcursor](https://docs.rs/crate/xcursor/0.3.11/source/) | 0.3.11 |  | MIT |  | Yes |
| [xkbcommon](https://docs.rs/crate/xkbcommon/0.9.0/source/) | 0.9.0 |  | MIT |  | Yes |
| [xkbcommon-dl](https://docs.rs/crate/xkbcommon-dl/0.4.2/source/) | 0.4.2 |  | MIT |  | Yes |
| [xkeysym](https://docs.rs/crate/xkeysym/0.2.1/source/) | 0.2.1 |  | MIT OR Apache-2.0 OR Zlib |  | Yes |
| [xml-rs](https://docs.rs/crate/xml-rs/0.8.29/source/) | 0.8.29 |  | MIT | Yes | Yes |
| [xmlwriter](https://docs.rs/crate/xmlwriter/0.1.0/source/) | 0.1.0 |  | MIT | Yes | Yes |
| [yazi](https://docs.rs/crate/yazi/0.2.2/source/) | 0.2.2 |  | Apache-2.0 OR MIT | Yes | Yes |
| [yeslogic-fontconfig-sys](https://docs.rs/crate/yeslogic-fontconfig-sys/6.0.1/source/) | 6.0.1 |  | MIT |  | Yes |
| [yoke](https://docs.rs/crate/yoke/0.8.4/source/) | 0.8.4 |  | Unicode-3.0 | Yes | Yes |
| [yoke-derive](https://docs.rs/crate/yoke-derive/0.8.4/source/) | 0.8.4 |  | Unicode-3.0 | Yes | Yes |
| [zbus](https://docs.rs/crate/zbus/5.19.0/source/) | 5.19.0 |  | MIT |  | Yes |
| [zbus_macros](https://docs.rs/crate/zbus_macros/5.19.0/source/) | 5.19.0 |  | MIT |  | Yes |
| [zbus_names](https://docs.rs/crate/zbus_names/4.3.5/source/) | 4.3.5 |  | MIT |  | Yes |
| [zcheapstr](https://docs.rs/crate/zcheapstr/1.2.0/source/) | 1.2.0 |  | MIT |  | Yes |
| [zeno](https://docs.rs/crate/zeno/0.3.3/source/) | 0.3.3 |  | Apache-2.0 OR MIT | Yes | Yes |
| [zerocopy](https://docs.rs/crate/zerocopy/0.8.63/source/) | 0.8.63 |  | BSD-2-Clause OR Apache-2.0 OR MIT | Yes | Yes |
| [zerocopy-derive](https://docs.rs/crate/zerocopy-derive/0.8.63/source/) | 0.8.63 |  | BSD-2-Clause OR Apache-2.0 OR MIT | Yes | Yes |
| [zerofrom](https://docs.rs/crate/zerofrom/0.1.8/source/) | 0.1.8 |  | Unicode-3.0 | Yes | Yes |
| [zerofrom-derive](https://docs.rs/crate/zerofrom-derive/0.1.8/source/) | 0.1.8 |  | Unicode-3.0 | Yes | Yes |
| [zerotrie](https://docs.rs/crate/zerotrie/0.2.5/source/) | 0.2.5 |  | Unicode-3.0 | Yes | Yes |
| [zerovec](https://docs.rs/crate/zerovec/0.11.9/source/) | 0.11.9 |  | Unicode-3.0 | Yes | Yes |
| [zerovec-derive](https://docs.rs/crate/zerovec-derive/0.11.7/source/) | 0.11.7 |  | Unicode-3.0 | Yes | Yes |
| [zlib-rs](https://docs.rs/crate/zlib-rs/0.6.8/source/) | 0.6.8 |  | Zlib | Yes | Yes |
| [zmij](https://docs.rs/crate/zmij/1.0.23/source/) | 1.0.23 |  | MIT | Yes | Yes |
| [zune-core](https://docs.rs/crate/zune-core/0.5.3/source/) | 0.5.3 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [zune-jpeg](https://docs.rs/crate/zune-jpeg/0.5.15/source/) | 0.5.15 |  | MIT OR Apache-2.0 OR Zlib | Yes | Yes |
| [zvariant](https://docs.rs/crate/zvariant/5.15.1/source/) | 5.15.1 |  | MIT |  | Yes |
| [zvariant_derive](https://docs.rs/crate/zvariant_derive/5.15.1/source/) | 5.15.1 |  | MIT |  | Yes |
| [zvariant_utils](https://docs.rs/crate/zvariant_utils/4.2.0/source/) | 4.2.0 |  | MIT |  | Yes |
