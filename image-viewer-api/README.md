# Restricted raster API 0.4.0 and SDK 0.5.0

This independent public workspace exposes a filename-free, bounded protocol and
worker runtime. Providers use the released SDK registry package, never private
host crates. The SDK enters the native worker sandbox before invoking a parser.

Version 0.2.0 adds `DocumentMetadata`, `Message::PageViewport`, `Document` and
`serve_document`. Existing MIV1 image messages are byte-compatible. Image packages
continue declaring manifest protocol 1; paginated packages declare protocol 2.
`midnight-image-viewer-sdk` 0.3.0 keeps the 0.2.0 interface and only depends on
`midnight-worker-sdk` 0.2.0 instead of 0.1.0, which adds the explicit `MWS2`
startup policy and keeps the legacy `MWS1` handshake.

API 0.3.0 adds `FailureReason` (`Message::FailureReason`, tag 10): a provider's
short explanation of a `Failure`, 1–`MAX_REASON_BYTES` (160) printable ASCII bytes
without `/` or `\`, sent only directly after `Failure` as the last message.
`FailureReason::sanitize` reduces arbitrary error text to that form. SDK 0.4.0
writes `Failure` and then the sanitized reason of the provider's `io::Error` whenever
Open, document metadata or a render fails; its interface is otherwise unchanged.
Hosts before 0.3.0 stop reading at `Failure`, so they never see the reason; newer
hosts read on until the reason or the end of the stream (bounded) before stopping
the worker, and keep their plain message when a worker before SDK 0.4.0 sends none.
API 0.4.0 raises `MAX_DECODED_BYTES`, the original image's RGBA8 budget that both
`Metadata::validate` and providers' decoders apply, from 128 MiB to 256 MiB; nothing
else changes. SDK 0.5.0 only depends on API 0.4.0 instead of 0.3.0. The wire format is
unchanged and not negotiated: a host before 0.4.0 refuses a newer worker's metadata
above 128 MiB as an invalid message (its plain "damaged or incomplete" text), which
is accepted because the viewer plugins ship with the application.
The image and PDF viewer plugins pin
SDK 0.5.0. Provider error texts therefore reach the viewer's status line: keep them
short and free of paths.
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
