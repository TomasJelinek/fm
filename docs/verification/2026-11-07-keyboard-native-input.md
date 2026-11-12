# Keyboard search crash investigation — 2026-11-07

Status: **not reproduced; not fixed**. No production behavior changed. The new
code adds an opt-in external-input mode to the native diagnostic example and a
reusable Windows driver in `scripts/probe-keyboard-search.ps1`.

## Historical failure

Windows Application events 1000/1001 identify one failure at 11:05:25 local time:

- Application: `D:\experiments\dev\FM\_build\release\me.exe`.
- Faulting module: `ucrtbase.dll` 10.0.26100.9711, offset `0xa527e`.
- Exception: `0xc0000409`, subcode 7; application timestamp `0x6aeef3e1`.
- Report ID: `48c3fde8-c3fe-45f5-904e-186474f98bfc`.

This records an abort, without a Rust panic site or application call stack. It
does not establish memory corruption or identify the search callback as its cause.
The WER archive directory remains denied by Windows ACLs even outside the Codex
sandbox. The temporary dump named by WER was not available for analysis. No ACLs,
registry settings, dump configuration or installed application files were changed.

The persisted profile is version 10, Dracula, desktop density, Segoe UI 13, and
has no shortcut overrides. The native probe uses that appearance with in-memory
preferences. Before and after profile SHA-256:
`EE8B6D629D9931443C1BD39A9B8BD65C791E54D5F11EF86C7640E8C04E1DC6CB`.

## Current verification

The example was freshly built with Rust 1.99.1:

```powershell
$env:CARGO_PROFILE_DEV_DEBUG='0'
$env:CARGO_PROFILE_TEST_DEBUG='0'
$env:CARGO_INCREMENTAL='0'
./scripts/cargo.ps1 +1.99.1 build -p midnight-ui --example native_keyboard_search --release
./scripts/probe-keyboard-search.ps1
```

The actual desktop run used Winit/FemtoVG-WGPU with `WGPU_BACKEND=dx12`, scale
2.25, and 1340×820 logical client size. Native Unicode `SendInput` checks the
probe's foreground-window ownership before injecting text. The example reports
its observed query, matching-row count, stable command ID and capture state;
the driver asserts those values instead of inferring success from process survival.

- Full native run: **163 query/focus assertions**, exit 0. Two rounds of a–z and
  0–9 each started from an empty query and were followed by Backspace.
- Native text: `copy`, `files.copy` (2 results), `Copy files to clipboard`
  (1 result), Czech, CJK, Arabic/Hebrew, and decomposed `e` + combining acute.
  Exact query spelling was asserted. Unknown Unicode queries returned zero rows.
- The selected command was first moved to the end of the catalogue (`drive.z`),
  and retained that stable identity through filtering and clearing.
- **100 rapid edit/clear rounds** used ordered OS input, followed by an asserted
  marker query to prove the preceding input queue had drained.
- Native click into the capture field plus F8 produced candidate `F8` with an
  unchanged empty query. Native click back to search accepted `copy` again.
- Reusable source-script follow-up: 19 assertions, zero alphabet/digit rounds,
  the same 100 rapid edit/clear rounds and focus checks, exit 0.
- Original release example path: native incremental typing/deletion, empty
  matches, scrolled selection, actual 320×200 resize and 1340×820 recovery, exit 0.
- Latest pre-existing keyboard integration binary: **19 passed**, 0 failed.
  It was run directly, so this is explicitly not a new Cargo test compilation.
- Focused Rust formatting and `git diff --check`: passed.

Release example SHA-256:
`8475EBBBED510D4257677FF22FFB715CFA401F8B342BD253CB812B300785A2F6`.

Logs/captures in this directory:

- `build-release.log`
- `keyboard-options-existing-binary.log`
- `native-driver-full.log`, `native-input-full.err`
- `native-driver-source-script.log`, `native-input.err`
- `native-input-before.png`, `native-input-full-after.png`
- `native-default.err`, `fmt-check.log`

## Limits and remaining evidence

These native runs exercise the actual release UI/presenter and renderer, using
in-memory preferences. They do not reproduce the full application's asynchronous
engine/plugin composition or recover the original process state.

Supplementary-character native automation remains **unqualified**. Per-UTF-16-unit
VK_PACKET input did not preserve the surrogate pair, and a separate queued-message
attempt also failed to deliver the intended character. Neither caused a crash.
These failed driver experiments are not counted as successful Unicode checks or
as evidence for the reported first-character abort. They require a correctly
observed native committed-text/IME boundary before attributing the loss to an
application component.

A causal fix still requires a reproducible failing run with stderr/backtrace or
a readable crash dump identifying the panic/abort call stack. First-character
search must remain open in the implementation backlog until that evidence exists.
No exception swallowing, dependency changes, filesystem work in UI callbacks,
security relaxation, or speculative search/model rewrite was introduced.
