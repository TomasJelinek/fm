# Native macOS filesystem package implementation plan

> For agentic workers: execute this package-owned plan inline, following the
> test-driven-development and verification-before-completion skills. The parent
> owns shared API/host integration; do not change it from this package task.

**Goal:** Supply an independent read-only Darwin Browse/Read/Stat/Volumes/Storage
provider against the existing public filesystem ABI and SDK.

**Architecture:** `NativeFilesystem<MacOs>` owns the standard bounded ABI lifecycle.
Darwin files implement descriptor-relative lookup, enumeration, captured readers
and mounted-volume queries. Portable policy helpers validate native text,
capacity and bounded mount-snapshot retry. Only the public API/SDK and pinned libc
are dependencies; no application-private crates are shared.

**Spec:** Parent task and `../../docs/superpowers/specs/2026-11-07-native-filesystem-plugin-design.md`.

**Constraints:** Raw Unix filename bytes remain identities; Unicode display is
separate. No application path-length limit, content reads of links/devices/FIFOs,
implicit writes, credentials or elevation. Local storage remains Unknown without
positive SSD proof. Output belongs under repository `_build/`, caches under `.tools/`.
No Cargo execution until coordinated with the parent. No commits or app changes.

Review focuses on replaced file names, special-file substitution, raw bytes and
directory-link navigation, trailing slash/dot semantics, and mount-table churn.
Darwin cross-checks do not establish runtime, dynamic-loading or application support.

- [x] Write failing portable tests for capacity overflow/independent fields, raw
  mounted-name bounds and retrying saturated mount snapshots; implement policy.
- [x] Write package-owned native ABI tests for paged listing/raw names, selected
  content, replacement/edit rejection, links/FIFOs, cancellation, long paths and
  volume reporting. Implement adapter, descriptor boundary and captured reader.
- [x] Add real dylib-load conformance test and metadata/notices for libc 0.2.191.
- [x] Run available host tests and Darwin source/test cross-checks, format and
  strict lints. Record exact evidence and native qualification still required.

Native execution is pending. The five portable behavior tests were observed
failing against initial stubs before implementation. Darwin tests were written
before the native implementation and initially failed to type-check because the
package entry was absent; no native runtime red/green result is claimed.

Files: `src/lib.rs`, `src/policy.rs`, `src/adapter.rs`, `src/system.rs`,
`src/directory.rs`, `src/content.rs`, `src/volumes.rs`; package integration tests
and fixtures under `tests/`; independent Cargo manifest/lock, `plugin.toml`,
README and THIRD-PARTY notices. No unsupported macOS version is advertised.
