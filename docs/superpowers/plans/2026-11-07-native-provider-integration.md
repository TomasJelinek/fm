# Native filesystem packages and application integration

> Implement the accepted filesystem-plugin design with independent API/plugin
> projects and the existing host security boundary. Execute the tasks with causal
> tests; a mock registry alone does not qualify production integration.

**Goal:** Route native browsing and viewer capture through installed native
filesystem packages, while retaining archive protocol 1 and viewer compatibility.

**Architecture:** A separate versioned C service ABI exposes bounded synchronous
Browse/Read calls. Host backend workers own dispatch, cancellation and library
lifetimes. Native packages own Windows/Linux filesystem implementations in
separate projects and depend only on the public API/SDK. Host adapters preserve
raw native paths during migration; display labels never grant native authority.

**Spec:** `docs/superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md`,
`docs/developer/FILESYSTEM_MIGRATION.md`, and backlog VFS-01.

## Constraints and ownership

- Existing trusted-library admission remains mandatory; native in-process code
  is trusted, not sandboxed. No ACL weakening or shared-directory workaround.
- No `MAX_PATH`, ANSI conversion, UI I/O or private host dependency in plugins.
- Keep archive worker messages and viewer ABI unchanged. Absence/unavailability
  must fail explicitly, never invoke built-in native browsing as a fallback.
- Root owns host integration and viewer capture; the API/package worker owns
  independent API/SDK and native packages. Transfer and crash workers own their
  separate domains. Cargo access is coordinated per platform.

## Tasks and evidence

1. Inject native viewer capture into `midnight-viewer-engine::Service`, retaining
   the current default for unconverted callers. Capture current and Space-next
   files through the supplied service on workers. Test exact visible order,
   non-native rejection, cancellation, failed-provider refusal and retained
   provider content. A callback error must never reopen a native path.
2. Implement additive public API/SDK, request/response bounds and independent
   conformance tests. Define scoped handles, call cancellation, native address
   codecs, streamed listing cursors, stat/root/storage capabilities and retained
   content revisions. Deliver ABI/header/documentation together.
3. Implement separate Windows/Linux native packages and plugin-owned tests. Raw
   names, long paths, replacement/change detection, links and cancellation need
   real OS fixtures; no mock test substitutes for native safety.
4. Reuse trusted loading in a host adapter. Validate all returned lengths, tags,
   handle ownership and capabilities; retain the library until its final source
   and buffer are released. Exercise missing package and cancellation cleanup.
5. Wire actual application native listing and F3 to the installed package.
   Preserve existing locale/projection/selection behavior and explicitly guard
   unavailable-provider actions. Package/build scripts must install the plugin
   independently. Test removal/restart without a hidden native fallback.
6. Run Windows/Linux targeted conformance and UI regressions, strict checks and
   native release acceptance. Document remaining mutation/watch/other-service
   extraction and generic tree/network migration as incomplete until implemented.

Review focus: malformed ABI replies, late cancellation, same-caption distinct
native names, provider removal with open viewers, and native path reuse after
capture. All new dependencies require the existing fee-free license policy.
