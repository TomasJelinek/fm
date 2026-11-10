# Archive containers through native filesystem capture

This qualification extends the native filesystem cutover to new archive opens
and restored saved archive tabs. It is part of VFS-01, not completion of the full
provider migration, nested archives or generic writes.

## Behavior and saved addresses

Production app composition injects `native_content::archive` into the engine.
Both navigation routes use `InstalledProvider::capture_matching`, with a saved
revision supplied on restore. Missing/rejected native providers fail closed even
when a readable file exists at the supplied path. The archive decoder remains an
independent, restricted worker and receives only bounded source bytes.

`Mount::open_captured` retains the exact native container `PathBuf` separately
from its pathless `CapturedFile`. It does not open the locator or construct a path
from a display name. Cancellation, locator validation, source length/revision
budgets and source validation precede worker startup. Two-worker admission and
existing request deadlines remain intact. Listing/read exchanges continue to
revalidate captured source identity and revision.

The old Windows `windows-file-v1` and current `native.windows.read.v1` encoders
have the same 48-byte payload: volume, 128-bit file ID, length, write time and change
time. Linux `linux-file-v1` and `native.linux.read.v1` have the same 56-byte payload:
device, inode, length, mtime/ns and ctime/ns. Separate platform adapters recognize
only their exact v1 prefixes and complete payload lengths. Every metadata byte
must agree. Unknown/foreign/truncated/extended tokens fail; prefix replacement
alone never proves identity. Successful restore records the current token.

## Windows validation

- Native host: 34 tests passed, including real InstalledProvider/CaptureCache
  composition through a test ABI transport, both platform token codecs, missing
  plugin and capture cancellation. Four intended RED failures preceded the fix.
- Archive host: seven ordinary tests passed; the actual pathless-content worker
  test separately passed in a protected installation. The latter initially failed
  by trying to open the intentionally absent Unicode locator. The final test
  proves listing/read from supplied bytes and rejection after source mutation.
- Engine: 448 unit tests passed, one unrelated opt-in test ignored. Two capture
  regressions first failed: ignoring an injected refusal and ignoring cancellation.
- Actual app engine: both protected-runtime tests passed. They open a real gzip
  through the native provider, restore its legacy address, read the member, reject
  source mutation, and reject new/restored opens with a missing native package.
- Targeted all-target strict Clippy passed for native host, archive host and app;
  the engine dependency retains 39 pre-existing unused-code warnings from unfinished
  provider/tree wiring. This is not a clean full-workspace/release qualification.
- A read-only production-route benchmark uses the same app capture factory and
  engine navigation as ordinary archive opening. It does not modify the supplied
  archive. Package discovery finishes before the measured requests.

Independent source review found no introduced security/correctness defect in
these routes. Native ReadAt/ValidateRead callbacks cannot be cancelled once entered;
this inherited limitation is unchanged, and no claim of interruptible OS I/O is
made. Capture/dispatch cancellation checks keep the UI independent of those calls.

Windows evidence is under `_build/qa/archive-native-capture/`; native codec/capture
RED/GREEN/Clippy evidence is under `_build/qa/archive-native/`. Relevant app tests
live in `midnight-explorer/src/native_archive_tests.rs`. They require a protected
native/archive installation and owned `restore 文档.gz`, copied from the existing
host `hello.gz` fixture before running. The successful-restore test deliberately
changes this owned fixture. The separate timing test requires
`MIDNIGHT_ARCHIVE_TEST_SOURCE` and performs reads only.

## Performance gate and deployment

The supplied SDK 7z archive is 19,818,839 bytes. The existing optimized archive
worker opened it through the old direct-capture host in 1.713 s on first launch,
then 81.6/86.5 ms. Root listing/relisting took ~1 ms. Temporary diagnostic timing
then measured 133/82/78 ms opens, with spawn+ready taking 74/27/23 ms. The one-off
cold delay did not recur; its cause is not claimed isolated or fixed.

The first production native-provider probe measured 568/433/406 ms Open/List.
This exposed redundant native validations from nested captured-content wrappers
in the reader cache. The cache now retains raw native content leases beneath one
public CapturedFile wrapper. Three resident chunks require 9 explicit ValidateRead
calls instead of 18, with all 3 data reads unchanged. SDK before/after validation,
outer captured-source validation, exact length/revision checks on reopen and the
16-reader cache bound remain. Four new tests cover that request budget, resident
mutation, mutation during read and identity/length mismatch after eviction.

The resulting native-host suite passes 38 tests and strict Clippy. Independent
review found no introduced ownership/integrity issue. The final hash-verified
Windows app harness passes all seven installed archive/comparison/image/video/
Unicode-source tests in 3.18 s; archive Open/List measured 315/297/306 ms and relist
~1.3 ms. Seventeen ordinary app tests pass; 14 opt-in tests remain separately gated.
These measurements still exceed the earlier direct-capture host probe. Deployment
is withheld while the remaining repeated validations are investigated. No
containment, revision or checksum requirement is waived for performance.

The initial attempted final integration run accidentally copied the previous
harness while Cargo was still linking. Its output is retained as
`native-prior-binary.log` and does not qualify this optimization. After Cargo
completed, the actual final executable was copied, its SHA-256 was compared with
the source, fixtures were regenerated and all seven tests were repeated. That
authoritative output is `native-final.log`, with `test-harness.json` recording
the exact artifact.

The Linux baseline passes all 38 native-host unit tests, both real native
integration tests, all ten archive-host tests (including ignored real-worker
gates), and both actual app archive tests: 52 tests total. Targeted strict Clippy
passed with the same 39 existing engine dependency warnings. The final audit
confirmed 254 unchanged source inputs, matching staged artifact hashes, protected
runtime permissions and no remaining qualification workers. Evidence is under
`_build/goal/archive-native-linux/`, with its baseline preserved separately before
the subsequent checked-read optimization.

## Checked-read follow-up

The host now dispatches direct broker and retained-reader requests through one
bounded checked-read helper. Generic sources retain before/after validation.
The native adapter checks the original immutable lease metadata and uses the
SDK's already validated ReadAt, avoiding duplicate ValidateRead calls. The cache
forwards the original expectations across reopen. EOF and explicit revalidation
still invoke the live validator. No plugin ABI, dependency or binary changed.
See [the content contract](../developer/CONTENT_READS.md).

Host RED tests first demonstrated the ignored hook and redundant EOF validations.
Native RED tests demonstrated six duplicate validations for three reads after
the host change. The final native trace has zero separate ValidateRead calls and
three ReadAt calls, each still validated before/after by the native SDK.

Windows qualification under `_build/qa/checked-read/`:

- 52 ordinary plugin-host tests and four actual basic-viewer plugin tests pass.
  The separate wrong-ABI fixture test was not run in this slice.
- 42 native-host unit tests and two installed native-plugin tests pass.
- All ten archive-host tests pass, including actual restricted-worker tests.
- 17 ordinary app tests and seven native archive/comparison/thumbnail scenarios
  pass. The latter include a read-only benchmark of the supplied SDK 7z archive.
- Targeted strict Clippy and the debug build pass, with the existing 39 engine
  dependency warnings. Changed source files pass rustfmt; workspace fmt still
  reports an unrelated existing expression in `midnight-core/src/entry.rs`.
- Independent review found no introduced correctness/security defect in the
  checked-read dispatch, native adapters, cache reopening or lease disposal.

Production Open/List now measures **175.9 / 149.3 / 167.8 ms**, compared with
315 / 297 / 306 ms after the raw-cache change and 568 / 433 / 406 ms before it.
Relisting measures 1.2-4.0 ms. This is a measured reduction in the native-provider
route, not proof of equality with the earlier 82-87 ms direct-capture probe or a
fix for the unreproduced cold-start delay. Source SHA-256 before/after confirms
the supplied user archive was unchanged. The new app executable SHA-256 is
`77a0dc6719b7cbe1ac0e06f3d4f64f756ec2c84444fde59233290f76a41007dc`.

The initial broad plugin-host invocation lacked its protected fixture directory;
the authoritative corrected run is `host-protected.log`. The initial standalone
worker invocation named a nonexistent installed subdirectory; the corrected
verified-path run is `windows-worker-final.log`. Neither setup failure is hidden
or counted as passing. App harness staging happened only after Cargo completed,
and the source/staged harness hashes were verified.

Windows deployment is complete in
`C:\Users\tjelinek\MidnightExplorer-20261109-064239-a020dd9d`.
The installed archive worker hash matches the qualified worker. Protected runtime
registration matches the app hash above, and the desktop shortcut targets this
new installation. The previous installation remains available.

Both headless and native UI startup smoke tests passed from `_build/debug/me.exe`
through registered runtime discovery. The captured frame was inspected: both
panels, toolbars, command line and bottom buttons render without a startup error.
Saved preferences are byte-identical before/after the tests. Evidence:
`deploy.log`, `startup.json`, `startup.png`, `smoke.*.log` and `ui-smoke.*.log`.

Linux checked-read qualification completed successfully on Ubuntu 24.04:
100 tests passed (40 ordinary plugin-host tests, 42 native-host unit tests plus
two installed-provider tests, ten archive-host tests and six actual app
archive/comparison/thumbnail tests). Five optional plugin-host fixture tests were
not run in this Linux slice. Targeted strict Clippy passed with the same 39 engine
dependency warnings.

The final audit verified 1,250 unchanged workspace/API inputs, 120 staged files,
matching artifact hashes, protected runtime permissions and no surviving test
workers or staging temporaries. The app harness SHA-256 is
`6e802d8744814784e88008f42c026e1e32b51739e1a47d80239f01428a0b027f`.
Native and archive binaries retain their baseline hashes; no plugin rebuild was
needed. The reconstructed protected Linux runtime and regenerated owned fixtures
were checked independently of the earlier raw-cache baseline. Final evidence is
`_build/goal/checked-read-linux/results.json` and `final-audit.log`.
