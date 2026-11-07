# Restricted image viewer

Independent process plugin for Midnight Explorer. PNG, JPEG, GIF, WebP, BMP, ICO
and TIFF use exactly the pinned image-rs and moxcms libraries used by thumbnails.
The first still frame is decoded once, oriented, converted to sRGB and retained
inside the sandbox. Viewports sample the original pixels: 1:1 is exact,
magnification uses nearest original pixels, and reduction uses alpha-weighted area
filtering. No full zoomed-image allocation is created.

Only the released `midnight-image-viewer-sdk`/API and worker SDK are host
dependencies. The process enters its Windows AppContainer or Linux worker
restriction before reading content. The protocol contains no path, URL, native
handle, UI object or executable command. Decoding has a 32768-pixel edge and
128 MiB decoded-data budget; viewports have 4096-pixel edges and at most 4 Mi pixels.
Opaque source reads are at most 256 KiB. The host enforces read volume, read count,
source revision, per-response deadlines and a 512 MiB/60 CPU-second process limit.
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
Removing this package removes image support. Tests generate their own image data.
