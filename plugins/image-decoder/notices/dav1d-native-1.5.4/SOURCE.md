# Static native AV1 decoder

dav1d 1.5.4 is linked statically under BSD-2-Clause.
Source: https://download.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz
SHA-256: 686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5

Built with tools, tests, examples and dependency downloads disabled.
The Rust dav1d/dav1d-sys bindings retain their adjacent MIT notices.

The build applies the upstream MSVC preprocessor fix from dav1d commit
`a34e0685b9111b9eb1d7835ea7bea5b425dced73` and uses the release version
from the archive when generating the Meson version header.
