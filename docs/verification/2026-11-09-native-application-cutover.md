# Native application filesystem cutover

This candidate is based on `18a1661e5202b8213cc6eb7d5ecf2c5b37912efd`.
It was first reconstructed on `fab0d5d30761b9b174cd04d3d7015b6b1616f18f`, then
rebased onto the separately committed theme changes without altering them.
It connects the already committed native filesystem host to the application.
It excludes the concurrent theme, startup, renderer, inline-tree, archive-format
selection, file-picker, transfer and checksum changes.

## Included behavior

Panel browsing, metadata, folder-size traversal and volume/storage queries use
`InstalledProvider`. Native F3/NextFile, comparison and thumbnail content use the
same injected content authority. An unavailable provider reports failure; these
routes do not retry through a built-in native opener. Existing desktop, watch,
mutation, transfer and other explicitly injected platform services remain.

New archive opens and restored archive tabs use the engine's `ArchiveCapture`
callback. The native host validates saved current or supported legacy revisions
before the archive worker starts. `Mount::open_captured` retains the exact native
locator separately from the content lease; the decoder still receives bytes only.
The committed suffix-selection and saved-address schemas are unchanged.

Native and archive packages share the installation directory. Archive discovery
leaves native package validation to the native host and counts every directory
against its scan budget, rejecting overflow rather than selecting a partial
catalog. The existing public plugin ABI, API/SDK sources and independently built
plugin packages are unchanged.

Comparison capture checks cancellation before and after each provider call.
Thumbnail capture receives the admitted job's live cancellation flag and retains
one immutable source per visible token. The existing operation proof, checksum,
source-revision and worker-admission requirements are unchanged.

The engine fills missing localized timestamp fields before entry/stat publication
and preserves localization already supplied by a provider. Windows and Linux
locale services expose their existing native formatter through `format_datetime`.
No extra stat is required for complete listing metadata.

## Dependencies and notices

The application adds local dependencies on the existing native host and, for
tests, plugin host. Third-party declarations, pinned versions, license choices
and selected notices are unchanged. The complete workspace inventory already
includes both host crates and their dependencies. Fresh target and runtime/build
dependency captures passed against that inventory: Windows selected 327 and 324
external packages respectively; Linux selected 398 and 396. All selected pins,
declared licenses and target scopes are covered. All 609 lockfile package identities
are unchanged. Release packaging distributes the selected upstream notices.

## Verification status

The candidate was reconstructed in `_build/qa/native-cutover/source` without
modifying the root source or index. Selectively recomposed files require fresh
checks; previous whole-application tests and deployed binaries do not qualify
this candidate. Existing native API/SDK/package sources remain at the committed,
separately qualified baseline.

Focused tests cover comparison capture/refusal/cancellation, thumbnail source
revision/cancellation, pathless archive sources and locator retention, shared
package discovery, injected archive refusal/restore revision, locale enrichment,
and actual installed app-service routes. App tests use owned fixtures and a
protected runtime. CI stages the app harness and filesystem smoke executable in
that runtime before execution.

Fresh qualification on the final `18a1661` composition passed on Windows x64 and
Linux x86-64. Counts below are separate gates; the installed filesystem gate also
runs the five filesystem unit tests included in the backend gate.

| Gate | Windows | Linux |
| --- | --- | --- |
| Comparison, engine, filesystem and thumbnail unit tests | 357 passed, 2 ignored | 346 passed, 2 ignored |
| Engine locale integration | 2 passed | 2 passed |
| Platform locale tests | 4 passed | 4 passed |
| Installed filesystem host, discovery and workers | 10 passed | 10 passed |
| Ordinary application tests | 17 passed, 9 ignored | 17 passed, 8 ignored |
| Installed native content, archives, comparison and thumbnails | 6 passed | 6 passed |
| Installed F3/NextFile and Source/Markdown/BMP/PDF viewers | 2 passed | 2 passed |
| All-target Clippy for changed packages, `-D warnings` | passed | passed |
| Normal debug application build and filesystem startup smoke | passed | passed |
| Startup with the native package absent | refused, exit 1 | refused, exit 1 |

The eight installed application tests were run explicitly after the ordinary
suite. The remaining Windows-only optional GUI fixture and debug CPU benchmark
were not run. Windows UI startup was also exercised and its screenshot inspected;
preferences remained byte-identical. No remote CI or release-build qualification
is claimed. Formatting and the 100,000-byte source limit passed (1,129 authored
files). Linux final audit checked 1,834 unchanged qualification inputs, all staged
artifact hashes and absence of surviving candidate workers.

The UI screenshot exposed a missing integration dependency: both platform locale
services inherited a trait default returning no timestamp. Regression tests failed
on both platforms before the small formatter overrides and passed afterward.
Fixtures were recomposed against the committed comparison, viewer and archive
interfaces instead of importing newer unrelated features. The copied Windows
runtime initially lacked the deployment's explicit read/execute grants on four
FFmpeg DLLs; reproducing those code-file-only grants made the real video tests
pass. Linux private test data used a protected POSIX filesystem, since the mounted
source drive cannot satisfy the private-directory mode check. Failure logs are
retained alongside the successful reruns.

Evidence is under `_build/qa/native-cutover/` in the authoring workspace, with Linux
logs in `linux/` and fresh dependency captures in `dependencies/`. The tested input
manifest is `source-qualified-inputs.json` (SHA-256
`e75b21ab99f56e4614ce1893f079b330752a071dd36c98d0cec707bb1cc1e318`).
Only this report and the inventory status were updated after qualification.

The Windows application SHA-256 is
`32258b8e7430b71bc564d586f8eabebcd6ebb9b0190b97dd68dcb86fef1a8b24`;
the Linux application SHA-256 is
`2aac2fc1b4e66154cf7f21f97960527f4668dfff06bc084a6ac7c6eede33ebf7`.
These are private test artifacts, not replacements for the user's installed app.
Independent review found no introduced actionable defect. This report accompanies
the scoped native application cutover; later working-tree changes have separate
qualification. A final pre-commit audit verified all 1,834 frozen inputs unchanged.
