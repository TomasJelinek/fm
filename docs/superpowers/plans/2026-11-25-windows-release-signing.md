# Windows release signing and per-user MSI

**Goal:** Implement the user's requested trusted signing of GitHub release ZIPs
and an MSI that defaults to the active user's profile with a configurable folder.

**Architecture:** A manually dispatched workflow processes a complete Windows ZIP
on a draft release. Trusted default-branch scripts validate it, Azure signs an
explicit binary list, and native Windows verification gates regenerated manifests
and ZIP creation. A per-user MSI is built from that same signed payload and signed
and verified separately. The workflow replaces draft assets; publication remains
manual. No signing credentials, subscriptions, resources or commits are created.

**Tech stack:** Python standard library, PowerShell, Windows Installer COM/makecab,
GitHub Actions, Azure Artifact Signing via OIDC.

## Assignments and acceptance

- Root owns `scripts/windows-release-signing.py`,
  `scripts/verify-windows-release.ps1`, and
  `scripts/tests/test_windows_release_signing.py`. Test safe extraction, revision
  and inventory checks, explicit signing scope, rejection of unrelated mutations,
  CEF then outer inventory refresh, and independent native signature refusal.
- Build specialist owns `.github/workflows/sign-windows-release.yml`,
  `scripts/github-windows-release.ps1` and its tests. Require draft release,
  protected environment, trusted workflow code, least-privilege OIDC, timestamps,
  verified ZIP/MSI and refreshed checksums. Never publish implicitly.
- Architect owns `scripts/build-windows-msi.ps1`, `scripts/windows-msi.py` and MSI
  tests. Default to LocalAppData/Programs/Midnight Explorer; allow folder change,
  no scope/features choices, current-user registration and uninstall. Verify a
  real fixture MSI under `_build/`; user-profile install testing is not authorized.
- Documentation owns the signing runbook, scoped dependency/service records,
  README link and active-work receipt. Record only actually verified outcomes.

## Validation and limits

Run targeted release/MSI tests and direct packaging boundaries, PowerShell parse
checks and real MSI database inspection. No Cargo changes or competing build
leases. Get a fresh independent review before reporting completion. Azure identity
verification, protected environment configuration and a real signing run require
external setup; local fixtures cannot establish public trust or SmartScreen
reputation. User requested notice before any commit; leave all changes uncommitted.
