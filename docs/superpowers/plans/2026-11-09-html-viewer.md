# Embedded Chromium HTML Viewer Implementation Plan

> **For agentic workers:** Use `superpowers:subagent-driven-development` for the
> independent slices below; the coordinator owns integration and build leases.

**Goal:** Open HTML in an optional Chromium-backed viewer inside Midnight Explorer.

**Architecture:** An independent CEF process plugin uses a public interactive
protocol. Backend services own captured resources, process supervision and frames;
Slint presents the surface and forwards input. Existing viewer protocols and
decoder sandbox profiles remain unchanged.

**Tech Stack:** Rust, Slint, Tokio, pinned CEF/cef-rs, existing platform APIs.

**Spec:** [HTML viewer design](../specs/2026-11-09-html-viewer-design.md)

## Implementation status — 10 November 2026

Tasks 1–4 are implemented, including the independently built Windows CEF provider,
public API/SDK, captured native/archive resources, host supervision, Slint surface
and input. The initial native fixture, actual host and Slint-window probes pass.
The checklist below records the original implementation sequence; current evidence
and remaining qualification are authoritative in the
[verification report](../../verification/2026-11-10-html-viewer.md).

Linux process containment is implemented and tested, but native Chromium rendering
is still unqualified. The 10 November environment audit found usable existing user
cgroup delegation and WSLg; the direct shell is outside that delegation, so native
qualification must launch through the user manager. The exact Linux CEF runtime
and package prerequisites remain to be completed under the
[Linux completion plan](2026-11-10-html-linux.md).
Do not restart implementation of the completed Windows slices. Remaining native
qualification includes Linux display/sandbox/IME and extended crash/stress testing.

## Global constraints

- Independent API/SDK and plugin workspaces; no private application dependencies
  in the plugin. Platform implementations use separate files.
- Free-of-charge dependency policy, pinned-license audit and notices apply to
  runtime binaries as well as Rust dependencies.
- All generated output under `_build`, caches under `.tools`; preserve unrelated
  current work. Share one main application build lease.
- Public Rust style, 100 columns, UTF-8/LF, handwritten source files under 100 KB.
- Offline/JavaScript-disabled by default; keep Chromium sandbox enabled.

## Review focus

1. Popup/redirect/file URL bypasses must not gain filesystem or shell authority.
2. Closing or changing files during rendering must not show stale pixels or leak
   Chromium subprocesses and resource leases.
3. Relative assets must work with captured native/archive sources without treating
   display labels or temporary paths as operation identities.
4. Editable HTML focus and IME must not accidentally trigger panel file cycling.
5. A successful API test must not be mistaken for an installed native browser test.

## Task 1: Public interactive protocol

Create independent `web-viewer-api/midnight-web-viewer-api` and
`web-viewer-api/midnight-web-viewer-sdk` projects and tests. Define the spec's
`Command`, `Event`, `Viewport`, `Policy`, `Resource` and `Frame` types, validated
framing, bounded I/O and generic service helpers without host or CEF dependencies.

- [ ] Write roundtrip, malformed length/tag, numeric overflow, invalid UTF-8,
  stale-generation and queue ownership tests; observe expected failures.
- [ ] Implement the versioned contract and publish its precise field/key semantics
  in the API README for downstream tasks.
- [ ] Pass the complete API/SDK tests, formatting and strict Clippy. Produce local
  registry packages beneath `_build` for the independent plugin.

## Task 2: Pinned CEF provider and runtime packaging

Create `plugins/midnight-viewer-html/` with its own lockfile/tests, policy and CEF
adapters, separate Windows/Linux entry files, manifest and distribution notices.
Consumes Task 1's API/SDK; produces the optional `web-viewer.json` runtime package.

- [ ] Verify archive metadata/digests, extract beneath `_build/html-viewer`, inspect
  pinned licenses/credits and transitive dependencies before runtime distribution.
- [ ] Write failing policy/resource-origin/frame lifecycle tests; implement the
  offline provider, CEF request handlers, owned frames and input/search/zoom.
- [ ] Implement the exact Windows bootstrap DLL entry and Linux isolated lifecycle;
  never add an unsandboxed fallback.
- [ ] Exercise the public protocol against native CEF with deterministic HTML/CSS,
  script/network denial, source assets and close/crash fixtures.
- [ ] Add build/package helpers and inventory/notices integration; keep the provider
  removable without affecting other viewers.

## Task 3: Backend host and resource authority

Create `crates/midnight-web-viewer-host/` and separate platform launcher files.
Extend host-side captured-content support and viewer routing without changing
existing public semantic/raster contracts. Consumes Task 1 packets and captured
source/resource capabilities; produces bounded generic interactive viewer events.

- [ ] Write failing trusted-package, message validation, process lifecycle,
  stale-frame and contained-resource tests.
- [ ] Implement private package discovery, supervised multiprocess launch,
  asynchronous bounded transport and latest-frame coalescing.
- [ ] Implement captured native and archive asset scopes with revision/containment
  checks; test percent-encoded traversal, links, mutation and absent assets.
- [ ] Wire viewer engine opening/routing, cancellation, source reads, clipboard
  and browser find/zoom; preserve Basic View and existing associations.
- [ ] Test the complete affected host crates, including failure and cleanup paths.

## Task 4: Viewer frontend integration

Extend `crates/midnight-core/src/viewer.rs`, viewer UI modules and `viewer.slint`
with a generic interactive surface and typed input. Keep platform-specific input
conversion in separate files. Integrate optional providers into existing Options
and extension association discovery.

- [ ] Add failing frontend tests for HTML mode, search, resize/stale frames,
  content versus toolbar focus, Escape and browser-arbitrated file cycling.
- [ ] Reuse normal viewer chrome/surface; forward pointer, keyboard and IME once.
  Add explicit content-policy controls with the spec's safe defaults.
- [ ] Verify existing text/source/Markdown/image/PDF controls remain functional.

## Task 5: Installed qualification and documentation

- [ ] Run independent plugin/API suites and affected host/UI suites, strict Clippy,
  formatting, source-size checks and a fresh code review.
- [ ] Package/register the tested private runtime. Open HTML through the actual
  viewer, inspect HTML/CSS/relative assets, search/zoom/input, Escape and process
  cleanup; measure full-window scroll/resize latency.
- [ ] Qualify Linux independently or state the precise unverified/platform blocker;
  never infer cross-platform success from Windows.
- [ ] Update the feature status, viewer/developer API documentation, security model,
  build instructions, dependency inventory and release notices with actual results.

Integration and commits must preserve unrelated work in this shared checkout;
passing a slice is not completion of the overall application goal.
