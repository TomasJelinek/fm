# Restricted raster API 0.2.0 and SDK 0.3.0

This independent public workspace exposes a filename-free, bounded protocol and
worker runtime. Providers use the released SDK registry package, never private
host crates. The SDK enters the native worker sandbox before invoking a parser.

Version 0.2.0 adds `DocumentMetadata`, `Message::PageViewport`, `Document` and
`serve_document`. Existing MIV1 image messages are byte-compatible. Image packages
continue declaring manifest protocol 1; paginated packages declare protocol 2.
`midnight-image-viewer-sdk` 0.3.0 keeps the 0.2.0 interface and only depends on
`midnight-worker-sdk` 0.2.0 instead of 0.1.0, which adds the explicit `MWS2`
startup policy and keeps the legacy `MWS1` handshake. The image and PDF viewer
plugins pin SDK 0.3.0.
Released packages are immutable: a change ships as a new version, never as
different contents for an existing one.

Document metadata contains 1–4,096 ordered page dimensions. Each dimension uses
the existing validated logical-size contract. PDF providers conventionally use
points. Page requests contain a zero-based page number and the existing physical
viewport scale/origin; output is straight-alpha sRGB RGBA8. The output/request ID
must match, and the host rejects a page index outside the captured metadata.
All source reads occur during `Open`; subsequent rendering uses retained content.
The SDK rejects invalid page requests before calling the provider renderer.

The protocol has no filenames, URLs, native handles, markup or script channels.
It does not itself isolate code: the native worker sandbox, host source broker,
bounded supervisor, source validation and trusted installation checks are required.
