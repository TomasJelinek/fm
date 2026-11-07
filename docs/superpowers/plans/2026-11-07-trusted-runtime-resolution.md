# Trusted Runtime Resolution Implementation Plan

> Implement with superpowers:executing-plans and independently delegated viewer
> verification. The user has authorized implementation and parallel work.

**Goal:** Fix direct shared-build startup without weakening plugin security, and
prove the actual viewers work.

**Architecture:** A shared backend resolver selects either the trusted executable
location or an explicitly registered, matching private installation. Existing
plugin hosts retain all package validation and isolation.

**Tech stack:** Rust, existing SHA-256 crate, Win32 protected ACLs, PowerShell,
existing Slint/native viewer test harnesses.

**Spec:** `../specs/2026-11-07-trusted-runtime-resolution.md`.

## Constraints and review focus

- Non-UI I/O stays on backend workers; Windows/Linux files stay separate.
- No trust bypass, untrusted startup copying or permission changes to ancestors.
- Runtime records are private, bounded, lossless and tied to the exact host hash.
- Preserve absent optional plugins, portable installations and built-in text diff.
- QA must not read/write real settings through a normal app launch.

## Tasks

- [x] Reproduce the exact shared release failure with isolated smoke settings;
  inspect actual inherited ACLs and compare the working private installation.
- [x] Add `midnight_plugin_host::installed_executable() -> Result<PathBuf, Error>`
  with platform-specific resolver files and security regression tests.
- [x] Extend `scripts/deploy-private.ps1` with explicit registration and an
  isolated test registry option; add `scripts/tests/test-runtime-registration.ps1`
  with matching digest/path, atomic update and unsafe-path refusal checks.
- [x] Wire viewer catalog/service, thumbnail service, archives and comparator to
  the shared resolver. Keep explicit-root APIs and optional text diff semantics.
- [x] Complete Windows build helper deployment/registration and update run docs,
  security contract, developer API notes and dependency inventory.
- [x] Verify all five viewer types using real installed packages and native UI;
  fix observed viewer defects with focused regressions.
- [x] Run affected Windows/Linux tests, strict Clippy and format/size checks;
  build/deploy/register release and prove direct shared-path startup and viewers.
- [x] Review the verified changes and document limitations. Commit/push is recorded
  by the repository history and final delivery message.
