# Optional executable-size diagnostic

`cargo-bloat` 0.12.1 is a development-only tool, installed with its published lock
file into `.tools/cargo-bloat`. It is not linked into Midnight Explorer and not
included in releases. Its [pinned upstream source](https://github.com/RazrFalcon/cargo-bloat/tree/69ccabda2b1671c4fd675e9a660af29ee985506f)
and [MIT license](https://github.com/RazrFalcon/cargo-bloat/blob/69ccabda2b1671c4fd675e9a660af29ee985506f/LICENSE)
permit this use without fees; the crate archive keeps the license file.

The table lists the packages of the tool's Windows x64 build, the tool included, with
the licenses their pinned crates declare. MIT is the selected alternative for every
multi-licensed entry. The published lock file also pins the yanked `libc` 0.2.154
(MIT OR Apache-2.0), which `term_size` uses only outside Windows, and the
`winapi-i686-pc-windows-gnu` and `winapi-x86_64-pc-windows-gnu` 0.4.0 shims
(MIT/Apache-2.0), which an MSVC build does not use.

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

Installation:

```powershell
./scripts/cargo.ps1 install cargo-bloat --version 0.12.1 --locked `
    --root .tools/cargo-bloat --target-dir _build/tools/cargo-bloat --jobs 2
```

Cargo warns that the lock file pins a yanked `libc`; the Windows build does not use it.
