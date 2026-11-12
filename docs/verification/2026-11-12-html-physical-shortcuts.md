# HTML physical navigation shortcut compatibility

Backspace and Alt+Left/Right now reach Chromium before the host resolves a file
navigation action. The CEF key callback uses its live editable-focus value; the
cached display State.editable field is not authority. After arbitration the host
resolves the current configured chord and ignores replies after content loses
focus. Unbound keys are not replayed. Key.repeat is preserved in CEF's documented
EVENTFLAG_IS_REPEAT flag (8192); the added navigation keys do not repeat file
changes, while editable input and baseline Space behavior retain their semantics.

The independent public API/SDK is version 0.2.0 because adding Rust enum variants
changes exhaustive source matches. MWV1 remains wire version 1. Existing tags 0–9
are byte-for-byte unchanged; the new HostShortcut tags are Backspace 10, AltLeft 11
and AltRight 12. New tags require the exact explicit launch opt-in
MIDNIGHT_WEB_VIEWER_HOST_SHORTCUTS=physical-v1. The host's dedicated launch entry
adds that fixed value to its cleared environment; the old spawn entry remains
baseline and never inherits an ambient opt-in. New providers under old hosts and
old providers under new hosts retain baseline wire traffic. The strict manifest
schema and protocol 1 require no additional fields.

The coordinator requested source implementation before the final coordinated
build. No pre-change behavior RED is claimed for this addition. Added regression
coverage pins all legacy shortcut bytes and the extended codec, tests exact/absent
opt-in, live editable focus, modifier variants and repeat suppression, and checks
host binding changes and stale focus. The existing real Chromium qualification
also exercises all three new signals outside inputs and retains Backspace and Alt
arrows inside a textarea, including an assertion that Backspace deletes text.

Linux public API/SDK workspace tests pass 13 (protocol 8 and SDK 5); strict all-target
Clippy passes. The independently built HTML plugin's no-default-features library
passes 9 tests (four new host-key cases and five existing output/surface cases),
and its strict all-target Clippy passes. Evidence is under
`_build/qa/viewer-physical-keys-20261112`: `api-linux.log`,
`api-clippy-linux.log`, `plugin-linux.log`, and `plugin-clippy-linux.log`.
The initial generic Linux Cargo cache lacked optional CEF metadata; the existing
dedicated HTML cache resolved it without dependency or source changes. That
infrastructure refusal is retained in `plugin-linux-missing-cache.log`.

The Windows installed-runtime qualification below supplies the native browser
evidence. The pure Linux tests do not claim installed Linux runtime qualification.

The coherent Linux application batch passes all 120 viewer UI library tests,
including browser-first physical keys, live rebinding and stale-focus refusal.
Web host tests pass 6, including equality of the platform launch constants and
public API contract. The Linux browser supervisor filter passes 12 with 2 opt-in
native cgroup tests ignored. Viewer file-order tests pass 3; default/override and
comparison-context integration targets pass 6 + 2, and actual viewer keyboard tests
pass 2. Evidence: `viewer-ui-linux.log`, `web-host-linux.log`,
`browser-launch-linux.log`, `viewer-file-order-linux.log`, `core-viewer-linux.log`,
`core-comparison-linux.log`, and `viewer-keyboard-linux.log` in the same QA folder.

A later independent review confirmed that arbitrary saved Previous/Next-file
remaps could bypass the browser arbiter, including the outer native digit/bracket
route. The scoped correction forwards those non-arbitrated HTML shortcuts as
browser input and retains text events. Browser-arbitrated Space/Shift+Space,
Backspace and Alt arrows still use current bindings. Other viewer modes retain
arbitrary file-navigation remaps. A new test covers both directions, printable Q,
Ctrl+J, digits, brackets and the software fallback; the previous Ctrl+N expectation
now requires browser forwarding. Generic arbitrary-key arbitration is an explicit
follow-up, rather than relying on stale State.editable. No public API/wire or
packaged provider change is needed for this host-only correction.

The full Linux native-CEF library configuration also passes all 12 tests, including
the resource cancellation fixture, selection bounds and live-focus policy helper.
Strict native release Clippy passes for the library and tests. Evidence:
`plugin-native-linux.log` and `plugin-native-clippy-linux.log`. This validates the
native adapter's compilation and unit boundaries; actual browser focus remains a
separate installed-runtime qualification. An omitted opt-in field in one native
resource test initializer was corrected before this gate, without changing its
resource-ownership assertions.

The final Linux delta passes all 121 viewer UI tests after the custom-remap and
Space stale-focus corrections. Its compile invocation started at
2026-11-12T10:02:37.8257484Z, after the last source change at 10:01:50.3213960Z.
The four-process bounded artifact batch passes 161 tests across its six targets;
all selected-target strict Clippy checks pass. Evidence: `delta-html-viewer-linux.log`,
`delta-build.jsonl`, `delta-clippy-linux.log` and `final-summary.txt`.
The final source remains frozen and all owned Linux build processes have exited.

The final Windows host filter passes all 10 web-input tests against the frozen
source, including both custom-remap directions and stale replies for all supported
physical chords (`final-web-input-windows.log`, 0.42 seconds). The protected HTML
0.2.0 runtime also passes both actual Chromium tests
(`html-native-windows.log`, 4.18 seconds). These exercise the new Backspace and Alt
arrow signals outside editable content and preserve textarea editing without file
navigation. The tested runtime is
`C:/Users/tjelinek/MidnightExplorer-20261112-user-menu-d5cf41d7`. The coordinating
Windows startup qualification additionally reports native and software backends
starting with exit code 0, empty stderr and unchanged preferences. No installed
Linux Chromium result is claimed.
