# Restricted image viewer

Independent process plugin for Midnight Explorer. PNG, JPEG, GIF, WebP, BMP, ICO
and TIFF use exactly the pinned image-rs and moxcms libraries used by thumbnails.
SVG uses the same shared restricted decoder with resvg 0.48.1 and embedded Noto
fonts. It draws shapes, paths, text, gradients, patterns, markers, clip paths, masks,
filters, local reuse and `data:` PNG/JPEG/GIF/WebP images; scripts never run,
`foreignObject` is skipped for its `switch` fallback, and linked files and URLs are
never opened. Malformed documents and documents over a bound are refused with the
exact reason and bound: 4 MiB of source, 262,144 XML nodes, depth 64, an estimated
256 MiB of converted tree after expanding references, and 448 MiB of memory and 2^33
pixel operations at the drawing size (the
[decoder contract](../image-decoder/README.md#svg) lists them all). The decoder's
note naming skipped content does not reach the status line until image viewer API
0.5.0 (VIEW-06). Embedded Noto Sans supplies Latin, Arabic and Hebrew fallback;
system-font appearance and full Unicode coverage are not promised.
Font provenance and OFL terms are retained in `notices/Noto-Sans/`.
AVIF uses that same shared decoder with statically linked dav1d 1.5.4. Its encoded
container is capped at 32 MiB; color and alpha each receive a 16,777,216-pixel
native limit before decoding, with one native thread. SVGZ and HEIC/HEIF remain
unsupported; AVIF animation, derived grids and HDR tone mapping are not claimed.
The first still frame is decoded once, applies supported orientation/color metadata, and is retained
inside the sandbox. Viewports sample the original pixels: 1:1 is exact,
magnification uses nearest original pixels, and reduction uses alpha-weighted area
filtering. No full zoomed-image allocation is created.
The last reduced viewport request is cached (at most 16 MiB) across 1:1 inspection,
so repeating that exact request does not filter the original pixels again. The
application's viewer requests 256x256 tiles, so this helps whole-viewport consumers
such as image comparison; tiled Fit views rely on the host's tile cache.

Only the released `midnight-image-viewer-sdk`/API and worker SDK are host
dependencies. The process enters its Windows AppContainer or Linux worker
restriction before reading content. The protocol contains no path, URL, native
handle, UI object or executable command. Decoding has a 32768-pixel edge and
256 MiB decoded-data budget (`MAX_DECODED_BYTES` of API 0.4.0); viewports have
4096-pixel edges and at most 4 Mi pixels.
Opaque source reads are at most 256 KiB. Decoder reads use a seek-preserving window
of at most 256 KiB that batches small codec reads and keeps fetched bytes across
codec seeks, so seeking codecs (TIFF strips and tiles) stay within the host's read
budget. The host enforces read volume, read
count, source revision, per-response deadlines and a 1 GiB/60 CPU-second process limit.
The CPU allowance is cumulative across a session; exhaustion ends that session.

Version 0.4.0 pins SDK 0.5.0 (API 0.4.0), whose decoded-data budget is 256 MiB instead
of 128 MiB; version 0.3.0 pinned SDK 0.4.0 (API 0.3.0). When decoding fails, the SDK sends the
decoder's error text, sanitized to at most 160 printable ASCII characters without
paths, after the failure, and the host shows it after its own sentence, for example
"This image is damaged or incomplete: the file is empty". The shared decoder names
empty sources and content that is not an image before trying SVG; codec errors keep
image-rs wording, such as "unexpected end of file" for a cut PNG. An SVG limit or
unsupported construct is a complete sentence starting with "This ", which the host
shows on its own ("This SVG is larger than 4 MiB, the limit for SVG images.").

Use a separately released offline SDK registry. The project never edits host
sources. All build/test output must lie below a `_build` directory: this project's own
(`./_build`), that of the repository holding it in `plugins/`, or otherwise the first
`_build` of the path. That `_build` and every folder below it must not be a link or
junction; the folders above it may be links or hidden, such as `AppData`:

```powershell
./scripts/ci.ps1 -SdkConfig <sdk>/.cargo/registry.toml -BuildRoot ./_build/check
./scripts/build.ps1 -SdkConfig <sdk>/.cargo/registry.toml -BuildRoot ./_build/build
```

```sh
bash scripts/ci.sh <sdk>/.cargo/registry.toml ./_build/check
bash scripts/build.sh <sdk>/.cargo/registry.toml ./_build/build
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

The native package tooling supports Windows x64 and Linux x86-64; ARM64 is not
supported.
