# AVIF resource limits

This is the published image-rs **0.25.10** source from
https://github.com/image-rs/image, licensed **MIT OR Apache-2.0**. Midnight selects
MIT; both upstream license files and the published VCS metadata are retained.
The registry package SHA-256 is
`85ab80394333c02fe689eaf900ab500fbd0c2213da414687ebf995a65d5a6104`, with upstream
commit `76e57184f22772dad1138e96954e57945406b15e`.

The decoder changes are limited to `src/codecs/avif/decoder.rs`:

- `AvifDecoder::new_with_settings(reader, &settings)`. Both the primary and alpha
  dav1d decoders receive the same explicit settings before allocating decoded
  frames. The existing `new` constructor delegates using upstream default
  settings, preserving its behavior.
- `read_until_ready` terminates when an AV1 item contains no displayable frame
  (for example only a sequence header, padding or a hidden frame). Upstream loops
  on dav1d's `Again` forever once all input is consumed, which pins a core until
  the host's deadline kills the worker. After the second consecutive `Again` with
  no pending input (the first only enables dav1d's draining, the second drains its
  frame threads), construction fails with "AV1 item produced no displayable frame".
  Items that produce a picture decode exactly as before.

YUV conversion, color handling and container parsing remain upstream code. The
independent Midnight image decoder must limit encoded input and configure
frame/thread limits; setting image-rs `Limits` after construction is too late for
the initial AV1 decode. With those limits and the bounded frame wait above, AVIF
construction is bounded in memory and does not spin; total CPU time still depends
on the decoder's per-frame limits and the host's process deadlines.

The local Cargo metadata marker keeps this dependency in third-party inventories.
Only independent image plugin workspaces select this patch; the application-wide
image-rs dependency is not replaced. Cargo's registry cache is never modified.
Published upstream source is retained; this package does not include upstream
integration fixtures excluded by the registry package. Runtime validation belongs
to the independent decoder/plugin tests, including actual AVIF pixels, oversized
primary/alpha rejection, and primary/alpha items without a displayable frame.
