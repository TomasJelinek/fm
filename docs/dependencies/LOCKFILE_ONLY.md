# Lock file entries outside the inventory

The [complete inventory](CARGO_LIBRARIES.md) lists the third-party packages that Cargo
selects for the captured targets of each inventory workspace. The lock files also pin
packages that no captured target selects: dependencies for other operating systems or
architectures, and optional dependencies whose features are not enabled. No Windows or
Linux build compiles them and no package ships them. This list names each of them, so
the two lists together account for every third-party package in the lock files of the
23 inventory workspaces. Workspace names are those of the complete inventory.

192 package/version records; 154 distinct crate names.

Licenses are the declarations of the pinned versions, read from the package archives in
the Cargo cache. No build downloads the packages marked †; their licenses are from the
crates.io record of the same version, whose checksum matches the lock file.

Two lock files are not inventory inputs. `vendor/image/Cargo.lock` is the upstream
image-rs lock file kept with the vendored source: the three image workspaces build that
source with their own lock files. The negative plugin-host test fixture's lock file
(`crates/midnight-plugin-host/tests/fixtures/bad-descriptor/Cargo.lock`) has no
third-party package.

Refresh this list with the inventory: after a lock file changes, every third-party
package in it must appear either in the complete inventory or here.

| Library | Version | Declared license | Workspaces | Not selected because |
| --- | --- | --- | --- | --- |
| [accesskit_ios](https://crates.io/crates/accesskit_ios/0.1.2) | 0.1.2 | MIT OR Apache-2.0 | app | used only on iOS |
| [accesskit_macos](https://crates.io/crates/accesskit_macos/0.26.3) | 0.26.3 | MIT OR Apache-2.0 | app | used only on macOS |
| [android-activity](https://crates.io/crates/android-activity/0.6.1) | 0.6.1 | MIT OR Apache-2.0 | app, renderer-tests | used only on Android |
| [android-build](https://crates.io/crates/android-build/0.1.4) | 0.1.4 | MIT | app | needed only by `i-slint-backend-android-activity` 1.18.1, not selected either |
| [android-properties](https://crates.io/crates/android-properties/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `android-activity` 0.6.1, not selected either |
| [android_system_properties](https://crates.io/crates/android_system_properties/0.1.5) | 0.1.5 | MIT/Apache-2.0 † | renderer-tests | optional in `wgpu-hal` 30.0.1, not enabled |
| [android_system_properties](https://crates.io/crates/android_system_properties/0.1.6) | 0.1.6 | MIT OR Apache-2.0 | app | used only on Android; optional in `wgpu-hal` 30.0.1, not enabled |
| [bincode](https://crates.io/crates/bincode/2.0.1) | 2.0.1 | MIT | app | optional in `typed-index-collections` 3.5.0, not enabled |
| [bindgen](https://crates.io/crates/bindgen/0.72.1) | 0.72.1 | BSD-3-Clause | app | needed only by `skia-bindings` 0.153.3, not selected either |
| [bitflags](https://crates.io/crates/bitflags/1.3.2) | 1.3.2 | MIT/Apache-2.0 | app, renderer-tests | needed only by `core-graphics` 0.23.2, `core-graphics-types` 0.1.3, 1 more, not selected either |
| [block2](https://crates.io/crates/block2/0.5.1) | 0.5.1 | MIT | app, renderer-tests | used only on macOS, iOS |
| [block2](https://crates.io/crates/block2/0.6.2) | 0.6.2 | MIT | app, renderer-tests | app: used only on iOS, macOS; optional in `wgpu-hal` 30.0.1, not enabled; renderer-tests: optional in `wgpu-hal` 30.0.1, not enabled |
| [borsh](https://crates.io/crates/borsh/1.8.1) | 1.8.1 | MIT OR Apache-2.0 | app | optional in `smol_str` 0.3.7, not enabled |
| [bumpalo](https://crates.io/crates/bumpalo/3.20.3) | 3.20.3 | MIT OR Apache-2.0 | app, archive, html-viewer, renderer-tests | app: needed only by `wasm-bindgen-macro-support` 0.2.130, not selected either; archive: needed only by `wasm-bindgen-macro-support` 0.2.130, not selected either; html-viewer: needed only by `wasm-bindgen-macro-support` 0.2.130, not selected either; renderer-tests: needed only by `wasm-bindgen-macro-support` 0.2.127, not selected either |
| [bytes](https://crates.io/crates/bytes/1.11.1) | 1.11.1 | MIT † | renderer-tests | needed only by `combine` 4.6.7, not selected either |
| [calloop](https://crates.io/crates/calloop/0.14.5) | 0.14.5 | MIT | app | needed only by `i-slint-backend-linuxkms` 1.18.1, not selected either |
| [cgl](https://crates.io/crates/cgl/0.3.2) | 0.3.2 | MIT / Apache-2.0 | app, renderer-tests | app: needed only by `glutin` 0.32.3, not selected either; renderer-tests: used only on macOS |
| [combine](https://crates.io/crates/combine/4.6.7) | 4.6.7 | MIT † | renderer-tests | needed only by `jni` 0.22.4, not selected either |
| [combine](https://crates.io/crates/combine/4.6.9) | 4.6.9 | MIT | app | needed only by `jni` 0.22.4, not selected either |
| [console_error_panic_hook](https://crates.io/crates/console_error_panic_hook/0.1.7) | 0.1.7 | Apache-2.0/MIT † | renderer-tests | used only on WebAssembly, WASI, Emscripten |
| [cookie](https://crates.io/crates/cookie/0.18.2) | 0.18.2 | MIT OR Apache-2.0 | html-viewer | needed only by `cookie_store` 0.22.1, not selected either |
| [cookie_store](https://crates.io/crates/cookie_store/0.22.1) | 0.22.1 | MIT OR Apache-2.0 | html-viewer | optional in `ureq` 3.5.1, not enabled |
| [core-foundation](https://crates.io/crates/core-foundation/0.9.4) | 0.9.4 | MIT OR Apache-2.0 | app, renderer-tests | used only on macOS, iOS |
| [core-foundation-sys](https://crates.io/crates/core-foundation-sys/0.8.7) | 0.8.7 | MIT OR Apache-2.0 | app, renderer-tests | app: used only on macOS, iOS; renderer-tests: needed only by `core-foundation` 0.9.4, not selected either |
| [core-graphics](https://crates.io/crates/core-graphics/0.23.2) | 0.23.2 | MIT OR Apache-2.0 | app, renderer-tests | used only on macOS |
| [core-graphics-types](https://crates.io/crates/core-graphics-types/0.1.3) | 0.1.3 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `core-graphics` 0.23.2, not selected either |
| [crossbeam-utils](https://crates.io/crates/crossbeam-utils/0.8.21) | 0.8.21 | MIT OR Apache-2.0 | renderer-tests | needed only by `concurrent-queue` 2.5.0, not selected either |
| [crunchy](https://crates.io/crates/crunchy/0.2.4) | 0.2.4 | MIT | app, image-decoder, image-viewer, renderer-tests, thumbnail-image | used only on SPIR-V |
| [deranged](https://crates.io/crates/deranged/0.5.9) | 0.5.9 | MIT OR Apache-2.0 | html-viewer | needed only by `time` 0.3.55, not selected either |
| [dispatch](https://crates.io/crates/dispatch/0.2.0) | 0.2.0 | MIT | app, renderer-tests | needed only by `objc2-foundation` 0.2.2, not selected either |
| [dispatch2](https://crates.io/crates/dispatch2/0.3.1) | 0.3.1 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | app: needed only by `glutin` 0.32.3, `objc2-core-foundation` 0.3.2, 1 more, not selected either; renderer-tests: used only on macOS |
| [drm](https://crates.io/crates/drm/0.14.1) | 0.14.1 | MIT | app | optional in `softbuffer` 0.4.8, not enabled |
| [drm](https://crates.io/crates/drm/0.15.0) | 0.15.0 | MIT | app | optional in `wgpu-hal` 30.0.1, not enabled |
| [drm-ffi](https://crates.io/crates/drm-ffi/0.9.1) | 0.9.1 | MIT | app | needed only by `drm` 0.14.1, `drm` 0.15.0, not selected either |
| [drm-fourcc](https://crates.io/crates/drm-fourcc/2.2.0) | 2.2.0 | MIT | app | needed only by `drm` 0.14.1, `drm` 0.15.0, not selected either |
| [drm-sys](https://crates.io/crates/drm-sys/0.8.1) | 0.8.1 | MIT | app | needed only by `drm-ffi` 0.9.1, not selected either |
| [dtor](https://crates.io/crates/dtor/0.8.1) | 0.8.1 | Apache-2.0 OR MIT | app | optional in `ctor` 0.10.1, not enabled |
| [foreign-types](https://crates.io/crates/foreign-types/0.5.0) | 0.5.0 | MIT/Apache-2.0 | app, renderer-tests | needed only by `core-graphics` 0.23.2, not selected either |
| [foreign-types-macros](https://crates.io/crates/foreign-types-macros/0.2.3) | 0.2.3 | MIT/Apache-2.0 † | renderer-tests | needed only by `foreign-types` 0.5.0, not selected either |
| [foreign-types-macros](https://crates.io/crates/foreign-types-macros/0.2.4) | 0.2.4 | MIT/Apache-2.0 | app | needed only by `foreign-types` 0.5.0, not selected either |
| [foreign-types-shared](https://crates.io/crates/foreign-types-shared/0.3.1) | 0.3.1 | MIT/Apache-2.0 | app, renderer-tests | needed only by `foreign-types` 0.5.0, not selected either |
| [futures-core](https://crates.io/crates/futures-core/0.3.32) | 0.3.32 | MIT OR Apache-2.0 † | renderer-tests | needed only by `futures-util` 0.3.32, not selected either |
| [futures-task](https://crates.io/crates/futures-task/0.3.32) | 0.3.32 | MIT OR Apache-2.0 † | renderer-tests | needed only by `futures-util` 0.3.32, not selected either |
| [futures-util](https://crates.io/crates/futures-util/0.3.32) | 0.3.32 | MIT OR Apache-2.0 † | renderer-tests | needed only by `js-sys` 0.3.104, not selected either |
| [glifo](https://crates.io/crates/glifo/0.4.0) | 0.4.0 | Apache-2.0 OR MIT | pdf-viewer, thumbnail-image | optional in `vello_cpu` 0.3.0, not enabled |
| [grid](https://crates.io/crates/grid/1.0.1) | 1.0.1 | MIT | app | optional in `taffy` 0.10.1, not enabled |
| [hermit-abi](https://crates.io/crates/hermit-abi/0.3.9) | 0.3.9 | MIT OR Apache-2.0 | app | needed only by `io-lifetimes` 1.0.11, not selected either |
| [hermit-abi](https://crates.io/crates/hermit-abi/0.5.2) | 0.5.2 | MIT OR Apache-2.0 † | renderer-tests | used only on Hermit |
| [hermit-abi](https://crates.io/crates/hermit-abi/0.5.3) | 0.5.3 | MIT OR Apache-2.0 | app, thumbnail-video | used only on Hermit |
| [i-slint-backend-android-activity](https://crates.io/crates/i-slint-backend-android-activity/1.18.1) | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | app | optional in `slint` 1.18.1, not enabled |
| [i-slint-backend-linuxkms](https://crates.io/crates/i-slint-backend-linuxkms/1.18.1) | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | app | optional in `i-slint-backend-selector` 1.18.1, `slint` 1.18.1, not enabled |
| [i-slint-backend-testing](https://crates.io/crates/i-slint-backend-testing/1.18.1) | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | app | optional in `i-slint-backend-selector` 1.18.1, not enabled |
| [i-slint-renderer-skia](https://crates.io/crates/i-slint-renderer-skia/1.18.1) | 1.18.1 | GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0 | app | used only on iOS; optional in `i-slint-backend-selector` 1.18.1, `i-slint-backend-winit` 1.18.1, not enabled |
| [iana-time-zone-haiku](https://crates.io/crates/iana-time-zone-haiku/0.1.2) | 0.1.2 | MIT OR Apache-2.0 | app | used only on Haiku |
| [icu_plurals](https://crates.io/crates/icu_plurals/2.4.0) | 2.4.0 | Unicode-3.0 | app | optional in `icu_decimal` 2.4.0, not enabled |
| [icu_plurals_data](https://crates.io/crates/icu_plurals_data/2.4.0) | 2.4.0 | Unicode-3.0 | app | needed only by `icu_plurals` 2.4.0, not selected either |
| [input](https://crates.io/crates/input/0.10.0) | 0.10.0 | MIT | app | needed only by `i-slint-backend-linuxkms` 1.18.1, not selected either |
| [input-sys](https://crates.io/crates/input-sys/1.19.0) | 1.19.0 | MIT | app | needed only by `input` 0.10.0, not selected either |
| [io-lifetimes](https://crates.io/crates/io-lifetimes/1.0.11) | 1.0.11 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | app | needed only by `udev` 0.9.3, not selected either |
| [itertools](https://crates.io/crates/itertools/0.13.0) | 0.13.0 | MIT OR Apache-2.0 | app | needed only by `bindgen` 0.72.1, not selected either |
| [jni](https://crates.io/crates/jni/0.22.4) | 0.22.4 | MIT OR Apache-2.0 | app, renderer-tests | app: used only on Android; renderer-tests: needed only by `android-activity` 0.6.1, not selected either |
| [jni-macros](https://crates.io/crates/jni-macros/0.22.4) | 0.22.4 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `jni` 0.22.4, not selected either |
| [jni-sys](https://crates.io/crates/jni-sys/0.3.1) | 0.3.1 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `ndk` 0.9.0, `ndk-sys` 0.6.0+11769913, not selected either |
| [jni-sys](https://crates.io/crates/jni-sys/0.4.1) | 0.4.1 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `jni` 0.22.4, `jni-sys` 0.3.1, not selected either |
| [jni-sys-macros](https://crates.io/crates/jni-sys-macros/0.4.1) | 0.4.1 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `jni-sys` 0.4.1, not selected either |
| [jobserver](https://crates.io/crates/jobserver/0.1.34) | 0.1.34 | MIT OR Apache-2.0 | renderer-tests | optional in `cc` 1.2.63, not enabled |
| [jobserver](https://crates.io/crates/jobserver/0.1.35) | 0.1.35 | MIT OR Apache-2.0 | app | optional in `cc` 1.11.0, not enabled |
| [js-sys](https://crates.io/crates/js-sys/0.3.104) | 0.3.104 | MIT OR Apache-2.0 † | renderer-tests | used only on WebAssembly, WASI, Emscripten; optional in `getrandom` 0.4.3, `instant` 0.1.13, 3 more, not enabled |
| [js-sys](https://crates.io/crates/js-sys/0.3.107) | 0.3.107 | MIT OR Apache-2.0 | app, archive, html-viewer | app: used only on WebAssembly, WASI, Emscripten; optional in `chrono` 0.4.45, `sys-locale` 0.3.2, 4 more, not enabled; archive: used only on WebAssembly, WASI, Emscripten; optional in `getrandom` 0.4.3, not enabled; html-viewer: needed only by `web-time` 1.1.0, not selected either |
| [kqueue](https://crates.io/crates/kqueue/1.2.1) | 1.2.1 | MIT | app | used only on iOS, BSD; optional in `notify` 8.2.1, not enabled |
| [kqueue-sys](https://crates.io/crates/kqueue-sys/1.1.2) | 1.1.2 | MIT | app | needed only by `kqueue` 1.2.1, not selected either |
| [libloading](https://crates.io/crates/libloading/0.9.0) | 0.9.0 | ISC | html-viewer | used only on macOS |
| [libredox](https://crates.io/crates/libredox/0.1.17) | 0.1.17 | MIT † | renderer-tests | needed only by `orbclient` 0.3.55, not selected either |
| [libredox](https://crates.io/crates/libredox/0.1.25) | 0.1.25 | MIT | app | needed only by `orbclient` 0.3.55, not selected either |
| [libseat](https://crates.io/crates/libseat/0.2.4) | 0.2.4 | MIT | app | needed only by `i-slint-backend-linuxkms` 1.18.1, not selected either |
| [libseat-sys](https://crates.io/crates/libseat-sys/0.2.0) | 0.2.0 | MIT | app | needed only by `libseat` 0.2.4, not selected either |
| [libudev-sys](https://crates.io/crates/libudev-sys/0.1.4) | 0.1.4 | MIT | app | needed only by `udev` 0.9.3, not selected either |
| [linux-raw-sys](https://crates.io/crates/linux-raw-sys/0.9.4) | 0.9.4 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | app | needed only by `drm-sys` 0.8.1, not selected either |
| [lru](https://crates.io/crates/lru/0.18.3) | 0.18.3 | MIT | renderer-tests | optional in `femtovg` 0.27.0, not enabled |
| [ndk](https://crates.io/crates/ndk/0.9.0) | 0.9.0 | MIT OR Apache-2.0 | app, renderer-tests | used only on Android |
| [ndk-context](https://crates.io/crates/ndk-context/0.1.1) | 0.1.1 | MIT OR Apache-2.0 | app, renderer-tests | app: used only on Android; renderer-tests: needed only by `android-activity` 0.6.1, not selected either |
| [ndk-sys](https://crates.io/crates/ndk-sys/0.6.0%2B11769913) | 0.6.0+11769913 | MIT OR Apache-2.0 | app, renderer-tests | optional in `wgpu-hal` 30.0.1, not enabled |
| [nix](https://crates.io/crates/nix/0.31.3) | 0.31.3 | MIT | app | needed only by `i-slint-backend-linuxkms` 1.18.1, not selected either |
| [num-conv](https://crates.io/crates/num-conv/0.2.2) | 0.2.2 | MIT OR Apache-2.0 | html-viewer | needed only by `time` 0.3.55, `time-macros` 0.2.32, not selected either |
| [objc-sys](https://crates.io/crates/objc-sys/0.3.5) | 0.3.5 | MIT | app, renderer-tests | app: needed only by `objc2` 0.5.3, not selected either; renderer-tests: needed only by `objc2` 0.5.2, not selected either |
| [objc2](https://crates.io/crates/objc2/0.5.2) | 0.5.2 | MIT † | renderer-tests | used only on macOS, iOS |
| [objc2](https://crates.io/crates/objc2/0.5.3) | 0.5.3 | MIT | app | used only on macOS, iOS |
| [objc2](https://crates.io/crates/objc2/0.6.4) | 0.6.4 | MIT † | renderer-tests | used only on macOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled |
| [objc2](https://crates.io/crates/objc2/0.6.5) | 0.6.5 | MIT | app, html-viewer | app: used only on macOS, iOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled; html-viewer: used only on macOS |
| [objc2-app-kit](https://crates.io/crates/objc2-app-kit/0.2.2) | 0.2.2 | MIT | app, renderer-tests | used only on macOS |
| [objc2-app-kit](https://crates.io/crates/objc2-app-kit/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | used only on macOS |
| [objc2-cloud-kit](https://crates.io/crates/objc2-cloud-kit/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-contacts](https://crates.io/crates/objc2-contacts/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-core-location` 0.2.2, not selected either |
| [objc2-core-data](https://crates.io/crates/objc2-core-data/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-app-kit` 0.2.2, `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-core-foundation](https://crates.io/crates/objc2-core-foundation/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | app: used only on macOS, iOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled; renderer-tests: used only on macOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled |
| [objc2-core-graphics](https://crates.io/crates/objc2-core-graphics/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | app: used only on macOS, iOS; optional in `wgpu-hal` 30.0.1, not enabled; renderer-tests: optional in `wgpu-hal` 30.0.1, not enabled |
| [objc2-core-image](https://crates.io/crates/objc2-core-image/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-app-kit` 0.2.2, `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-core-location](https://crates.io/crates/objc2-core-location/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-cloud-kit` 0.2.2, `objc2-ui-kit` 0.2.2, 1 more, not selected either |
| [objc2-core-text](https://crates.io/crates/objc2-core-text/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | optional in `fontique` 0.11.1, not enabled |
| [objc2-encode](https://crates.io/crates/objc2-encode/4.1.0) | 4.1.0 | MIT | app, html-viewer, renderer-tests | app: needed only by `objc2` 0.5.3, `objc2` 0.6.5, not selected either; html-viewer: needed only by `objc2` 0.6.5, not selected either; renderer-tests: needed only by `objc2` 0.5.2, `objc2` 0.6.4, not selected either |
| [objc2-foundation](https://crates.io/crates/objc2-foundation/0.2.2) | 0.2.2 | MIT | app, renderer-tests | used only on iOS, macOS |
| [objc2-foundation](https://crates.io/crates/objc2-foundation/0.3.2) | 0.3.2 | MIT | app, renderer-tests | app: used only on macOS, iOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled; renderer-tests: used only on macOS; optional in `fontique` 0.11.1, `wgpu-hal` 30.0.1, not enabled |
| [objc2-io-surface](https://crates.io/crates/objc2-io-surface/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | needed only by `objc2-core-graphics` 0.3.2, not selected either |
| [objc2-link-presentation](https://crates.io/crates/objc2-link-presentation/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-metal](https://crates.io/crates/objc2-metal/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-core-image` 0.2.2, `objc2-quartz-core` 0.2.2, not selected either |
| [objc2-metal](https://crates.io/crates/objc2-metal/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | optional in `wgpu-hal` 30.0.1, not enabled |
| [objc2-quartz-core](https://crates.io/crates/objc2-quartz-core/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-app-kit` 0.2.2, `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-quartz-core](https://crates.io/crates/objc2-quartz-core/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app, renderer-tests | app: used only on iOS, macOS; optional in `wgpu-hal` 30.0.1, not enabled; renderer-tests: optional in `wgpu-hal` 30.0.1, not enabled |
| [objc2-symbols](https://crates.io/crates/objc2-symbols/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-ui-kit](https://crates.io/crates/objc2-ui-kit/0.2.2) | 0.2.2 | MIT | app, renderer-tests | used only on iOS |
| [objc2-ui-kit](https://crates.io/crates/objc2-ui-kit/0.3.2) | 0.3.2 | Zlib OR Apache-2.0 OR MIT | app | used only on iOS |
| [objc2-uniform-type-identifiers](https://crates.io/crates/objc2-uniform-type-identifiers/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-ui-kit` 0.2.2, not selected either |
| [objc2-user-notifications](https://crates.io/crates/objc2-user-notifications/0.2.2) | 0.2.2 | MIT | app, renderer-tests | needed only by `objc2-ui-kit` 0.2.2, not selected either |
| [orbclient](https://crates.io/crates/orbclient/0.3.55) | 0.3.55 | MIT | app, renderer-tests | used only on Redox |
| [plain](https://crates.io/crates/plain/0.2.3) | 0.2.3 | MIT/Apache-2.0 | app, renderer-tests | app: needed only by `libredox` 0.1.25, not selected either; renderer-tests: needed only by `libredox` 0.1.17, not selected either |
| [portable-atomic](https://crates.io/crates/portable-atomic/1.13.1) | 1.13.1 | Apache-2.0 OR MIT † | renderer-tests | used only on targets without 64-bit atomics; optional in `wgpu-core` 30.0.1, `wgpu-hal` 30.0.1, not enabled |
| [portable-atomic-util](https://crates.io/crates/portable-atomic-util/0.2.7) | 0.2.7 | Apache-2.0 OR MIT † | renderer-tests | optional in `wgpu-hal` 30.0.1, not enabled |
| [portable-atomic-util](https://crates.io/crates/portable-atomic-util/0.2.8) | 0.2.8 | Apache-2.0 OR MIT | app | optional in `wgpu-hal` 30.0.1, not enabled |
| [powerfmt](https://crates.io/crates/powerfmt/0.2.1) | 0.2.1 | MIT OR Apache-2.0 | html-viewer | needed only by `time` 0.3.55, not selected either |
| [prettyplease](https://crates.io/crates/prettyplease/0.2.37) | 0.2.37 | MIT OR Apache-2.0 | app | needed only by `bindgen` 0.72.1, not selected either |
| [r-efi](https://crates.io/crates/r-efi/5.3.0) | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | app, renderer-tests | used only on targets where `cfg(all(target_os = "uefi", getrandom_backend = "efi_rng"))` holds |
| [r-efi](https://crates.io/crates/r-efi/6.0.0) | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | api, app, archive, renderer-tests | used only on targets where `cfg(all(target_os = "uefi", getrandom_backend = "efi_rng"))` holds |
| [raw-window-metal](https://crates.io/crates/raw-window-metal/1.1.0) | 1.1.0 | MIT OR Apache-2.0 | app, renderer-tests | optional in `wgpu-hal` 30.0.1, not enabled |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.4.1) | 0.4.1 | MIT | app, renderer-tests | used only on Redox |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.5.18) | 0.5.18 | MIT | app, renderer-tests | used only on Redox |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.8.0) | 0.8.0 | MIT † | renderer-tests | needed only by `libredox` 0.1.17, not selected either |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.9.5) | 0.9.5 | MIT | app | needed only by `libredox` 0.1.25, not selected either |
| [rustversion](https://crates.io/crates/rustversion/1.0.22) | 1.0.22 | MIT OR Apache-2.0 † | renderer-tests | needed only by `num_enum` 0.7.6, `wasm-bindgen` 0.2.127, not selected either |
| [rustybuzz](https://crates.io/crates/rustybuzz/0.20.1) | 0.20.1 | MIT | renderer-tests | optional in `femtovg` 0.27.0, not enabled |
| [serde](https://crates.io/crates/serde/1.0.228) | 1.0.228 | MIT OR Apache-2.0 | renderer-tests | declared under `cfg(any())`, which never holds; optional in `codespan-reporting` 0.13.1, `femtovg` 0.27.0, 4 more, not enabled |
| [serde_derive](https://crates.io/crates/serde_derive/1.0.228) | 1.0.228 | MIT OR Apache-2.0 | renderer-tests | declared under `cfg(any())`, which never holds |
| [shlex](https://crates.io/crates/shlex/1.3.0) | 1.3.0 | MIT OR Apache-2.0 | app | needed only by `bindgen` 0.72.1, not selected either |
| [simd_cesu8](https://crates.io/crates/simd_cesu8/1.2.0) | 1.2.0 | Apache-2.0 OR MIT | app, renderer-tests | needed only by `jni` 0.22.4, `jni-macros` 0.22.4, not selected either |
| [simdutf8](https://crates.io/crates/simdutf8/0.1.5) | 0.1.5 | MIT OR Apache-2.0 | app, renderer-tests | needed only by `simd_cesu8` 1.2.0, not selected either |
| [skia-bindings](https://crates.io/crates/skia-bindings/0.153.3) | 0.153.3 | MIT | app | needed only by `skia-safe` 0.153.3, not selected either |
| [skia-safe](https://crates.io/crates/skia-safe/0.153.3) | 0.153.3 | MIT | app | needed only by `i-slint-renderer-skia` 1.18.1, not selected either |
| [termcolor](https://crates.io/crates/termcolor/1.4.1) | 1.4.1 | Unlicense OR MIT | app, renderer-tests | optional in `codespan-reporting` 0.13.1, not enabled |
| [time](https://crates.io/crates/time/0.3.55) | 0.3.55 | MIT OR Apache-2.0 | html-viewer | needed only by `cookie` 0.18.2, `cookie_store` 0.22.1, not selected either |
| [time-core](https://crates.io/crates/time-core/0.1.9) | 0.1.9 | MIT OR Apache-2.0 | html-viewer | needed only by `time` 0.3.55, `time-macros` 0.2.32, not selected either |
| [time-macros](https://crates.io/crates/time-macros/0.2.32) | 0.2.32 | MIT OR Apache-2.0 | html-viewer | needed only by `time` 0.3.55, not selected either |
| [toml](https://crates.io/crates/toml/1.2.0%2Bspec-1.1.0) | 1.2.0+spec-1.1.0 | MIT OR Apache-2.0 | app | needed only by `skia-bindings` 0.153.3, not selected either |
| [toml_datetime](https://crates.io/crates/toml_datetime/1.1.1%2Bspec-1.1.0) | 1.1.1+spec-1.1.0 | MIT OR Apache-2.0 † | renderer-tests | needed only by `toml_edit` 0.25.12+spec-1.1.0, not selected either |
| [toml_edit](https://crates.io/crates/toml_edit/0.25.12%2Bspec-1.1.0) | 0.25.12+spec-1.1.0 | MIT OR Apache-2.0 † | renderer-tests | needed only by `proc-macro-crate` 3.5.0, not selected either |
| [toml_parser](https://crates.io/crates/toml_parser/1.1.2%2Bspec-1.1.0) | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 † | renderer-tests | needed only by `toml_edit` 0.25.12+spec-1.1.0, not selected either |
| [udev](https://crates.io/crates/udev/0.9.3) | 0.9.3 | MIT | app | needed only by `input` 0.10.0, not selected either |
| [uds_windows](https://crates.io/crates/uds_windows/1.2.1) | 1.2.1 | MIT | app | used only on 32-bit x86 Windows, Windows on Arm64, Windows GNU toolchains |
| [unicode-bidi-mirroring](https://crates.io/crates/unicode-bidi-mirroring/0.4.0) | 0.4.0 | MIT/Apache-2.0 | renderer-tests | needed only by `rustybuzz` 0.20.1, not selected either |
| [unicode-ccc](https://crates.io/crates/unicode-ccc/0.4.0) | 0.4.0 | MIT/Apache-2.0 | renderer-tests | needed only by `rustybuzz` 0.20.1, not selected either |
| [unicode-properties](https://crates.io/crates/unicode-properties/0.1.4) | 0.1.4 | MIT/Apache-2.0 | renderer-tests | needed only by `rustybuzz` 0.20.1, not selected either |
| [unty](https://crates.io/crates/unty/0.0.4) | 0.0.4 | MIT OR Apache-2.0 | app | needed only by `bincode` 2.0.1, not selected either |
| [valuable](https://crates.io/crates/valuable/0.1.1) | 0.1.1 | MIT | app | optional in `tracing-core` 0.1.36, not enabled |
| [wasi](https://crates.io/crates/wasi/0.11.1%2Bwasi-snapshot-preview1) | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | app, html-viewer | used only on WASI |
| [wasip2](https://crates.io/crates/wasip2/1.0.3%2Bwasi-0.2.9) | 1.0.3+wasi-0.2.9 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT † | renderer-tests | used only on WASI |
| [wasip2](https://crates.io/crates/wasip2/1.0.4%2Bwasi-0.2.12) | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | app | used only on WASI |
| [wasm-bindgen](https://crates.io/crates/wasm-bindgen/0.2.127) | 0.2.127 | MIT OR Apache-2.0 † | renderer-tests | used only on WebAssembly, WASI, Emscripten; optional in `getrandom` 0.4.3, `instant` 0.1.13, 2 more, not enabled |
| [wasm-bindgen](https://crates.io/crates/wasm-bindgen/0.2.130) | 0.2.130 | MIT OR Apache-2.0 | app, archive, html-viewer | app: used only on WebAssembly, WASI, Emscripten; optional in `chrono` 0.4.45, `sys-locale` 0.3.2, 3 more, not enabled; archive: used only on WebAssembly, WASI, Emscripten; optional in `getrandom` 0.4.3, not enabled; html-viewer: needed only by `js-sys` 0.3.107, `web-time` 1.1.0, not selected either |
| [wasm-bindgen-futures](https://crates.io/crates/wasm-bindgen-futures/0.4.77) | 0.4.77 | MIT OR Apache-2.0 † | renderer-tests | used only on WebAssembly, WASI, Emscripten; optional in `wgpu` 30.0.1, not enabled |
| [wasm-bindgen-futures](https://crates.io/crates/wasm-bindgen-futures/0.4.80) | 0.4.80 | MIT OR Apache-2.0 | app | used only on WebAssembly, WASI, Emscripten; optional in `wgpu` 30.0.1, not enabled |
| [wasm-bindgen-macro](https://crates.io/crates/wasm-bindgen-macro/0.2.127) | 0.2.127 | MIT OR Apache-2.0 † | renderer-tests | needed only by `wasm-bindgen` 0.2.127, not selected either |
| [wasm-bindgen-macro](https://crates.io/crates/wasm-bindgen-macro/0.2.130) | 0.2.130 | MIT OR Apache-2.0 | app, archive, html-viewer | needed only by `wasm-bindgen` 0.2.130, not selected either |
| [wasm-bindgen-macro-support](https://crates.io/crates/wasm-bindgen-macro-support/0.2.127) | 0.2.127 | MIT OR Apache-2.0 † | renderer-tests | needed only by `wasm-bindgen-macro` 0.2.127, not selected either |
| [wasm-bindgen-macro-support](https://crates.io/crates/wasm-bindgen-macro-support/0.2.130) | 0.2.130 | MIT OR Apache-2.0 | app, archive, html-viewer | needed only by `wasm-bindgen-macro` 0.2.130, not selected either |
| [wasm-bindgen-shared](https://crates.io/crates/wasm-bindgen-shared/0.2.127) | 0.2.127 | MIT OR Apache-2.0 † | renderer-tests | needed only by `wasm-bindgen` 0.2.127, `wasm-bindgen-macro-support` 0.2.127, not selected either |
| [wasm-bindgen-shared](https://crates.io/crates/wasm-bindgen-shared/0.2.130) | 0.2.130 | MIT OR Apache-2.0 | app, archive, html-viewer | needed only by `wasm-bindgen` 0.2.130, `wasm-bindgen-macro-support` 0.2.130, not selected either |
| [web-sys](https://crates.io/crates/web-sys/0.3.104) | 0.3.104 | MIT OR Apache-2.0 † | renderer-tests | used only on WebAssembly, WASI, Emscripten; optional in `instant` 0.1.13, `wgpu` 30.0.1, 2 more, not enabled |
| [web-sys](https://crates.io/crates/web-sys/0.3.107) | 0.3.107 | MIT OR Apache-2.0 | app | used only on WebAssembly, WASI, Emscripten; optional in `sys-locale` 0.3.2, `wgpu` 30.0.1, 2 more, not enabled |
| [web-time](https://crates.io/crates/web-time/1.1.0) | 1.1.0 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on WebAssembly, WASI, Emscripten; optional in `i-slint-core` 1.18.1, not enabled; html-viewer: optional in `indicatif` 0.18.6, not enabled; renderer-tests: used only on WebAssembly, WASI, Emscripten |
| [wgpu-core-deps-apple](https://crates.io/crates/wgpu-core-deps-apple/30.0.1) | 30.0.1 | MIT OR Apache-2.0 | app, renderer-tests | optional in `wgpu-core` 30.0.1, not enabled |
| [wgpu-core-deps-emscripten](https://crates.io/crates/wgpu-core-deps-emscripten/30.0.1) | 30.0.1 | MIT OR Apache-2.0 | app, renderer-tests | optional in `wgpu-core` 30.0.1, not enabled |
| [wgpu-core-deps-wasm](https://crates.io/crates/wgpu-core-deps-wasm/30.0.1) | 30.0.1 | MIT OR Apache-2.0 | app, renderer-tests | optional in `wgpu-core` 30.0.1, not enabled |
| [winapi-util](https://crates.io/crates/winapi-util/0.1.11) | 0.1.11 | Unlicense OR MIT | app, renderer-tests | app: used only on 32-bit x86 Windows, Windows on Arm64, Windows GNU toolchains; renderer-tests: needed only by `same-file` 1.0.6, `termcolor` 1.4.1, 1 more, not selected either |
| [windows-sys](https://crates.io/crates/windows-sys/0.48.0) | 0.48.0 | MIT OR Apache-2.0 | app | needed only by `io-lifetimes` 1.0.11, not selected either |
| [windows-sys](https://crates.io/crates/windows-sys/0.59.0) | 0.59.0 | MIT OR Apache-2.0 | app, renderer-tests | used only on 32-bit x86 Windows, Windows on Arm64, Windows GNU toolchains |
| [windows-targets](https://crates.io/crates/windows-targets/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-sys` 0.48.0, not selected either |
| [windows_aarch64_gnullvm](https://crates.io/crates/windows_aarch64_gnullvm/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_aarch64_gnullvm](https://crates.io/crates/windows_aarch64_gnullvm/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on Windows on Arm64; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on Windows on Arm64 |
| [windows_aarch64_msvc](https://crates.io/crates/windows_aarch64_msvc/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_aarch64_msvc](https://crates.io/crates/windows_aarch64_msvc/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on Windows on Arm64; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on Windows on Arm64 |
| [windows_i686_gnu](https://crates.io/crates/windows_i686_gnu/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_i686_gnu](https://crates.io/crates/windows_i686_gnu/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on 32-bit x86 Windows; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on 32-bit x86 Windows |
| [windows_i686_gnullvm](https://crates.io/crates/windows_i686_gnullvm/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on 32-bit x86 Windows; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on 32-bit x86 Windows |
| [windows_i686_msvc](https://crates.io/crates/windows_i686_msvc/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_i686_msvc](https://crates.io/crates/windows_i686_msvc/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on 32-bit x86 Windows; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on 32-bit x86 Windows |
| [windows_x86_64_gnu](https://crates.io/crates/windows_x86_64_gnu/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_x86_64_gnu](https://crates.io/crates/windows_x86_64_gnu/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on Windows GNU toolchains; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on Windows GNU toolchains |
| [windows_x86_64_gnullvm](https://crates.io/crates/windows_x86_64_gnullvm/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [windows_x86_64_gnullvm](https://crates.io/crates/windows_x86_64_gnullvm/0.52.6) | 0.52.6 | MIT OR Apache-2.0 | app, html-viewer, renderer-tests | app: used only on Windows GNU toolchains; html-viewer: needed only by `windows-targets` 0.52.6, not selected either; renderer-tests: used only on Windows GNU toolchains |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.48.5) | 0.48.5 | MIT OR Apache-2.0 | app | needed only by `windows-targets` 0.48.5, not selected either |
| [winnow](https://crates.io/crates/winnow/1.0.3) | 1.0.3 | MIT † | renderer-tests | needed only by `toml_edit` 0.25.12+spec-1.1.0, `toml_parser` 1.1.2+spec-1.1.0, not selected either |
| [wit-bindgen](https://crates.io/crates/wit-bindgen/0.57.1) | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | app, renderer-tests | app: needed only by `wasip2` 1.0.4+wasi-0.2.12, not selected either; renderer-tests: needed only by `wasip2` 1.0.3+wasi-0.2.9, not selected either |
| [write-fonts](https://crates.io/crates/write-fonts/0.50.0) | 0.50.0 | MIT OR Apache-2.0 | app | needed only by `i-slint-renderer-skia` 1.18.1, not selected either |
| [xkbcommon](https://crates.io/crates/xkbcommon/0.9.0) | 0.9.0 | MIT | app | needed only by `i-slint-backend-linuxkms` 1.18.1, not selected either |
