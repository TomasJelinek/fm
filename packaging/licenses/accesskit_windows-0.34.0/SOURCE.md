# accesskit_windows 0.34.0 license supplement

The exact published crate declares `MIT OR Apache-2.0` but omits standalone
license files. This supplement retains the unmodified root license texts from
the upstream commit recorded in that crate's `.cargo_vcs_info.json`. Apache-2.0
is the selected free branch. MIT copyright/license text and the Chromium BSD
copyright, conditions and disclaimer are retained as upstream attribution;
retaining them does not change the declaration in the generated inventory.
This supplement does not change the application or SDK license.

- Published crate: <https://static.crates.io/crates/accesskit_windows/accesskit_windows-0.34.0.crate>
- Cargo.lock / crate archive SHA-256: `106c2b961215864d1c2e703ee63269c25c4e80a577ffb2c1017b9c17dcdf83a1`
- Upstream repository: <https://github.com/AccessKit/accesskit>
- Recorded commit: `c88605b96d04431f9c3c792464a0f2f253480e94`
- Recorded package directory: `platforms/windows`

| Retained unmodified file | Exact upstream source | Bytes | SHA-256 |
| --- | --- | --- | --- |
| LICENSE-APACHE | <https://raw.githubusercontent.com/AccessKit/accesskit/c88605b96d04431f9c3c792464a0f2f253480e94/LICENSE-APACHE> | 9723 | `62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a` |
| LICENSE-MIT | <https://raw.githubusercontent.com/AccessKit/accesskit/c88605b96d04431f9c3c792464a0f2f253480e94/LICENSE-MIT> | 1023 | `23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3` |
| LICENSE.chromium | <https://raw.githubusercontent.com/AccessKit/accesskit/c88605b96d04431f9c3c792464a0f2f253480e94/LICENSE.chromium> | 1559 | `845022e0c1db1abb41a6ba4cd3c4b674ec290f3359d9d3c78ae558d4c0ed9308` |

Pass `packaging/licenses` as `--license-overrides` to the existing release
notice collector. It copies this version-specific supplement only when the
resolved package has no packaged license files; keep the original filenames
and complete texts in the distributable `licenses/` tree.
