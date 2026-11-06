# Third-party material

`fixtures/CaseFolding-17.0.0.txt` is Unicode 17.0.0 data from
https://www.unicode.org/Public/17.0.0/ucd/CaseFolding.txt, distributed under the
included `UNICODE-LICENSE.txt` (Unicode License V3). `src/casefold.rs` is generated
from its default C and F entries by `scripts/generate-casefold.ps1`; no download
occurs during plugin build or runtime.

The public SDK bundle supplies `midnight-plugin-api`, `midnight-plugin-sdk`,
`minicbor`, `minicbor-derive`, `proc-macro2`, `quote`, `syn` and `unicode-ident`.
The pinned third-party crates' original license and copyright files are included
under `notices/` in every standalone plugin package. These are copied from the
released SDK bundle's vendored sources and cover build dependencies as well as
runtime dependencies. `UNICODE-LICENSE.txt` covers the bundled case-folding data.
No parser, GUI, filesystem or application dependency is linked.

The public API/SDK retains its existing project license status; these third-party
notices do not assign a license to Midnight Explorer or its API/SDK source.
