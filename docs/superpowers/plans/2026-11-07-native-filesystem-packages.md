# Native Filesystem Packages Implementation Plan

> Use superpowers:executing-plans inline. Parent approved this contract and
> implementation in the shared workspace; no subagents, commits or root manifest
> edits by this implementer. Coordinate Cargo with root/other owners.

**Goal:** Real Windows/Linux native listing and selected-file F3 through optional
independently built filesystem packages, with no hidden host-native fallback.

**Architecture:** A public std-only synchronous C ABI and strict binary model
codec, a safe author SDK owning FFI/buffer lifetimes, and separate per-OS native
libraries. Root supplies trusted dynamic loading, FileSystem/provider adapters,
viewer capture injection and production composition.

**Spec:** [accepted ABI/design](../specs/2026-11-07-native-filesystem-plugin-design.md).

**Stack:** Rust1.99/edition2024, std, existing pinned windows-sys0.61.2 and
libc0.2.191 in native projects. No new third-party library.

**Current evidence:** Public workspace 29 tests + cancellation doctest on both
OSes; actual Windows package six and Linux package eight tests, strict all-target
checks and 17-workspace inventory are qualified in
[the package report](../../verification/2026-11-07-native-filesystem-packages.md).
SDK resource responses now use an owned `Reply` publication transaction, without
wire/C ABI changes. Parent-owned host cutover and optional-package absence/F3
acceptance remain pending; the original task checklists below are sequencing
guidance rather than a claim of end-to-end completion.

## Constraints and review focus

- Released archive worker protocol1 and viewer C ABI remain unchanged.
- All FFI pointers are borrowed or explicitly leased; callback fields are Option
  unsafe extern C pointers. Cancellation cannot escape safe CallContext's borrow.
- No Rust allocator/layout, app/core/engine/UI dependency or private plugin pool
  crosses the boundary. Native packages are trusted code, without sandbox claims.
- Preserve raw UTF16/Unix names/paths and native provenance; never parse captions.
  Listing is live; content alone has a revision-validated retained handle.
- First bounded page before full traversal, missing metadata and failed/partial
  listing not success, no buffer/handle reuse/foreign release, no early DLL/permit
  release, source edits/replacements detected, no symlink/FIFO content read.
- Output `_build/goal/native-plugin`, separate Windows/Linux targets; no native
  app or live preference write. Per-OS code/tests remain separate.

## 1. Public API, wire and C layout

Create `filesystem-api/midnight-filesystem-plugin-api/{Cargo.toml,src/lib.rs,
src/abi.rs,src/model.rs,src/wire.rs,src/wire/encode.rs,src/wire/decode.rs}` and
`tests/{codec.rs,layout.rs}`; add only this member to the independent API workspace.
Create `filesystem-api/include/midnight_filesystem.h` and protocol documentation.

Produces typed Address/Node/Metadata/Request/Response/Failure/Operation, exact v1
constants, ABI Api/Config/Call/Cancellation/Buffer and encode/decode functions.

- [ ] Behavioral RED using encoder/decoder stubs: literal field fixture,
  roundtrip raw surrogate/invalid bytes, all request/response variants, missing
  metadata, malformed tags/versions/bools/trailing bytes and length-before-alloc.
- [ ] Implement bounded strict LE codec with schema prefix and borrowed cursor.
  Follow the design's exact field order, bounds, optional flags and EOF semantics.
- [ ] Layout/default table tests, caller capacity/reserved/header/unknown callback
  tests; publish C assertions for supported64-bit targets.
- [ ] GREEN standalone API tests, rustfmt and strict Clippy, no host dependencies.

## 2. Safe author SDK and owner lifecycle

Create `filesystem-api/midnight-filesystem-plugin-sdk/{Cargo.toml,src/lib.rs,
src/runtime.rs,src/buffers.rs,src/ffi.rs,src/ffi_tests.rs}`. Add this workspace
member; depend only on the new public API. No shared viewer SDK modifications.

Produces `Filesystem::{create,handle}`, borrowed `CallContext`, descriptor export
macro, module-wide checked handle allocator, panic guards and instance buffers.

- [ ] RED through actual function pointers: create/call/release/destroy, wrong
  owner and double release, zero descriptor/defaults, rejected budget/admission,
  concurrent call cleanup and destroy while retained buffer/call, handler panic.
- [ ] Implement short registry critical sections, owned bounded reply allocations,
  exact caller-owned pointer/length validation and per-instance active-call guard.
  Native callbacks remain in the audited FFI module; safe context cannot escape.
- [ ] Include compilation-fail documentation for retaining the borrowed context.
  Cleanup remains callable after operation cancellation.
- [ ] GREEN API/SDK tests and strict Clippy; unchanged archive API tests still run.

## 3. Windows independent native package

Create standalone `plugins/midnight-filesystem-native-windows` with public-only
Cargo dependencies, manifest, notices, small focused path/node/listing/content/
storage modules and separate native tests. No private host/platform crate import.

- [ ] Audit exact pinned source/licenses/features before manifest changes and
  notify root dependency owner. Add complete package notices and inventory rows.
- [ ] RED native fixtures: streaming first page, raw UTF16 and >260 path, distinct
  logical node/kind ownership, cancellation, independent reads, exact EOF and
  source revision changes, special/link refusal and explicit capability absence.
- [ ] Implement std enumeration/no-follow metadata and regular-file handle capture
  using raw Windows APIs only where identity/stamps/storage require them. Include
  FileId/write/change time in version. Windows DLL uses SDK-exported v1 entry.
- [ ] Build the actual DLL and call its descriptor through an OS loader in a
  standalone conformance test; no direct Rust handler shortcut as sole proof.
- [ ] GREEN package tests/strict checks. Root installs/trusts this package and
  connects actual FileSystem/F3; no temporary static fallback in production.

## 4. Linux independent native package

Create standalone `plugins/midnight-filesystem-native-linux` with public-only API/
SDK + exact libc dependency, manifest/notices and per-OS native implementation.

- [ ] RED invalid UTF8 names, raw long paths, streaming cancellation, symlink/FIFO
  refusal, independent positioned readers, source edit/replacement/EOF and quota.
- [ ] Implement descriptor-relative access where whole-path syscall limits would
  otherwise prevent the >4096 fixture; readdir is incremental, no full collection.
  Source snapshots include dev/inode/mtime/ctime and nanoseconds.
- [ ] Implement mount/volume discovery and conservative storage query; Unsupported
  never calls a host service. Missing flags stay missing.
- [ ] Actual dlopen/dlsym descriptor call test + author-level native tests GREEN;
  strict Clippy and notices/license inventory updated. No paid/extra library.

## 5. Host handoff and integrated verification (root owner)

- [ ] Publish frozen API/manifest/decoder handoff and actual library artifact paths.
  Root adds its host dependency lines; preserve original archive routing.
- [ ] Real plugin Browse -> captured selected file -> injected F3 source -> exact
  content, with host-native spies proving no reopen/fallback. Verify absent and
  revoked package, stale handles, cancellation/outbox admission and both panes.
- [ ] Run focused existing viewer/archive tests and independent plugin/API/SDK
  tests on Windows/Linux. Root owns release packaging/native smoke and commit.
- [ ] Durable developer/verification docs distinguish Browse/Read production
  cutover from remaining mutation/transfer/Find/tree/provider migrations. Report
  macOS/aarch64 native validation dependencies accurately.
