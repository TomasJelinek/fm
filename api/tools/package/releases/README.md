# Immutable public API/SDK release inputs

`0.1.0/` preserves the exact previously released API and SDK archives, registry
records, C header and provenance receipt. Clean checkouts need these small inputs
because independent Basic and Markdown plugins retain released `=0.1.0` pins while
Source uses released `=0.2.0`. Sparse CI plugin jobs receive the resulting bundle
without application or unpublished public API sources.

The original release records identify commit
`a93557d4fc650c6dd8d83ad4ddd66003841a3e5a`. Frozen SHA-256 identities are checked in
the packager before seeding each fresh output. Archives and old index/header bytes
are copied verbatim. They are never reconstructed from current sources, repacked,
downloaded from an assumed external service, or overwritten under the same version.

API 0.1.0 archive: `f264a39c3ee43af518926887a7ef9e4a3751c093f21e924ca0e7f1e3d8009711`.
SDK 0.1.0 archive: `3cbd8b9bd227807c58ee3467a895e93ee0c59eb88064a8fc694f859233c8220a`.
Header: `def35c854f09ed1304ecb0ebe7c170e1135ff56bf60cac6e61641a0844625a01`.

Fresh bundles contain both old and current registry versions. Their top-level
`include/midnight_plugin_v1.h` remains the current generated header; the immutable
old header is also retained under `include/0.1.0/`. `artifacts.json` includes old
provenance with `preserved=true` and newly packaged current-source records.

The current canonical package function removes only repository VCS provenance
from Cargo archive contents, normalizes archive metadata and retains source bytes.
Actual VCS provenance stays in the distribution receipt. Independent Git commits
therefore do not invalidate a plugin's locked release checksum. Same-version
metadata/checksum/content conflicts fail rather than changing existing releases.

No project's or dependency's license is changed by preserving these artifacts.
