# Native filesystem application cutover

This integrates the [native host](2026-11-08-native-filesystem-host.md) and
[ABI1.1 volume/storage parity](2026-11-08-native-volume-parity.md) into production
composition. Panel listing, metadata, folder-size traversal, capacity/storage
queries and native F3 content now use `InstalledProvider`. There is no built-in
fallback for these routes when the installed native provider is unavailable.
Desktop integration, watches, mutations, transfers and other consumers retain
their explicit platform services pending the generic-provider migration.

`build.bat` and `build.sh` build the independent native package before viewers.
Release validation requires exactly one compatible native-address provider and
accepts native libraries alongside archive workers. CI builds each native plugin
with public API sources only, then tests it in a protected application installation.
The helper stages Linux package inputs and renames completed files, preserving
the old library inode mapped by a running app. This is atomic per file, not a
transaction across the whole package.

## Review-driven corrections

- The host initially inherited unsupported rich-volume defaults. Causal tests
  now cover label and rich-field mapping, independent available/free/used space,
  cancellation and missing optional capabilities.
- Independent review found valid portable UTF-8 volume text was rejected by the
  native-name decoder. A failing Unicode label/filesystem/source test preceded
  the volume-only correction. Raw native text remains lossless, foreign native
  name encodings remain rejected, and malformed UTF-8/NULs fail explicitly.
- Linux rebuilds previously copied over a live mapped library inode. Two causal
  tests now prove that an open old inode retains its bytes after publication and
  that input-staging failure preserves all old files and unrelated entries.
- Release validation previously accepted versions beyond the host's 64-byte
  UTF-8 limit. ASCII and multibyte boundary tests now match the native host.
- Native screenshot inspection found provider timestamps fell back to UTC because
  old adapters, rather than the engine, supplied localized display strings.
  Backend locale enrichment now preserves supplied localization and formats only
  missing display fields before streamed entry/stat batches. The Windows gate
  proves correct worker placement and zero extra stats for complete entries.
  Windows and Linux native locale and engine regressions pass, including preserved
  provider localization. The final Windows screenshot shows local date/time.
- The app's Windows test harness inherited the GUI subsystem, so direct shell
  invocation did not wait for its exit. Test builds now use the console subsystem;
  normal application builds retain the GUI subsystem.
- The first actual absent-provider viewer test rejected a generic path error with
  no filesystem context. The app capture boundary now identifies the native
  filesystem while retaining the exact underlying failure. Tests require that
  diagnostic and reject any content frame from an unavailable provider.
- The all-format test exposed delayed metadata changes on newly authored Windows
  BMP/PDF fixtures. A separate direct-ABI probe reproduced the change without an
  app running: scanner-owned extended attributes were added after roughly four
  and six seconds, while identity, length, write time and SHA-256 stayed identical.
  Old readers correctly rejected the changed revision. The test now observes a
  minimum fixture age plus quiet revisions and checks its authored bytes before
  capture. No production revision check or source-change refusal was weakened.
  The bounded setup wait is test-only, not viewer startup work.

## Verification scope

Artifacts are under `_build/goal/native-filesystem-cutover/`. Host gates exercise
real independent DLL/SO packages, Unicode and raw paths, captured revisions,
reader quotas, rich volume details, missing/duplicate/incompatible providers and
the installed service's no-fallback behavior. The final Windows and Linux host
gates each pass 23 unit tests and two actual module tests, plus strict all-target
checks. Both application debug builds and all-target strict gates pass.

App acceptance uses authored fixtures and smoke-only preferences, not the user's
normal profile. Tests run the actual app service with native F3/NextFile and an
absent provider. The all-package test additionally selects Source, Markdown,
image and PDF plugins, checks rendered content and retains manual PDF zoom while
changing pages. These backend integration checks do not by themselves prove
every keyboard gesture or compositor frame time; viewer controls and measured
renderer latency have their separate reports.

Windows passes both installed app-service viewer tests, including Source search
and coordinates, rendered Markdown, decoded image pixels and actual PDF page
pixels at retained manual zoom. Native and explicit software app startup both
pass with screenshot output; the localized panel date/time and complete lower
toolbar were visually checked. A separate protected copy without its native
package rejects startup rather than silently reading files. The normal preferences
hash remained unchanged throughout acceptance.

Linux initially passed the protected installed basic F3/NextFile/absent-provider
test and headless main-app volume/two-pane navigation smoke. Missing-provider
startup also fails explicitly. The subsequent
[Linux all-format qualification](2026-11-08-linux-viewer-cutover.md) rebuilds all
five independent packages and the combined debug app, passes both installed
app-service tests and headless navigation, and records final hashes and limits.
CI is configured to run both app-service tests with all independent packages;
the local evidence is not a remote CI success claim.

The registered Windows debug executable SHA-256 is
`CF155CD7E00BB0C0B019D2A5D8644442A83410BA7F3D370DAC256A56382C02DE`.
Its independent native DLL is the final package recorded in the volume-parity
report. Native/runtime graphs were refreshed for Windows/Linux; inventory counts
remain 490 package/version/source records, 458 names and 60 direct records.
The initial Linux debug executable SHA-256 was
`BD76F0423710760DBF0774B806725C78A2F2FAB76E73B5A408D77FD14B2CF54F`.
The later combined Linux app and all-viewer runtime are identified in the report
linked above.
The 41-case packaging/helper suite succeeds: Windows has 36 passed and five
platform skips; Linux has 40 passed and one skip. Rustfmt and whitespace checks pass;
all 1,219 authored source files remain within the 100,000-byte limit.
No remote CI result or release qualification is claimed here. Generic provider
identities, watches, mutations and the other migration consumers remain open.
