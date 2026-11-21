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
| Lua | lua |
| XML / XAML / MSBuild projects, resources and Windows manifests | xml, xsd, xsl, xslt, xaml, csproj, fsproj, vbproj, vcxproj, props, targets, resx, manifest |
| Universal Scene Description ASCII (USDA) | usda |
| CSS | css |
| GNU make | mk, mak, and the whole names makefile and gnumakefile (`Makefile`, `GNUmakefile`) |
| CSV (comma- or semicolon-separated values) | csv |
| TSV (tab-separated values) | tsv |

Extension recognition is ASCII case insensitive. The host also matches an
association entry against a whole file name, so the `makefile` and `gnumakefile`
entries route `Makefile` and `GNUmakefile`; hosts that predate that rule open them
as plain text. Extensionless Python, shell
Node and Lua scripts are recognized from their first shebang line after Open.
Host routing must choose this package before a shebang can be inspected; the
manifest does not claim every extensionless file. The 25 profiles have fixtures
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

Windows `.manifest` files use that same XML lexer. USDA has its own finite
streaming lexer for prim/edit keywords, Sdf value types, schema names after prim
specifiers, decimal/exponent/non-finite numbers, quoted and triple-quoted strings,
asset literals, angle-bracket scene paths and hash/slash/block comments. Triple
asset delimiters and their escapes follow the
[OpenUSD text grammar](https://github.com/PixarAnimationStudios/OpenUSD/blob/v26.11/pxr/usd/sdf/textFileFormatParser.h);
assets stay single-line, while triple-quoted strings retain multiline state.
Unknown namespace identifiers stay plain. Long literals, comments, names and
numbers retain finite Copy state, with at most 32 UTF-8 bytes of ambiguous suffix
left to the caller. Existing checkpoints and optional preparation recover that
state for distant windows. No USD document, layer, asset or scene is parsed,
evaluated or loaded. `.usd`, `.usdc` and `.usdz` are not claimed. Default extension
advertisements do not replace an explicit saved per-plugin extension override,
including an empty override; explicit Text mode remains unhighlighted.

CSS colours selectors (type, class, ID, pseudo and attribute names), property
names including custom properties, at-rules, `!important`, comments, strings and
unquoted `url(...)` contents, numbers with units and percentages, 3/4/6/8-digit
hexadecimal colours and function names. Rule blocks and declaration blocks are told
apart from the enclosing at-rule: `@media`, `@supports`, `@container`, `@layer`,
`@scope`, `@starting-style`, `@document` and `@-moz-document` hold what the block
around them holds (rules at the top level), `@keyframes` and its `-webkit-`, `-moz-`
and `-o-` forms hold keyframe selectors, and others hold declarations. Nesting depth
is a counter with 64 remembered levels, deeper blocks hold declarations. Inside a
declaration block a name followed by a colon is a property, otherwise a nested
selector, so a nested `a:hover` without `&` reads as a property. Values, cascade and
imported resources are never evaluated.

GNU make colours targets (special targets such as `.PHONY` as keywords), variable
names before `=`, `:=`, `::=`, `:::=`, `?=`, `+=` and `!=`, `$(...)`, `${...}`, `$x`
and automatic-variable references, the names of make's built-in functions,
directives at the start of a line (`include`, `ifeq`, `define`, `export`, ...),
comments including backslash-continued comment lines, and recipes: a line that
starts with a tab after a rule, or the text after `;` on a rule line, is shell
text with its `@`, `-` and `+` prefixes marked and a leading `#` line shown as a
comment. A tab-indented line before any rule is ordinary make syntax. `define`
bodies are literal text up to their `endef`, nested `define`s included. Backslash
newlines continue a logical line; an escaped backslash does not. Nothing is
expanded, included or run, and `.RECIPEPREFIX` is not honoured.

CSV and TSV colour every field by its column, in the manner of Rainbow CSV: the
first column keeps the text colour, the next four take the string, number, function
and type colours, and the cycle repeats every five columns. The header line uses the
same colours, so it names each colour's columns. Delimiters and the quotes around a
quoted field stay plain. CSV follows RFC 4180 quoting: a field that starts with a
quote may contain commas, doubled quotes and line breaks, and its column continues
across those line breaks; a record ends at an unquoted LF, CR or CRLF. A quote later
in a field and text after a closing quote stay in that field. An opening quote that
is never closed keeps the following lines in its field up to the next quote. TSV is
the IANA tab-separated form: tabs separate fields, each line is one record and quotes
are ordinary text, so a stray quote cannot affect later lines. The lexer keeps only a
column number and a quoting mode, so long lines and huge files are coloured in one
pass without buffering a field. Values are not typed or validated.

A `.csv` file is separated by commas or by semicolons, as spreadsheets save CSV in
locales whose decimal separator is the comma; semicolon files quote like comma files,
and their commas are ordinary text. Open decides once, from the file's first record
as far as Open reads it (the first 512 bytes of the file, without a byte order mark,
at most four lines). That record is split once at semicolons and once at commas, each
with RFC 4180 quoting, so a delimiter inside a quoted field counts for neither, and
semicolons are chosen only when they give more fields. Equal counts, a record with
neither delimiter and an empty file keep commas: `a;b,c` is comma-separated, while
`Price, EUR;Qty;Note` and a spreadsheet's `sep=;` first line are semicolon-separated.
Later records never change the choice, so their decimal commas stay inside fields and
every page and checkpoint of the file splits at the same delimiter. `.tsv` files
always split at tabs.

Lua highlights keywords, numbers, short strings and comments, and matched-level
long bracket strings/comments. Short-string escaped newlines and `\z` whitespace
escapes retain their state. YAML highlights mapping keys, scalar strings,
numbers/booleans/null, flow indicators and comments. Quoted scalars and literal or
folded block scalars retain state, including indentation/chomp indicators, blank
lines and dedent. Hashes inside unquoted scalar values such as URLs stay literal.
Neither profile executes content, resolves tags or validates a whole document.

Multiline strings and C-style block comments retain explicit state. Rust nested
block comments support depth 16; Python/TOML triple quotes and Rust raw strings
with up to 16 hashes are recognized. Single-line C-family, Java, C#, Go,
Kotlin, Swift, JavaScript/TypeScript, JSON, Batch and non-triple Python/TOML
literals end at an unescaped newline. Rust lifetimes and loop labels do not
open character literals, and JSON has no single-quoted strings. Java, Kotlin,
Swift and C# triple quotes and Go/JavaScript backticks span lines. TOML/shell
single quotes, Go raw strings and C# verbatim/raw literals take backslashes
literally; SQL and C# verbatim strings support doubled delimiters.

At most 256 syntax-state records are retained per session: 128 distributed
complete-line points, 126 recent hot points, an exact streaming frontier and a
reserved target snapshot. Distributed coverage starts with 2 KiB spacing and
doubles that spacing when full, retaining the BOM and earlier visited regions.
The fixed record split also bounds the aggregate preparation index to 64 KiB.
C++ raw strings retain their custom delimiter (up to 16 characters).
A viewport can rescan at most 32 KiB before its anchor;
distant uncached positions initially render plain text. A host supporting the
optional preparation request recovers the true lexer state in background quanta
and refreshes the latest stationary view. The first page remains window-bounded.
Huge files have no preparation size cutoff; a cold distant target can require
O(prefix) background I/O. Exhausted lexical depth/counter limits fail preparation
explicitly while the existing readable page remains available.

Dense pages are shortened at exact character/source boundaries when their styles
and anchors exceed the wire budget. Forward pages retain their beginning and
backward pages their end; continuation flags permit fetching the remaining text.
Styles are retained rather than clearing all colouring to fit a larger page.

## Bounds and protocol

This independent package uses only the released public API/SDK 0.2.0. Open returns
Opened; the host then requests its first viewport. All reads use owned SDK
continuations. Short reads continue until the requested bounded slice or EOF.
Close discards session state; the host owns closing the content reader lease.

Optional `PrepareSource` (request 7) is separate from ordinary Open/Viewport.
Each successful quantum emits one `SourcePrepared` (event 8), then Completed;
readiness is provisional until that terminal event. The full Viewport and source
revision identify the reservation. Its exact lexer snapshot and any resolved
token-role hints survive ordinary checkpoint insertion until a matching foreground
request takes an owned copy before its asynchronous read. Older hosts never send
this optional request and retain ordinary viewing; C ABI remains 1.0.

Preparation uses at most one 16 KiB positional read at a time, at most 64 service
replies and 256 KiB delivered bytes per quantum (including short replies), and
at most 64 KiB decoded text per turn. Its frontier excludes incomplete encoded
units and unconsumed lexical suffixes. A fixed raw tail handles one-byte replies;
huge identifiers, raw strings, Lua brackets, YAML keys/blocks, XML names/entities and USDA literals
use streaming state/counters instead of token or line buffers. Token roles that
depend on distant terminators are resolved by bounded background lookahead before
readiness, with at most two small role hints. The complete retained syntax index,
frontier and target state are bounded to 64 KiB and 256 records per session.

Optional Locate requests never build a whole-file line index. They use the nearest
exact checkpoint and a cumulative 256 KiB read budget, including boundary lookahead
and decoder overlap. Each bounded read gather reserves its entire requested range;
short replies fill the same reserved range. If the target is too distant or the
budget runs out, the request completes without a TextPosition. The host displays
unknown coordinates for that page and can retry after navigating elsewhere.
Returning near an exact checkpoint can recover line/column values. Explicit
search remains a separate cancellable streaming operation.

UTF-8 and BOM UTF-16 LE/BE, surrogate pairs, malformed units, CR/LF/CRLF and
control pictures are supported. C1 controls and explicit bidirectional embedding,
override and isolate controls (U+202A-U+202E, U+2066-U+2069) display as U+FFFD;
implicit direction marks remain. No terminal escape interpretation occurs.
Offsets use checked u64 arithmetic. Every displayed scalar boundary has an
exact source-byte anchor; incomplete encoded units are deferred to the next
read. Maximum emitted text is 8192 UTF-8 bytes, with a further requested line
limit. Ordinary reads use encoding-aware page spans: one source byte per text
byte and normalized line break for UTF-8, two for UTF-16, plus an 8-byte boundary
margin. Highlighted forward reads also reserve bounded lexical lookahead;
decoding stops after the replay prefix, selected page and 128 text bytes of
lookahead. Plain and unrecognized profiles never replay syntax checkpoints.
Enormous logical lines are fragments with `partial_line`/`has_after`.
Uncached seeks use four bytes of look-behind after UTF-16 word alignment and
select actual decoded boundaries. Backward pages retain the original source
anchor as an upper bound and select the nearest preceding displayed lines.
Forward search uses the same bounded alignment and excludes earlier matches.

Each instance permits 16 sessions and four active operations; Locate and
PrepareSource leave one slot for foreground work. Admission reclaims cancelled
requests' operation slots while the SDK retains their pending read continuations
until the host replies. Reads are at most
64 KiB, decoder text at most 64 KiB, styles at most 4096, and checkpoint state
at most 256 items per session. Four scratch decoders plus their inputs, output
models and state remain below the 16 MiB plugin budget. SDK queues separately
enforce their public byte/item caps. Host cancellation polling is batched during
decoding, lexing and search, and checked between asynchronous reads. Foreground
lexical replay polls between bounded chunks rather than once per token or line.

Search uses streaming KMP with a bounded query window across page boundaries.
Insensitive search compares Unicode lowercase expansions; this is not locale
collation or full Unicode case folding. Each 32 KiB read is decoded and scanned
completely, so source bytes are requested about once. Backward search scans from the start
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
