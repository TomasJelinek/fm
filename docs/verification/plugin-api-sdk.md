# Public plugin API/SDK verification — 2026-11-06

This record covers the initial isolated API/SDK gate: a draft C ABI, bounded
message models, owned asynchronous SDK, versioned offline release bundles and an
independently built conformance plugin. Subsequent work implemented application
discovery, the F3 window and independent Basic, Source and Markdown viewers; see
[viewer implementation verification](viewer-implementation.md) and the current
[developer guide](../developer/README.md). Filesystem/provider plugins remain
design-only. Counts and artifact hashes below are evidence from this earlier gate.

## Independent build and native boundary

`api/` and `plugins/viewer-conformance/` are independent Cargo workspaces with
their own locks. The fixture depends only on a versioned SDK from the `midnight`
registry; it has no path dependency on API sources or application crates.

A copied export contains only the plugin and released API/SDK bundle. Each platform
uses a fresh Cargo cache, explicit build directory and `--locked --offline`.
Cargo metadata confirms all nine source packages resolve inside that export;
only the fixture itself is a local package. The installed compiler and native SDK
remain external build prerequisites. This is a source-dependency isolation proof,
not a sandbox preventing trusted native plugins from accessing the process or OS.

Both Windows x64 and Linux x64 pass optimized fixture builds, strict all-target
Clippy and seven native C ABI scenarios. The C program loads the actual DLL/SO
through `midnight_plugin_entry_v1`; its Windows and Linux loaders are separate.

1. Short descriptor capacity leaves caller memory unchanged.
2. Rejected work creates no terminal; accepted work completes once.
3. Cancellation before execution still waits for physical job cleanup.
4. A positional read above 4 GiB validates generation and retains content through
   another worker; payload and raw output buffers release independently.
5. Shutdown consumes a late read reply without running its cancelled continuation.
6. One hundred progress events fill the bounded queue, suspend production without
   occupying a worker, then resume after drain/release and deliver one terminal.
7. An intentional Rust panic produces a failure event and clean destruction.

The panic hook's stderr output is expected in scenario seven; the test process
continues and exits successfully. Ordinary unwinding is contained; native UB,
aborts and double panics during stack unwinding are outside this guarantee.

## Rust and layout checks

The API has 22 tests, the SDK 32, and the release packager four. Windows and Linux
both pass all 58 and strict workspace/all-target Clippy.

The tests cover finite CBOR grammar/resource limits, invalid UTF-8/ranges, source
offset overflow, header/version checks, semantic text/binary/document models,
paired buffer allocation, foreign instance IDs, exactly-once release, terminal
reservations, queue pressure, asynchronous continuations, cancelled/stale replies,
panic cleanup and shutdown. A reproduced ID-reuse bug led to separate live-context
accounting: old retained contexts can no longer disappear from quiescence checks.
Completed retained contexts are capped at 128 per instance.

Native C and Rust agree on every public struct's size, alignment and field offsets
on Windows and Linux. The generated header is derived from the actual `repr(C)`
API, with a negative altered-layout test on Windows. The SDK owns opaque numeric
job tokens; neither side dereferences those cookies.

## Reproducible distribution

`api/scripts/package.ps1` / `package.sh` invoke a Rust maintainer tool. A fresh
bundle contains canonical API/SDK `.crate` archives, a real local registry index,
relocatable Cargo configuration, unchanged third-party vendor sources and the
released C header. No registry publication occurs. Original Cargo archive hashes
and repository provenance live in `artifacts.json`.

Cargo includes the whole Git commit in package metadata. Moving that metadata
outside our canonical archives prevents unrelated plugin commits from invalidating
their SDK lockfiles. All source/binary member bytes remain exact. Tests also prove
that changing source bytes changes the archive and an incorrect package prefix is
rejected. Content changes require version bumps after the draft API freezes.

Two independent packaging runs produced identical canonical SHA-256 values:

| Artifact | SHA-256 |
|---|---|
| `midnight-plugin-api-0.1.0.crate` | `51eab0fac11555c4224fe68abf188e9965bd75d5f9d6fed4264d9dd70d2f713e` |
| `midnight-plugin-sdk-0.1.0.crate` | `4c576bfd7238e97263ad51eb7e51387774e57b7b149450b1959fb08bf2cb4f9d` |

The same Windows-produced canonical archives passed the copied Linux SO build
and native C tests. The application loader and viewer content broker now also
exercise this contract, as recorded in the later viewer verification. ABI 1.0
remains a versioned draft without a long-term compatibility commitment.

Local ignored evidence: `_build/qa/viewer-plugins/`, including
`api-sdk-final-tests.log`, `api-sdk-final-clippy.log`, `linux-api-sdk-final.log`,
`windows-canonical-export.log`, `linux-canonical-export.log`, the exported Cargo
metadata and the RED/GREEN lifecycle/packaging tests. Generated source, archives,
compiler outputs and native test executables remain under `_build`.
