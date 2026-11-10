# Independent native filesystem package verification — 7 November 2026

Scope: the new public Browse/Read API/SDK and independently built Windows/Linux
native packages. Root host composition, installation, optional-package revocation
and selected-file F3 routing remain separate acceptance gates. Existing archive
worker protocol 1 and viewer ABI are unchanged. This report does not mark VFS-01
or native platform migration complete.

## Qualified boundaries

Windows x64/MSVC and Linux x86-64 GNU (Ubuntu 24.04 under WSL, ext4 native syscall
fixtures) build separate workspaces with public-only API dependencies. API/SDK
are std-only; native packages reuse pinned windows-sys/windows-link/libc under
inspected MIT alternatives. Inventory includes 17 workspaces, with unchanged
488 third-party package/version records, 456 names and 58 direct records.

- Public API workspace: 29 tests plus one compile-fail cancellation-lifetime
  doctest on each OS. Includes the unchanged archive API's four tests, six new
  codec tests, four C-layout/default tests and 15 SDK ownership/lifecycle tests.
- Windows package: six tests, including an actual restricted DLL load and
  callback-origin check, five native filesystem cases and selected-file bytes.
- Linux package: eight tests, including actual dlopen/dlsym/callback origin,
  five native cases, a descriptor-race regression and mount parser behavior.
- Strict Clippy with `--all-targets -D warnings` passes for the public workspace
  and both native packages. The C header compiles as C11 with static layout
  assertions on Linux. Rust layout tests qualify both 64-bit native targets.
- The inventory generator's focused tests and regenerated-inventory `--check`
  pass. Both native package notices retain the full selected MIT attribution.

Native tests preserve raw unpaired Windows UTF-16 and invalid Unix bytes,
incremental page ordinals, independent retained readers, exact EOF/empty files,
source edit/replacement validation, canceled cleanup and wrong-mount rejection.
Windows exercises paths beyond 260 characters. Linux constructs and reads an
owned descriptor-relative pathname exceeding 4300 bytes. Source files remain
under the 100 KB limit and use UTF-8/LF; generated/test output is under `_build`.

## Causal review regressions

SDK resource publication initially committed native state before encoding.
With a 10-byte reply budget, 70 Resolve calls reached the adapter only 64 times:
six calls hit leaked mount slots. The behavioral RED is
`publication-red.log`; `publication-green.log` records the rollback fix. Authors
now return owned `Reply` transactions; C ABI/wire remain unchanged. Tests cover
tiny Resolve replies, oversized valid reader revisions, failed page encoding,
injected buffer publication failure, post-handler cancellation and cleanup-failure
poisoning. Failed pages close their cursor rather than advancing invisibly;
cursor busy state extends through publication. Cleanup ignores cancellation.

On Windows, ordinary-root enumeration preserved raw `source.` but the original
short-path content open read sibling `source`. The exact byte mismatch is in
`windows-literal-red.log`. Explicit verbatim OS paths fix both dotted and spaced
names; `windows-literal-green.log` includes actual library and native tests.
Provenance addresses stay lossless. Unsupported device namespaces and ambiguous
dot-component navigation are refused instead of silently retargeted.

On Linux, regular-file qualification originally followed a data open. The owned
fixture replaces the source pathname with a FIFO immediately before data open;
the old implementation loses the regular source (`linux-open-red.log`). The fix
opens an O_PATH/no-follow probe, qualifies it before data access, then reopens the
exact retained inode through the internal proc descriptor namespace. The causal
GREEN is `linux-open-green.log`. Native tests also replace a resolved directory
with a symlink: the old kind token fails SourceChanged; explicitly resolving the
new link still allows directory navigation.

## Evidence and limits

Logs under `_build/goal/native-plugin/`:
`api-workspace-{windows,linux}.log`, `api-clippy-{windows,linux}.log`,
`windows-literal-green.log`, `native-windows-clippy.log`,
`native-linux-tests.log`, `native-linux-clippy.log`,
`header-layout-linux.log`, `inventory-tests.log`, `inventory-final-check.log`.
Standalone debug libraries are in `native-windows/debug/deps/` and
`native-linux/debug/deps/` beneath that directory. Host release packaging must
rebuild/install the appropriate final library with its manifest and notices.

Content is LiveValidated using retained handles and identity/size/write/change
stamps. During qualification, an NTFS same-clock-tick edit could retain every
reported stamp; the same-size-edit fixture now explicitly ages its owned change
stamp before capture, avoiding a timing-dependent test. Stamp validation is not
an immutable snapshot, cryptographic integrity proof or mutation permission.
Linux pseudo-filesystem content without a reliable known length is Unsupported.
StorageKind remains Unknown except conservative network detection; no positive
SSD hint is invented. No Watch or mutation operation is implemented here.

macOS and aarch64 native packages/runners are unqualified. Host no-fallback,
absence/revocation and production F3 acceptance remain required; independent
package tests do not establish those properties.
