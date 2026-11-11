# Renderer source patch

`femtovg/` is the published FemtoVG 0.27.0 source, licensed **MIT OR Apache-2.0**.
The application selects MIT and retains both upstream license files. Upstream:
https://github.com/femtovg/femtovg . The registry crate and its VCS metadata are
recorded by the vendored package. Cargo registry caches are not modified.
The published crate SHA-256 is
`31eebf3ab1b76359ccd5f850c07a7c6b2722c64d9127c40e02358e83f3ff9b1a`;
its recorded upstream commit is `0eb55780894a9200e9a70ca3d16bd1c641281a44`.
The local manifest marker keeps this path dependency in license inventories and
release notices; it does not turn it into project-owned code.

The host workspace patch adds bounded reuse of managed image/glyph bind groups.
Native DX12 stack sampling identified per-draw bind-group allocation and release
as a scrolling bottleneck: each sampler binding creates an internal GPU buffer.
The upstream mapper retained only the current group and rebuilt groups each frame.

Cache entries validate actual texture identities and sampler flags, not just
image ids (reallocation can preserve an id). Changing the uniform buffer clears
old bindings; image deletion removes all references to that image. Reallocated
unused images are pruned after the next nonempty rendered frame or renderer drop.
Pixel-only image updates retain their
bindings. Transient unmanaged filter textures bypass the cache. Least-recently
used eviction caps the cache at 512 bindings per renderer. There is no cross-thread
or cross-device cache, and WGPU retains resources needed by submitted commands.

The source patch is in `src/renderer/wgpu.rs` and its new `wgpu/bindings.rs` module.
Upstream tests remain alongside the source. The packaged book, GitHub workflows
and Cargo extraction marker are omitted; they are not needed to build or test.
The standalone lockfile starts from the published lock. Its Windows API family
is unified at `windows` 0.62.2 to match WGPU 30 and gpu-allocator; the original
0.58 selection did not compile their DX12 interface types together. This affects
only the independent test graph; the application retains its existing versions.

Run the added GPU regressions from the repository root (the explicit tests require
a real adapter and fail if none is available):

```powershell
$env:WGPU_BACKEND = 'dx12' # Use vulkan on a corresponding Linux GPU host.
cargo test --manifest-path vendor/femtovg/Cargo.toml --lib `
  --no-default-features --features wgpu --locked --target-dir _build `
  renderer::wgpu::bindings::tests -- --ignored --test-threads=1
```

They check actual output pixels, unchanged-frame reuse, pixel updates, same-id
texture replacement, deletion, uniform-buffer growth, eviction and pruning.
Upstream font tests requiring the excluded example assets need the upstream
checkout; they are not claimed as part of this focused GPU validation.

Performance and native image invalidation checks are documented in
[the scrolling investigation](../docs/NATIVE_SCROLL_VERIFICATION.md). This patch
is retained only with functional GPU checks and a measured native improvement.
