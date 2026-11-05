# Panel search, native metadata and generated-code reuse

This stage adds literal Unicode filename-prefix search, optional live filtering,
pointer-offset tooltips and native compression/link indication. Internal
Copy/Move/Delete remain disabled in this read-only stage. Their operation queue,
overwrite policy and mandatory independent destination-checksum verification
remain requirements for the next implementation stage.

The backend matches immutable full directory snapshots with Unicode case folding
and original grapheme boundaries. Query/navigation/source revisions are checked
after output backpressure and again by the presenter. Filtering keeps watches and
size scans on the canonical listing; actions use the visible projection. Tests
cover stale results, held clicks, hidden selection, two panes/tabs, cancellation,
full queues, IME commits, emoji backspace and same-folder refresh/sort continuity.

Rendering retains a single shaped paragraph for colored filename prefixes.
Long RTL names pan toward native selection endpoints instead of hiding the match
at the clipped edge. Search editing similarly pans to the caret, while the match
count uses its natural width. Extreme filesystem timestamps are checked before
local-time conversion; raw timestamps retain their sorting semantics.

Compression comes from no-follow native metadata, not archive suffixes or stored
size guesses. Windows uses its native attribute; Linux respects the statx support
mask without opening contents, including FIFOs. Directory links/junctions stay
separate from real directories and are excluded from recursive size traversal.
Windows supplies system link overlays; other platforms use host link artwork or
recognizable backend fallback artwork. Compression and link marks can coexist.

Shutdown transfers the closed event receiver itself to backend reclamation.
A sender with a preexisting Tokio permit can still publish after receiver close;
retiring only the messages present during a single drain misses that race. A
drop-thread regression verifies that late payloads are released off the UI caller.

The Windows COM guard is constructed lazily after successful initialization.
A real isolated-STA regression verifies that a rejected incompatible initialization
leaves the original apartment intact. This follows Microsoft's rule that only
[successful COM initialization calls acquire a matching uninitialization](https://learn.microsoft.com/en-us/windows/win32/api/combaseapi/nf-combaseapi-coinitializeex).
The earlier eager `then_some(guard)` dropped a temporary guard on failure and
released state it did not own.

## Generated Rust review

An earlier tooltip build adapter wrote a complete token stream as one line:
`target/debug/build/fm-ui-cf2269b653974855/out/main.rs`, 8,105,236 bytes. Omitting
normal formatting was our build integration bug. The adapter now formats its Rust
AST with syn/prettyplease. Formatting increases disk bytes without adding logic;
the reuse comparison below uses equivalent formatting on both sides.

| Measurement | Before reuse | After reuse |
| --- | ---: | ---: |
| Stock compiler, normalized token bytes | 8,040,877 | 4,839,307 |
| Tooltip-adapted output, normalized token bytes | 8,195,817 | 4,871,700 |
| Tooltip-adapted output, formatted bytes | 14,748,180 | 8,704,058 |
| Tooltip-adapted output, formatted lines | 303,032 | 178,905 |
| Tooltip definition sites | 41 | 17 |

The adapted output decreases by **3,324,117 normalized token bytes (40.6%)**;
formatted output decreases by 41.0%. Normalized token text distinguishes actual
code reduction from whitespace. This is generated-source size, not executable
size, RAM consumption or a controlled build-time benchmark. Generated code still
contains layout, binding, state and lifecycle implementations; it is not several
megabytes of application data or text copied from files being browsed.

The changes are in handwritten inputs and functions:

- `crates/fm-ui/ui/hinted-inputs.slint` shares native ComboBox/LineEdit/action-button controls
  with internal tooltips; per-instance children previously forced inlining.
- `crates/fm-ui/ui/command-menu.slint` shares F9/F10 menu bodies. Immutable
  delegate positions preserve a held gesture while native icon images arrive.
- `crates/fm-ui/src/file_keys.rs` and `file_key_bridge.rs` share panel key policy
  and dispatch. Native text/IME pass-through does not refresh the row models.
- `crates/fm-ui/build/tooltip.rs` stores each original stock show body once in
  a private impure function, reused by initial show and conditional position retry.

The exact-pinned adapter changes compiler IR structurally. It neither edits the
dependency registry nor rewrites generated Rust text. Normal Slint resource/import
tracking remains in build.rs. Source review checked function scope, borrow safety,
retry/hide lifecycle and all ten original menu actions. Runtime tests exercise
native input and rendered pixels, including nested popup tooltips, movement,
large fonts, all window edges, click-through and live icon delivery.

Audit evidence is local and ignored: `.tools/slint-generated-audit/`,
`.tools/tooltip-function-reuse-spike/audit-baseline/` and
`.tools/slint-generated-reuse-final/` and
`.tools/slint-generated-hinted-actions-production/`. Repeated stock generation from each frozen
input produces identical normalized token text. Public source files retain the
100,000-byte limit; Cargo-generated `OUT_DIR/main.rs` is an ignored build artifact.
The actual native Cargo output in this verification is
`target/debug/build/fm-ui-64e7e371c9f495ad/out/main.rs`; its 8,704,058 bytes match
the adapted audit output byte for byte (SHA256
`d5d9abe3ef22fce81603e6fe8f98510cc34e13388f64293bb41e6d2aeb7148c8`).
Cargo's hash-named build directory can change later.

The final local button pass preserves eleven original hint expressions, callbacks,
icons and gesture identities. Its same-input-path prototype removes another
1,086,550 normalized bytes, then the same five-file change is adopted and audited
against the actual Cargo output. The [deeper size review](slint-code-generation-size.md)
explains the remaining initialization/layout/popup costs and measured alternatives;
40.6% is not a claimed optimum.

Verification also exposed an exhausted development disk. Linux cache writes
reported EIO/ENOSPC, and a Windows link failed while writing its debug database.
Only explicitly verified obsolete prototype binaries and older project-test PDBs
were removed; source, logs, current symbols and final binaries were preserved.
A clean native link then succeeded with the same default debug profile. This
does not establish an inherent code-generation/type limit or isolate the old
partial PDB from disk pressure; no debug information was disabled.

## Final platform verification

Windows x64 has **467 unique passing tests**, zero unresolved failures. The
unchanged backend/platform suites contribute 270, and the final fresh UI suite
contributes 197. Three subprocess fixtures are directly ignored and exercised by
passing parent tests. The initial UI pass exposed one stale accumulated Ctrl+Space
expectation; its corrected test and the complete final UI rerun of 197 tests passed.
Whole-workspace strict all-targets Clippy, formatting, build, backend smoke and
the source guard covering 189 files at a 100,000-byte limit pass.

Native icon fidelity tests retain their required nonempty-art and visible-overlay
assertions, using a shared test-only qualification phase. A separate 16-thread
real-OS test validates optional availability and pixel bounds. Probes identified
Shell E_OUTOFMEMORY before DIB/drawing during concurrent stock lookups, including
after disk recovery. Production lookup/fallback behavior is unchanged, and engine
fallback-art tests remain mandatory.

Linux/Ubuntu 24.04 under WSL has **439 passing tests**, no failures or ignored
tests across the full workspace checkpoint. After the final UI-only change, all
195 UI tests passed again, followed by whole-workspace strict all-targets Clippy,
the native application build, the 189-file source guard and backend smoke. The
backend/platform sources remained unchanged, as confirmed by the frozen source
hashes. The actual root build.sh helper also passed. Linux UI tests use the
headless software renderer; this is not a native Linux desktop visual capture.

The actual root build.bat helper and native Windows FemtoVG-WGPU smoke passed.
The inspected [current screenshot](../screenshots/panel-search-windows.png) shows
Unicode names, a real junction with its system overlay, a compression mark only
on the actual compressed fixture, the complete single-line cwd and centered
footer controls. The final FM 0.1 window was left visible and responding in the
interactive Windows desktop session, with its executable path and native title
verified. No build followed that launch.

Final hashes match all 241 frozen source/configuration files. The actual Cargo
generated file matches the audited AST output byte for byte. Temporary build
products and detailed logs remain ignored; only handwritten code, tests, build
helpers, documentation and the intentional screenshot are committed. Hosted CI
has not been claimed as a completed run.
