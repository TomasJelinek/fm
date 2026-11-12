# Restricted image thumbnails

The independent worker uses [the shared decoder](../image-decoder/README.md) for
PNG, JPEG, GIF, WebP, BMP, ICO, TIFF, bounded static SVG and AVIF. The first still frame
is oriented, reduced without upscaling and converted to straight-alpha sRGB.
SVG rasterizes directly to the requested aspect ratio with embedded Noto fonts;
it receives no path, network or system-font access. See the shared decoder's
explicit SVG subset, resource limits and bundled font provenance.

The provider version changes whenever decoding policy changes, invalidating old
thumbnail cache identities. AVIF shares the viewer's pinned static dav1d decoder,
32 MiB encoded-container cap, and primary/alpha native frame limits. HEIC/HEIF is
not enabled. The root thumbnail build scripts prepare pinned native tools/static
code and copy the full shared notice set, including the MPL parser source archive.
Use `-NativeOffline` or `MIDNIGHT_IMAGE_NATIVE_OFFLINE=1` to forbid native downloads.
