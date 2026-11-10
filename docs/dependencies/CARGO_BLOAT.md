# Optional executable-size diagnostic

`cargo-bloat` 0.12.1 is a development-only tool, installed with its published
lockfile. It is neither linked into Midnight Explorer nor included in releases.
Its [pinned upstream source](https://github.com/RazrFalcon/cargo-bloat/tree/69ccabda2b1671c4fd675e9a660af29ee985506f)
and [MIT license](https://github.com/RazrFalcon/cargo-bloat/blob/69ccabda2b1671c4fd675e9a660af29ee985506f/LICENSE)
permit this use without mandatory fees. Original license files remain in Cargo's
source cache. Application dependency lockfiles and features are unchanged.

The following graph was captured on Windows x64 with Cargo metadata from the
installed 0.12.1 manifest and lockfile on 8 November 2026. MIT is the selected
alternative for every multi-licensed entry. This table includes the tool itself;
platform-excluded packages are not asserted to be compiled into the Windows tool.

| Package | Version | Declared license |
| --- | --- | --- |
| aho-corasick | 1.1.3 | Unlicense OR MIT |
| binfarce | 0.2.1 | MIT |
| cargo-bloat | 0.12.1 | MIT |
| fallible-iterator | 0.2.0 | MIT/Apache-2.0 |
| json | 0.12.4 | MIT/Apache-2.0 |
| memchr | 2.7.2 | Unlicense OR MIT |
| memmap2 | 0.9.4 | MIT OR Apache-2.0 |
| multimap | 0.10.0 | MIT OR Apache-2.0 |
| pdb | 0.8.0 | MIT OR Apache-2.0 |
| pico-args | 0.5.0 | MIT |
| regex | 1.10.4 | MIT OR Apache-2.0 |
| regex-automata | 0.4.6 | MIT OR Apache-2.0 |
| regex-syntax | 0.8.3 | MIT OR Apache-2.0 |
| scroll | 0.11.0 | MIT |
| term_size | 0.3.2 | MIT/Apache-2.0 |
| uuid | 1.8.0 | Apache-2.0 OR MIT |
| winapi | 0.3.9 | MIT/Apache-2.0 |

Reproduction:

```powershell
./scripts/cargo.ps1 install cargo-bloat --version 0.12.1 --locked `
    --root .tools/cargo-bloat --target-dir _build/tools/cargo-bloat --jobs 2
```

The diagnostic's manifest metadata and build logs are retained under
`_build/qa/cargo-bloat-20261108/`. The published lockfile also mentions the yanked
`libc` 0.2.154, which is not in this Windows target graph; the locked install warning
is retained in the task log. No application dependency was downgraded to that version.
