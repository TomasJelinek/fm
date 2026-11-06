# Theme Expansion and Command Row Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans for palette integration
> and superpowers:dispatching-parallel-agents for independent settings and command
> row changes. Share the workspace with explicit file ownership and Cargo tokens.

**Goal:** Eight readable themes and a command editor row matching the footer height.

**Architecture:** Shared Slint palette data supplies semantic tokens. Core theme
identities pass through existing backend settings and presenter preferences.
Command controls reuse the compact footer geometry without changing actions.

**Tech stack:** Existing Rust, Slint 1.18.1 and current native settings services.

**Spec:** [theme expansion](../specs/2026-11-06-theme-expansion-design.md).

## Constraints and review focus

- Preserve Blue/Dark/Light IDs and settings migration; no extra dependency.
- No filesystem/settings access from UI callbacks; no native icon tinting.
- One Cargo invocation per OS; all build and QA output under `_build`.
- Rustfmt 100 columns and handwritten source files at most 100,000 bytes.
- Check selected/search text, failed-copy/delete messages and focus visibility.
- Small font, font32, touch, native button minima and popup placement must fit.

## Tasks

- [x] Settings: extend `PreferenceTheme`, common codec, and presenter mappings;
  test roundtrips, previous IDs, unknown names and choosing each new theme.
  Files: core `preferences.rs`, common `settings.rs` and focused tests,
  UI `presenter/preferences.rs` and its tests.
- [x] Command row: add failing geometry assertions, then adjust only command
  controls in `ui/main.slint` and `ui/terminal-control.slint`. Root changes
  `Theme.command-height` to `Theme.bottom-height`. Test actual focus/history/Enter
  as well as containment in `tests/command_line_layout.rs` or its existing equivalent.
- [x] Palettes: extract reusable definitions to `ui/theme-palettes.slint`, wire
  `theme.slint` and the Options picker, and test new theme contrast and rendering.
  Preserve existing palette values and append the five documented choices.
- [x] Review changes, update README/main design, run focused checks on Windows and
  Linux, inspect native renders, then rebuild the Windows release and smoke test it.
- [ ] Commit and push the reviewed batch under the existing user authorization.
