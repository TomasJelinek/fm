# Restricted image viewer

Independent process plugin for Midnight Explorer. PNG, JPEG, GIF, WebP, BMP, ICO
and TIFF use exactly the pinned image-rs and moxcms libraries used by thumbnails.
SVG uses the same shared restricted decoder with resvg 0.48.1 and embedded Noto
fonts. SVG supports shapes, paths, gradients, local reuse and text; it rejects
images, scripts, foreign objects and filter/mask/clip/marker/pattern effects.
The source limit is 4 MiB, XML limit 32768 nodes, depth 64 and local-reference
expansion 65536 visits. Embedded Noto Sans supplies Latin, Arabic and Hebrew
fallback; system-font appearance and full Unicode coverage are not promised.
Font provenance and OFL terms are retained in `notices/Noto-Sans/`.
AVIF uses that same shared decoder with statically linked dav1d 1.5.4. Its encoded
container is capped at 32 MiB; color and alpha each receive a 16,777,216-pixel
native limit before decoding, with one native thread. SVGZ and HEIC/HEIF remain
unsupported; AVIF animation, derived grids and HDR tone mapping are not claimed.
The first still frame is decoded once, applies supported orientation/color metadata, and is retained
inside the sandbox. Viewports sample the original pixels: 1:1 is exact,
magnification uses nearest original pixels, and reduction uses alpha-weighted area
filtering. No full zoomed-image allocation is created.
The last reduced viewport is cached (at most 16 MiB) across 1:1 inspection, so
returning to the same Fit view does not filter the original pixels again.

Only the released `midnight-image-viewer-sdk`/API and worker SDK are host
dependencies. The process enters its Windows AppContainer or Linux worker
restriction before reading content. The protocol contains no path, URL, native
handle, UI object or executable command. Decoding has a 32768-pixel edge and
128 MiB decoded-data budget; viewports have 4096-pixel edges and at most 4 Mi pixels.
Opaque source reads are at most 256 KiB. Decoder reads use a bounded 256 KiB
buffer to batch small codec row requests. The host enforces read volume, read
count, source revision, per-response deadlines and a 512 MiB/60 CPU-second process limit.
The CPU allowance is cumulative across a session; exhaustion ends that session.

Use a separately released offline SDK registry. The project never edits host
sources. All build/test output must be inside a directory named `_build`:

```powershell
./scripts/ci.ps1 -SdkConfig C:/sdk/.cargo/registry.toml -BuildRoot ./_build/check
./scripts/build.ps1 -SdkConfig C:/sdk/.cargo/registry.toml -BuildRoot ./_build/build
```

```sh
bash scripts/ci.sh /sdk/.cargo/registry.toml ./_build/check
bash scripts/build.sh /sdk/.cargo/registry.toml ./_build/build
```

Install the resulting package directory under the application's `viewers/` root.
The independent `image-viewer.json` manifest must not coexist with `plugin.toml`
in the same package. Installation/removal takes effect after application restart.
The host validates the entire package tree and ancestors before launching code.
Removing this package removes image support. Tests generate owned raster/SVG data
and use the explicitly licensed upstream AVIF fixtures recorded in the shared decoder.

Build/CI scripts prepare hash-pinned private Meson/Ninja/NASM tools and a static
dav1d library beneath the build root before Cargo. Initial native preparation may
download verified archives; Cargo remains locked/offline. Use `-NativeOffline`
on PowerShell or `MIDNIGHT_IMAGE_NATIVE_OFFLINE=1` on Bash to require cached native
archives as well. An existing native C compiler is required; no system packages
are silently installed. The package contains no extra decoder DLL/shared object.

The native package tooling is qualified for Windows x64 and Linux x86-64, matching
the application's current system requirements. ARM64 image-package build tooling
is deferred; accepting a target string in older scripts was not qualification.
