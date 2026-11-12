# AVIF resource limits

This is the published image-rs **0.25.10** source from
https://github.com/image-rs/image, licensed **MIT OR Apache-2.0**. Midnight selects
MIT; both upstream license files and the published VCS metadata are retained.
The registry package SHA-256 is
`85ab80394333c02fe689eaf900ab500fbd0c2213da414687ebf995a65d5a6104`, with upstream
commit `76e57184f22772dad1138e96954e57945406b15e`.

The only decoder change adds `AvifDecoder::new_with_settings(reader, &settings)`.
Both the primary and alpha dav1d decoders receive the same explicit settings before
allocating decoded frames. The existing `new` constructor delegates using upstream
default settings, preserving its behavior. YUV conversion, color handling and
container parsing remain upstream code. The independent Midnight image decoder
must limit encoded input and configure frame/thread limits; setting image-rs
`Limits` after construction is too late for the initial AV1 decode.

The local Cargo metadata marker keeps this dependency in third-party inventories.
Only independent image plugin workspaces select this patch; the application-wide
image-rs dependency is not replaced. Cargo's registry cache is never modified.
Published upstream source is retained; this package does not include upstream
integration fixtures excluded by the registry package. Runtime validation belongs
to the independent decoder/plugin tests, including actual AVIF pixels and
oversized primary/alpha rejection.
