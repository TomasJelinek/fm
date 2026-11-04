# Stage 1 verification — 4 November 2026

The first milestone is a runnable, read-only dual-pane local filesystem browser.
Copying, remote protocols and destructive operations are not implemented yet.

## Executed checks

| Check | Observed result |
| --- | --- |
| Windows `cargo test --workspace --offline --locked` | 63 passed; zero failures or ignored tests after platform/style separation |
| Windows `cargo fmt --all -- --check` | Passed |
| Windows strict workspace Clippy, all targets, `-D warnings` | Passed |
| Windows `cargo build -p fm-app --offline --locked` | Passed; `target/debug/fm-app.exe` |
| Real filesystem headless application smoke | Passed for project and Salamander source directories |
| Native default FemtoVG-WGPU window/focus/snapshot smoke | Passed |
| Ubuntu 24.04 WSL core/platform/engine/common/Linux-adapter tests | 32 passed; zero failures after platform separation |
| Ubuntu 24.04 WSL full workspace tests | 60 passed; zero failures, including UI and actual-focus integration without a display |
| Ubuntu 24.04 WSL `cargo check -p fm-ui` | Passed |
| Ubuntu 24.04 WSL `cargo check -p fm-app` | Passed |
| Ubuntu 24.04 WSL actual `fm-app` executable build | Passed; runtime libraries resolve from system paths without the local build prefix |
| Ubuntu 24.04 WSL actual executable headless smoke | Passed: volumes and both real directories loaded without build-prefix runtime overrides |
| Ubuntu 24.04 WSL strict workspace/all-target Clippy | Passed |
| Windows/Linux source-size checks | 39 handwritten files pass the 100,000-byte cap; exact boundary tested |

The final platform/style refactor was independently reviewed and approved.
Cargo target dependency trees confirm that the Windows app includes the Windows
adapter and Linux includes the Linux adapter, with no opposite native adapter.
The common crate contains only portable code; OS implementations/tests are split.

The native screenshot at [stage-1-windows.png](../screenshots/stage-1-windows.png)
was inspected. It shows Czech, Chinese/Japanese, Arabic, Hebrew, supplementary
emoji and combining-accent filenames. The actual Slint focus integration test
and native window smoke check cover pane focus transfer separately from pure
presenter state.

Linux native build dependencies were downloaded from official Ubuntu repositories,
checked against signed index metadata/package SHA256 and extracted only under
`.tools/linux-native`; system packages/settings were not modified. Normal Linux
development prerequisites are listed in the README. The build uses normal linked
Fontconfig; the attempted dynamic-API override was rejected and is not enabled.

## Covered behavior

- Real files/directories, independent panels, metadata enrichment, unavailable or
  disappearing entries, symbolic links and Windows junctions.
- Bounded result queues, superseded navigation, cancellation, uncancellable old
  reads and reserved worker capacity for the other pane.
- Generation-safe history, failure rollback, progressive new-directory names,
  identity-preserving sorting/selection and volume-path memory.
- Raw non-UTF-8 Linux names and unpaired-surrogate Windows display-path handling.
- Exact Unicode sequences across filesystem, engine and model/input boundaries;
  distinct NFC/NFD names and contents, ZWJ/non-BMP emoji and RTL/CJK scripts.
- Actual Windows Unicode paths beyond 260 UTF-16 units, with both provider and
  engine tests executed rather than capability-skipped.
- Backend reclamation ownership, bounded retry/backpressure, worker-thread
  destruction and FIFO shutdown barrier.
- Exclusive diagnostic PNG creation without overwriting an existing file.
- Literal Windows fixture parameters containing legal shell metacharacters.

Regression tests observed the original missing behaviors before fixes. Additional
Unicode mutation checks were performed only on disposable copies: all three new
provider tests detect Unicode corruption, and all three engine tests detect loss
of Unicode final entries. Production source was not mutated for those probes.

## Dependency security review

A verified official cargo-audit 0.22.2 binary checked the locked dependency graph
against RustSec database commit `965bf9ad083b58e6340d65829d85217b18be9290`
(3 November 2026). Result: **zero matching known vulnerabilities**, with five
informational unmaintained transitive packages: bincode, ttf-parser, zbus_names,
zvariant and zvariant_derive. These warnings remain tracked; no vulnerability
ignore rules were added. See `SECURITY.md` for the security/integrity contract.

Detailed logs, audit JSON, checksums and independent review reports are retained
under `.tools` and `.superpowers/sdd/stage-1`. They are local verification evidence,
not a claim that CI has run or that unknown vulnerabilities are absent.

## Remaining qualification and small limitations

- Linux/Mint native desktop interaction has not been exercised. Ubuntu compilation
  and full headless workspace tests do not establish Mint GUI acceptance.
- Full Unicode production rendering uses the normal WGPU path and installed font
  coverage. Slint's separate software renderer remains diagnostic; unrestricted
  complex-script coverage there is not certified.
- Page Up/Down currently move twenty rows rather than the exact viewport height.
- Selection reconciliation is O(selected); bulk/range selection is not implemented.
- Directory/index memory is O(entry count). Native p95/p99 latency, startup/cache
  effects and large-directory memory targets have not been benchmark-qualified.
- No verified-copy, FTP/FTPS/SFTP or macOS implementation is claimed. Their rules,
  Unicode behavior, lack of an application path ceiling and required corruption/
  race tests are specified in the architecture and security documents.
- Current multiselection uses independent cursor/selected identities and
  Space/Insert toggling. Full range/select-all behavior and the shared mutation
  queue with progress are specified for the copy/operation milestones; no
  Copy/Move/Delete handler is exposed by this read-only milestone.
