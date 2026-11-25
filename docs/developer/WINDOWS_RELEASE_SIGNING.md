# Windows release signing

The manual `.github/workflows/sign-windows-release.yml` workflow signs an existing
Windows portable ZIP on a **draft** GitHub release and creates a signed MSI from
the same signed payload. It does not compile the app or publish the release.
Microsoft Store/MSIX distribution is outside this workflow. Linux assets are unchanged.

The MSI installs for the current user, defaults to
`%LOCALAPPDATA%\Programs\Midnight Explorer`, and lets the user choose the destination
folder. There is no installation-scope selector, optional component selection or
extra installation option. The MSI builder uses Windows Installer COM and
`makecab.exe`; no WiX dependency or paid installer framework is introduced.
The initial dialog offers the destination path, Install and Cancel. There are no
custom progress/completion pages. Package versions must fit Windows Installer's
numeric `255.255.65535` bounds. The builder supports one embedded CAB with at most
2 GB of source files and Windows-1252 payload filenames; unsupported filenames
fail explicitly. The chosen installation folder supports Unicode. Standard MSI
file-version replacement rules apply, including retaining newer existing binaries.

Azure Artifact Signing (formerly Trusted Signing) provides the public-trust
Authenticode certificate. Signing identifies the publisher and protects the
signed bytes. It can reduce Windows warnings, but it cannot guarantee a warning-free
download or launch; EV certificates do not provide an instant SmartScreen bypass.
See Microsoft's [SmartScreen guidance](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation).

## External setup and costs

The repository integration does not create an Azure subscription, signing account,
identity validation, certificate profile, role assignment or GitHub environment.
It does not approve billing. An administrator must complete the setup below before
the first live run; no live signing result is claimed by the implementation.

Artifact Signing requires a **paid Azure subscription**; free, trial and sponsored
subscriptions are unsupported. Its service charges recur and account creation
starts billing, even before the first signature. Review the current monthly tier,
signature quota and overage costs before creating resources. This is the user's
requested external signing service, a narrow exception to the project's no-fee
dependency policy; it does not select a paid library license. See Microsoft's
[subscription requirements](https://learn.microsoft.com/en-us/azure/artifact-signing/faq)
and [pricing and billing terms](https://azure.microsoft.com/en-us/pricing/details/artifact-signing/).

Check **identity eligibility before paying**. As reviewed on 25 November 2026,
Public Trust supports organizations in the EU and other listed jurisdictions, but
individual developers only in the United States and Canada. A Czech individual
therefore cannot currently assume eligibility; an eligible, validated organization
is a distinct identity. Resource region availability does not waive identity
restrictions. Recheck the current [setup prerequisites and regions](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart).

1. In an eligible paid subscription, register `Microsoft.CodeSigning` and create
   an Artifact Signing account in a supported region. Record its regional HTTPS
   endpoint from Azure; the endpoint must match the account region.
2. Complete organization/individual identity validation in the Azure portal and
   create a **Public Trust** certificate profile. Private Trust and Public Trust
   Test profiles do not provide the public trust required by this release path.
   Record the complete certificate subject DN exactly as returned by the signing
   certificate, including field order, punctuation and spacing; do not substitute
   a display name or guessed `CN=` value.
3. Create a dedicated Microsoft Entra application and service principal for this
   repository's release signing. Grant only **Artifact Signing Certificate Profile
   Signer** at the specific certificate profile scope. Do not grant subscription
   Contributor/Owner or identity-verifier privileges to the workflow identity.
   See [profile-scoped role assignment](https://learn.microsoft.com/en-us/azure/artifact-signing/tutorial-assign-roles).
4. Add a federated identity credential to that Entra application with these exact
   values (case matters):

   | Field | Value |
   | --- | --- |
   | Issuer | `https://token.actions.githubusercontent.com` |
   | Audience | `api://AzureADTokenExchange` |
   | Subject | `repo:TomasJelinek/fm:environment:windows-release` |

   Use the environment subject, rather than a branch or pull-request subject.
   A repository rename, fork or environment rename requires an explicit trust
   update. See [Microsoft's OIDC instructions](https://github.com/Azure/artifact-signing-action/blob/main/docs/OIDC.md)
   and [federated credential fields](https://learn.microsoft.com/en-us/entra/workload-id/workload-identity-federation-create-trust).
5. Create the GitHub environment **`windows-release`**. Require a release reviewer,
   prevent self-review where available and restrict deployments to the repository's
   protected default branch. Review workflow/script changes before permitting a
   signing run. Protecting the environment matters because its name is part of the
   federated credential's signing authority.
6. Add these **environment variables** to `windows-release`. They are configuration
   identifiers, not client secrets; OIDC avoids storing a private key or Azure
   client secret in GitHub.

   | Variable | Required value |
   | --- | --- |
   | `AZURE_CLIENT_ID` | Dedicated Entra application's client ID |
   | `AZURE_TENANT_ID` | Entra tenant ID |
   | `AZURE_SUBSCRIPTION_ID` | Paid subscription containing the signing resource |
   | `AZURE_SIGNING_ENDPOINT` | Regional endpoint of the signing account |
   | `AZURE_SIGNING_ACCOUNT` | Artifact Signing account name |
   | `AZURE_SIGNING_CERTIFICATE_PROFILE` | Public Trust profile name |
   | `WINDOWS_SIGNING_PUBLISHER` | Exact full subject of the expected certificate |

## Signing and integrity contract

Run trusted workflow code from the protected default branch. The release ZIP is
untrusted data: preparation validates the expected archive name, target and source
revision, checks its initial inventories, safely extracts it, and constructs a
fixed binary allowlist. No executable or script from the release is run. Archive
manifests cannot authorize arbitrary extra binaries to be signed.

The signing set covers the app, project helpers and installed plugin binaries.
There is one deliberate upstream exception: the HTML viewer's EXE is an unsigned
CEF bootstrap, paired with the project's client DLL. The sandbox requires both
to have the **same primary signing certificate**, so they must be signed together.
An existing foreign/upstream signature must never be replaced; preparation and
verification fail closed for an incompatible pair. Signing only the project DLL
breaks this contract.

External runtime DLLs, other third-party files and all notices retain their bytes.
After Authenticode signing and timestamping, verification requires valid signatures,
the configured exact publisher subject and timestamps, and matching primary
certificates for the CEF pair. Then inventories are refreshed in this order:

1. HTML viewer `runtime-files.json` hashes, retaining `archive_sha256` as the
   provenance of the original upstream CEF archive.
2. `BUILD-INFO.json` file hashes and package fingerprints, covering the changed
   binaries and refreshed viewer inventory.
3. The final portable ZIP, followed by its per-asset SHA-256 checksum.

The MSI is then built from that signed, inventoried payload. The MSI itself receives
its own Authenticode signature and timestamp, is verified against the same exact
publisher, and receives a separate checksum after signing. A signed MSI cannot
substitute for signatures on its extracted executable/DLL payload.

A checksum for the unsigned ZIP is invalid after signing. Do not reuse it or edit
the package after generating the final checksum.

## Release operator procedure

1. Assemble and review the Windows portable ZIP through the existing release
   process. Upload it to a **draft** release with the intended tag/revision. Keep
   the original ZIP and checksum until replacement is verified.
2. In GitHub Actions, select **Sign Windows draft release** and run the workflow from
   the protected default branch. Supply the draft release tag (`vMAJOR.MINOR.BUILD`).
   The workflow requires the corresponding exact asset name
   `midnight-explorer-MAJOR.MINOR.BUILD-x86_64-pc-windows-msvc.zip` and creates the
   matching `.msi`. Check its target and source
   revision before approving the `windows-release` environment deployment.
3. Inspect the run's validation, signing and verification results. Any mismatch
   blocks normal replacement. Do not resolve failures by relaxing publisher,
   timestamp, manifest, archive or CEF-pair checks.
4. After success, download the replaced draft ZIP, new signed MSI and their matching
   per-asset checksums. Verify the checksums and Authenticode signatures on an updated Windows
   system; exercise the installed app and HTML viewer from a protected extraction
   directory. Test the MSI's default and chosen installation folders and uninstall
   behavior with a disposable Windows test account. Review the retained run evidence before publishing.
   Each asset has a `.sha256` sidecar and a `.signing.json` audit record. If the
   draft already contains `SHA256SUMS`, the workflow updates the Windows ZIP entry
   and adds or updates the MSI entry while retaining other assets' entries; it
   does not create a global manifest when none exists. Recovery/evidence artifacts
   remain available from the workflow run for 14 days.
5. Publish the draft release manually only after the final assets are accepted.
   This signing workflow never changes a draft into a public release and refuses
   an already-public release.

Replacing GitHub assets is not transactional. A failed upload can leave a draft
with a missing ZIP or mismatched checksum. Inspect the actual release assets after
any interrupted replacement; restore the retained originals or rerun the reviewed
signing operation against a consistent draft. Do not publish a partial result.

## Acceptance still requiring external setup

Local archive/manifest and workflow checks do not prove public certificate trust,
Azure permissions, GitHub environment protection, timestamp-service availability
or SmartScreen reputation. The first live acceptance needs the eligible validated
identity, configured Azure/GitHub resources, a successful protected workflow run,
downloaded-asset integrity/signature verification and installed Windows smoke
checks. Record actual commands, source revision, run URL and results in
[Active work](../ACTIVE_WORK.md); never infer these outcomes from the presence of
the workflow or its unit tests.
The [local verification receipt](../verification/2026-11-25-windows-release-signing.md)
records the fixture and native pre-install checks already performed.
