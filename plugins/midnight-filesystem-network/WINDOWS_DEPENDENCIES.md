# Windows adapter dependency and integration notes

Add this target-specific dependency to the shared independent plugin manifest:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "=0.61.2", default-features = false, features = ["Win32_Foundation", "Win32_NetworkManagement_WNet"] }
```

Wire `#[cfg(windows)] pub mod windows;` into the shared library and select
`windows::Windows` for its Windows adapter. The shared `adapter.rs` was copied
without modification. Scratch library/manifest/lock changes were solely a
standalone qualification harness and are deliberately excluded from the Windows
source patch; do not replace the root shared library or policy files with them.

Actual pinned manifests and license files in the local Cargo registry were checked:

| Crate | Registry checksum | License choice |
| --- | --- | --- |
| windows-sys 0.61.2 | ae137229bcbd6cdf0f7b80a31df61766145077ddf49416a728b02cb3921ff3fc | MIT from MIT OR Apache-2.0 |
| windows-link 0.2.1 (only transitive dependency) | f0805222e57f7521d6a62e36fa9163bc891acd422f971defe97d64e70d0a4fe5 | MIT from MIT OR Apache-2.0 |

The resolved feature closure is Win32_Foundation, Win32,
Win32_NetworkManagement and Win32_NetworkManagement_WNet. There are no default
windows-sys features and no WinSock, credential, Shell or filesystem feature.
The three display-type numeric constants exposed by windows-sys under its
unrelated WinSock feature are documented locally instead of enabling that feature.

These pinned crates already exist in the project's inventory. This independent
package reuses them with the added WNet feature; integration must record this
package's usage in the inventory and include the following notice in its
distribution notices. Neither crate requires fees, subscriptions or royalties.
The project's own license is unchanged. No Salamander GPL implementation or
resources were copied; only the accepted interface behavior informed the adapter.

The identical MIT notice below applies to both pinned Windows binding crates:

```text
MIT License

Copyright (c) Microsoft Corporation.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE
```
