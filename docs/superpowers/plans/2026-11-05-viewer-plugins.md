# Viewer plugins and external editor implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans or
> superpowers:subagent-driven-development to implement this plan task by task.

**Goal:** Deliver independently built basic, syntax and Markdown viewer plugins,
host-owned asynchronous viewer windows, and configurable external editing with F4.

**Architecture:** Public C ABI/SDK exchanges bounded versioned messages and content
leases. The host supplies workers, discovery, content, routing and one Slint renderer;
plugins supply independently tested parsing/indexing/search. Native editing stays
behind platform services and passes literal arguments without shell interpolation.

**Tech stack:** Rust, Tokio, Slint 1.18.1, platform DLL/SO loading; syntax and
Markdown dependencies selected after their bounded renderer/parser spike.

**Spec:** [extension](../specs/2026-11-05-viewer-implementation-extension.md) and
[ABI design](../specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md).

## Global constraints

- Start implementation after the current mutation, scrolling, view-mode and branding stage.
- One process, trusted native plugins; no claim that an in-process C ABI is a sandbox.
- Plugin projects/tests are independent of application projects, sharing only public API/SDK.
- One Git repository, as selected by the user on 2026-11-06: independent Cargo
  workspaces under `plugins/`, not host workspace members. Verify each plugin's
  exported directory without host sources; CI cannot modify host files.
- All I/O, decoding, indexing, search, discovery and large cleanup runs on backend workers.
- Rustfmt 100 columns; handwritten source files <=100000 bytes; artifacts under `_build`.
- Unicode/native identity, u64 offsets, checked arithmetic and explicit memory/work limits.
- Windows/Linux implementations and tests are separate files; macOS remains a later platform.
- 256 KiB content chunks, 16 MiB viewer cache, 64 MiB aggregate viewer cache.

## Review focus

1. A malicious filename or editor argument must not become shell syntax.
2. Closing/cycling while reads run must release every lease without applying stale results.
3. Invalid bytes, huge lines and >4 GiB offsets must not cause unbounded allocation or truncation.
4. Plugin absence/incompatible ABI must remove capability rather than invoke a hidden fallback.
5. Styled text and Markdown must preserve Unicode/source mapping without executing active content.

## Task 1: Public API/SDK and separately built conformance fixtures

- [x] First prove no-wrap styled Unicode layout, source-correct selection and hit
  testing with Slint's paragraph renderer or a shared Parley adapter. Stock
  StyledText alone wraps and lacks selection; do not freeze incomplete UI models.
  The shared Parley/Swash adapter passes 13 Windows/Linux tests and visual proof;
  see [evidence](../../verification/viewer-renderer.md). Host accessibility and
  aggregate session budgeting remain part of Task 2, not completed UI features.

Files: independent `midnight-plugin-api` and `midnight-plugin-sdk` projects;
public `include/midnight_plugin_v1.h`, protocol/ownership specifications and fixtures.
Host project references only the public API artifact. Publishable `.crate` artifacts
are staged under `_build`, without requiring publication to a public registry.

- [x] Write independent ABI tests for short descriptors, major/minor compatibility,
  bounded CBOR, malformed ranges, exactly-once release, queue-full rejection, cancel,
  terminal delivery, service continuation and quiescent shutdown.
- [x] Run and record RED before implementing the ABI dispatcher/ownership adapters.
- [x] Implement `midnight_plugin_entry_v1`, common create/submit/cancel/drain/release/
  shutdown functions, schedule-work/content service messages and semantic viewer models.
  Generate/document the header from the actual repr(C) contract, with no Rust types at ABI.
- [x] Package API/SDK and build the fake cdylib from an exported project without application sources;
  run conformance tests independently and record GREEN.
  Windows/Linux pass 58 Rust tests, strict Clippy, and seven real DLL/SO scenarios
  from an exported independent fixture; see [evidence](../../verification/plugin-api-sdk.md).

## Task 2: Host loader, content broker and shared renderer

- [x] Measure generated Slint compilation before adding another host window.
  Evaluate a small bindings-only crate with `lib.test = false` so the same generated
  module is not compiled again inside every UI unit-test build. Keep handwritten
  presenter debugging available. This is a build-cost experiment, not a claim that
  moving files reduces generated source size; retain it only with measured benefit.

Files: `crates/midnight-plugin-host`, per-platform loader files, core viewer/content
contracts, `midnight-viewer-engine`, `midnight-viewer-ui/ui/viewer.slint`,
UI viewer presenter/renderer adapter and host integration tests.

- [x] Test executable-relative discovery, duplicate IDs, invalid manifests, wrong ABI,
  plugin removal, escaped package paths and blocked/cancelled content services.
- [x] Implement native secure loaders and descriptor validation, bounded host work and
  readonly content leases with stable revision checks. Preserve library lifetime until
  all callbacks/buffers/sessions quiesce. Test callbacks after cancellation.
- [x] Extend the Task 1 shared Parley renderer with generated semantic styles;
  test raw Unicode/RTL/combining and exact decoded text before/after styling. Do not split
  paragraphs into independently shaped token widgets or copy Slint into plugins.
- [x] Add virtualized viewer UI and tests for scrollbar/viewport requests, selection,
  Ctrl+F/next/previous, Escape, window size and Space cycling the captured panel order.
  Prove typing Space in search does not advance the file.
- [x] Run generic host integration tests using separately built fixture DLL/SO packages.

## Task 3: Basic text and binary plugin

Files: independent `plugins/midnight-viewer-basic` workspace, manifest, parser/index/
search modules, fixtures/tests, build helpers and CI. No host source modifications.

- [x] Add failing fixtures for UTF-8/UTF-16 boundaries, invalid/control bytes, all newline
  forms, binary hex rows, enormous lines, u64 offsets and cross-page/cancelled search.
- [x] Implement bounded viewport-first decoding, sparse indexing, search and source maps
  using only public API content/work services. Report partial/limited states explicitly.
- [x] Run plugin tests and package it beside a released host in a disposable install tree.
  Verify F3 unknown-type routing, Alt+F3, binary mode and plugin removal on Windows/Linux.

## Task 4: Syntax and Markdown plugins

Files: independent `plugins/midnight-viewer-source` and `plugins/midnight-viewer-markdown` workspaces,
each with its own manifests/dependencies/locks/tests/fixtures/build/CI.

- [x] Pin reviewed syntax/Markdown libraries and publish their actual supported catalogue
  and licences. Test every declared syntax, multiline context and unknown-type fallback.
- [x] Implement source highlighting through semantic spans; test budget exhaustion and
  unchanged raw text/selection. Reuse only public SDK mechanisms, no cross-plugin imports.
- [x] Implement Markdown semantic blocks, source maps and safe link metadata; test raw
  HTML/script, remote images, malformed/large nesting and cancellation. No automatic fetch.
- [x] Run each independent suite and real-host routing/absence/theme/Unicode integration.

## Task 5: External editor and settings

Files: core editor capability/preferences, platform-specific editor service and tests,
common settings codec/migration, engine launch controller, UI options/F4 wiring.

- [x] Test literal native path argv, shell metacharacters, missing editor, unsupported
  nonlocal entries, settings persistence and launch failure reporting.
- [x] Implement backend default detection and explicit executable/argument settings;
  substitute `{file}` only as a full argument. Add no shell parser for application paths.
- [x] Wire F4 with editor-focus guards and Options. Test focused-file dispatch without
  accidentally opening all multiselected files or interpreting a directory as text.
- [x] Add toolbar New empty file and Shift+F4, reusing the operation name dialog and
  bounded mutation queue. Add a prepared native creation capability with exclusive
  no-clobber semantics. Test Unicode names, existing file/directory collision,
  replaced parent, cancellation and literal zero-byte contents on each platform.
  Delivered early in the [empty-file stage](2026-11-05-empty-file.md); it creates
  the file without launching an editor. Do not reimplement it during F4 work.
- [x] Run platform/host tests and native smoke checks; document defaults and limitations.

## Task 6: Distribution, review and delivery

- [x] Run each independent plugin CI/test command, host tests, fmt, Clippy and size guards.
- [x] Build optimized host and plugin releases; package only versioned plugin artifacts.
- [x] Measure startup, first viewport, bounded memory, cancellation and large-file scroll/search.
- [x] Review ABI/lifetime and native launch boundaries independently; fix material findings.
- [x] Update README, architecture, keyboard help, security and verification records, and
  prepare the host and independent plugin sources for commit/push in the shared repository.

The [verification record](../../verification/viewer-implementation.md) contains final
Windows/Ubuntu results and limits. Delivery uses the existing feature branch and
the user's authorized commit/push workflow; publishing another GitHub release is
separate from this implementation stage.
