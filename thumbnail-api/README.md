# Thumbnail executable API 0.1.0

This workspace is separate from the already published viewer API/SDK 0.1.0.
It neither changes those packages nor extends their binary viewer ABI.
`midnight-thumbnail-sdk` and `midnight-worker-sdk` are the only Midnight
dependencies an independent decoder needs; neither references application crates.

Installed providers live only in `thumbnailers/` next to the application executable.
Each has a bounded JSON manifest with the exact fields shown in the independent
image/video projects: protocol `1`, stable provider ID, decoder/cache version,
one executable basename, lowercase extension hints and a video flag. Windows
installation appends `.exe` to the manifest executable. Trust checks cover the
installation ancestry and each manifest/helper. The host does not download plugins,
search PATH, pass media paths, or invoke shells. Extension hints choose a decoder;
the decoder still validates content and may reject it.

The worker host first sends `MWS1` limits. Before reading any media request the SDK
verifies Windows AppContainer/job restrictions or installs Linux seccomp/rlimits,
then replies `MWR1`. `MTH1` length-prefixed messages follow. Requests specify a
source length, maximum dimensions, request identity and frame index 0–5. Helpers
read/seek through bounded read-only broker messages. Responses contain exact-size
straight-alpha RGBA8 sRGB pixels or a failure; no paths, URLs, HTML or executable
content are accepted as results. The parent validates identity, size, source
revision, cancellation and deadlines before accepting a frame.

Frame zero is the ordinary static preview. Frames one through five are additional
video samples, requested only for visible selected/hovered items. Every accepted
frame is persisted before presentation. Unavailable helpers retain ordinary icons;
there is no in-process decoder fallback.

`api/tools/package` binary `worker-packages` emits a separate deterministic package
registry and vendored SDK bundle under an explicit `_build` directory. It includes
the filesystem API when that independent workspace is present. Existing viewer
release archives and checksums remain unchanged. Plugin manifests must pin the
released SDK version and registry before independent distribution; repository path
overrides are development-only until that bundle is generated and verified.
