# Custom Command ergonomics implementation plan

> For agentic workers: use subagent-driven-development for independently owned tasks.

**Goal:** Make Custom Commands as clear as Salamander's User Menu, with compact
ordering, explicit terminal execution, separators, and native icon selection.

**Architecture:** Retain stable command IDs and the atomic Options draft. Program
arguments remain native argv; terminal scripts remain an explicit mode with context
passed as environment data. Separators are configuration entries, never executable
commands. Reuse the existing backend picker and icon pipelines where possible.

**Tech stack:** Rust, Slint, existing Windows/Linux platform services. No mandatory
paid dependencies. Icon extraction must never execute code from the chosen DLL.

## Reference and decisions

Salamander `src/dialogs4.cpp`, `CCfgPageUserMenu::EnableControls`, `LoadControls`,
and `StoreControls` (around lines 2080–2210) separates executable/arguments/cwd,
terminal options and separator state. Its fixed path limits are not adopted.
The screenshot's one-line list and small ordering controls guide the new layout.

Existing Program commands launch without a terminal; Terminal commands open the
chosen terminal. Make that difference explicit in the form and preserve arguments
as separate values. Do not silently convert argv to executable shell source.

For icons, prefer a native Windows resource picker with a backend-created PNG
snapshot retained in the application's icon directory. This reuses existing path
persistence and preview caching. Linux should offer installed theme icons or a
custom image. Every newly selected icon or custom image must be imported into
managed storage as a 128 × 128 PNG with aspect ratio and transparency preserved.
Resource extraction requests the larger source directly; it must not merely upscale
an earlier 64px snapshot. Identical images share a content-addressed file. Neither
the toolbar nor saved settings may depend on the original selected image or DLL.
The icon path is read-only in the form so typing cannot bypass import. Verify that
the managed image still decodes after the original source has been removed.
Confirm implementation capability before marking either platform available.

## Parallel tasks

- [ ] UI: one-line list, compact header actions, retained drag reorder, selected
  item details; hide executable options for separators; explicit terminal mode;
  optional appearance/context sections; no preview side effects.
- [ ] Core/persistence: Separator invocation, backward-compatible round trip,
  configuration bounds, fail-closed preview/execution, no shortcut registration.
- [ ] Toolbar: render separators without buttons, preserve command identity,
  suppress useless leading/trailing/repeated separator lines.
- [ ] Icon backend: resource-only native selection/extraction, cancellable worker,
  owned storage, Linux theme/file source and stale-response guards.
- [ ] Verify: targeted configuration and actual Slint interaction tests, Windows
  and Linux affected boundaries, native smoke, final package build and documentation.

## Review focus

Malformed/old configuration; stale selected IDs while reordering; separator invoked
through a retained shortcut; filenames containing shell metacharacters; cancelling
Options after selecting an icon; resource picker closed during shutdown; minimum
window sizes and font/density changes. No checks should inspect or expose secret
environment values.
