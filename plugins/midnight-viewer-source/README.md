# Midnight source viewer 0.1.0

Independent `cdylib` package `midnight.viewer.source`. The host owns windows,
fonts, colours, selection and content leases. This package owns decoding,
lexical highlighting, bounded context checkpoints and streaming search. It has
no host-source, other-plugin, Slint, filesystem-open or network dependency.

## Actual syntax catalogue

| Lexical profile | Extensions |
| --- | --- |
| Rust | rs |
| C | c, h |
| C++ | cpp, cc, cxx, hpp, hh, hxx |
| JavaScript | js, mjs, cjs, jsx |
| TypeScript | ts, tsx, mts, cts |
| Python | py, pyw |
| POSIX shell / Bash / Zsh | sh, bash, zsh |
| Windows batch / CMD | bat, cmd |
| JSON | json |
| TOML | toml |
| Java | java |
| C# | cs |
| Go | go |
| Swift | swift |
| Kotlin | kt, kts |
| Ruby | rb |
| SQL | sql |
| YAML | yaml, yml |
| XML / XAML / MSBuild projects and resources | xml, xsd, xsl, xslt, xaml, csproj, fsproj, vbproj, vcxproj, props, targets, resx |

Extension recognition is ASCII case insensitive. Extensionless Python, shell
and Node scripts are recognized from their first shebang line after Open.
Host routing must choose this package before a shebang can be inspected; the
manifest does not claim every extensionless file. The 19 profiles have fixtures
checking their actual lexical styles (JSON has no comments). These are lexical profiles, not compiler grammars: interpolation,
heredocs, JSX markup, macro expansion and language-specific semantic analysis
are not promised. Unknown names remain plain text.

XML highlights element/processing-instruction names, attribute names and quoted
values, comments, CDATA, declaration markup and entity/character references.
Unicode names follow XML 1.0 lexical ranges. Multiline lexical state survives
complete-line checkpoints; ordinary mixed content stays plain. This is colouring,
not XML validation: DTDs are inert text, references are never expanded, and no
external resources are read. Declaration nesting is bounded to 16. XML defaults
do not claim HTML or SVG. Saved per-plugin extension overrides remain in control.

Multiline strings and C-style block comments retain explicit state. Rust nested
block comments support depth 16; Python/TOML triple quotes and Rust raw strings
with up to 16 hashes are recognized. At most 256 complete-line checkpoints are
retained per session. A viewport can rescan at most 32 KiB before its anchor;
distant uncached positions or exhausted token/depth limits render plain text.
Returning near a cached checkpoint restores highlighting. Source text and byte
anchors remain identical when colouring is omitted.

## Bounds and protocol

Open, Viewport, Search and Close use only public API/SDK 0.1.0. Open returns
Opened; the host then requests its first viewport. All reads use owned SDK
continuations. Short reads continue until the requested bounded slice or EOF.
Close discards session state; the host owns closing the content reader lease.

Optional Locate requests never build a whole-file line index. They use the nearest
exact checkpoint and a cumulative 256 KiB read budget, including boundary lookahead
and decoder overlap. Each bounded read gather reserves its entire requested range;
short replies fill the same reserved range. If the target is too distant or the
budget runs out, the request completes without a TextPosition. The host displays
unknown coordinates for that page and can retry after navigating elsewhere.
Returning near an exact checkpoint can recover line/column values. Explicit
search remains a separate cancellable streaming operation.

UTF-8 and BOM UTF-16 LE/BE, surrogate pairs, malformed units, CR/LF/CRLF and
control pictures are supported. No terminal escape interpretation occurs.
Offsets use checked u64 arithmetic. Every displayed scalar boundary has an
exact source-byte anchor; incomplete encoded units are deferred to the next
read. Maximum emitted text is 8192 UTF-8 bytes, with a further requested line
limit. Enormous logical lines are fragments with `partial_line`/`has_after`.
Uncached seeks use four bytes of look-behind after UTF-16 word alignment and
select actual decoded boundaries. Backward pages retain the original source
anchor as an upper bound and select the nearest preceding displayed lines.
Forward search uses the same bounded alignment and excludes earlier matches.

Each instance permits 16 sessions and four active operations. Reads are at most
64 KiB, decoder text at most 64 KiB, styles at most 4096, and checkpoint state
at most 256 items per session. Four scratch decoders plus their inputs, output
models and state remain below the 16 MiB plugin budget. SDK queues separately
enforce their public byte/item caps. Cancellation is checked during decoding,
lexing and between every asynchronous read.

Search uses streaming KMP with a bounded query window across page boundaries.
Insensitive search compares Unicode lowercase expansions; this is not locale
collation or full Unicode case folding. Backward search scans from the start
and retains the last match before the supplied anchor, trading latency for
bounded memory. Reads yield host workers; progress is emitted at most four
times per second. There is no whole-file size cap and cancellation interrupts
the scan.

## Independent build, CI and installation

Export this directory anywhere. Install Rust 1.99.0 or newer and provide the
released public SDK directory containing `.cargo/config.toml` and `registry/`.
No application checkout is required. `prepare-sdk` reads the local registry
location and writes a config under BuildRoot. Plugin-specific crates come from
the normal Cargo cache, independently of the SDK's API-only vendor directory.
For an uncached machine, run `cargo fetch --locked --config GENERATED_CONFIG`
once with network access before the offline CI command.

Windows PowerShell:

```powershell
./scripts/ci.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/source/_build
./scripts/build.ps1 -SdkConfig C:/sdk/.cargo/config.toml -BuildRoot C:/qa/source/_build -OutputDirectory C:/app/viewers/source
```

Linux:

```sh
bash scripts/ci.sh /sdk/.cargo/config.toml /qa/source/_build
bash scripts/build.sh /sdk/.cargo/config.toml /qa/source/_build /app/viewers/source
```

CI runs formatting, every independent test, strict Clippy and a native release
build. Packaging writes the actual `rustc` host target and DLL/SO name into the
manifest. It only installs its own package directory; it never edits the host
or another plugin. Removing the package removes the capability at host startup.
Run CI on both Windows and Linux; exported builds also contain their ABI
host-service fixture, delayed/cancelled reads, large offsets and parser tests.
XML coverage includes tags/attributes, comments, CDATA, processing instructions,
multiline/Unicode state, UTF-16 projects, short reads, cancellation, token/depth
limits, huge logical lines and bounded seeks in a virtual 10 GB source.

## Dependency review and licensing

`regex = 1.13.1` is pinned with default features disabled, retaining `std` and
`perf`. Its [documented automata bounds](https://docs.rs/regex/1.13.1/regex/)
avoid backtracking and backreferences. Only a fixed anchored ASCII token rule
is compiled, with 128 KiB compilation/DFA limits; file content never supplies
a regex. The rest of the scanner advances by Unicode scalar or bounded
delimiter. Dependency source, manifest and licence were reviewed locally.

Plugin-owned code is MIT. See THIRD-PARTY.md and notices/ for external licences.
The public project API/SDK release currently has no declared licence; the
plugin's MIT licence does not grant rights over that separate dependency.
