# Documentation map

This index describes the implemented Midnight Explorer 0.1 milestone and the
accepted work that follows it. A design or checked-off historical plan is not
evidence that a feature is available in the current executable.

## Current product and developer guidance

| Subject | Authoritative entry point |
| --- | --- |
| Available features, installation, build/run/check commands and output locations | [Project README](../README.md) |
| Keyboard and mouse behavior | [Keyboard reference](KEYBOARD_SHORTCUTS.md) |
| Security, identity, cancellation and integrity requirements | [Security requirements](../SECURITY.md) |
| Rust style, platform separation and source-size limit | [Code style](../CODE_STYLE.md) |
| Dependency and feature selection, free-use/distribution requirements | [Dependency policy](DEPENDENCY_POLICY.md) |
| Pinned dependency licenses and enabled features | [Third-party inventory](THIRD_PARTY.md) and [machine-readable inventory](third-party-inventory.csv) |
| Optimized build settings and qualification limits | [Release build](RELEASE_BUILD.md) |
| Windows native mutation guarantees and races | [Windows mutation safety](../crates/midnight-platform-windows/MUTATION_SAFETY.md) |
| Backend locale capture and presentation | [Locale formatting](../crates/midnight-platform/LOCALE_FORMATTING.md) |
| Rolling architecture and planned contracts | [Main architecture](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md) |

Implemented operations are rename/F2, empty-file creation/Shift+F4, directory
creation/F7, Trash deletion/F8 or Delete, and explicit permanent deletion with
Shift. They use the bounded confirmation and mutation queue. File activation
uses host associations, and submitted commands use the selected external shell.

F3/Alt+F3 internal viewing, configurable F4 editing, F5/F6 copy/move, dynamic
plugins, archive browsing, network-neighborhood discovery, reusable sudo
sessions and decoded media thumbnails remain planned. The Thumbnails panel mode
currently displays host icons. Windows F11/F12 network dialogs and native network
location identification do not imply network discovery.

Preferences use schema **6**, read versions **1–5**, and include ordered tab
locations/active tabs, panel modes, bookmarks, appearance and optional command
history. The [tab-session extension](superpowers/specs/2026-11-05-tab-session-extension.md)
and current README/security document supersede older schema descriptions.

## How to read specifications and plans

The dated files record the increment in which a contract was introduced. Their
unchanged constraints continue to apply, but later accepted extensions override
earlier behavior. Use the current references above for available functionality
and commands. Use the following precedence map before implementing an old plan.

| Earlier design | Later contract that takes precedence |
| --- | --- |
| [Commander controls](superpowers/specs/2026-11-04-commander-controls-design.md) and [compact tooltips](superpowers/specs/2026-11-04-compact-tooltips-design.md): original 44 px Touch targets, early controls and preferences | [Navigation/appearance](superpowers/specs/2026-11-04-navigation-and-appearance-design.md) supplies 40 px Touch metrics; the later contracts below define current persistence, commands and mutations. |
| [Navigation/appearance](superpowers/specs/2026-11-04-navigation-and-appearance-design.md): session-only tabs, earlier preference schemas and numbered bookmark assignments | [Panel navigation/details](superpowers/specs/2026-11-05-panel-navigation-details-design.md) adds command history and numbered tab shortcuts; [tab sessions](superpowers/specs/2026-11-05-tab-session-extension.md) adds schema 6 and persisted tabs/modes. |
| [Native controls/full path](superpowers/specs/2026-11-04-native-controls-and-full-path-design.md) and earlier control descriptions | [Compact-command verification](verification/compact-command-and-size-feedback.md) records the single horizontally pannable command row, quiet file-row hover and completed-partial size styling. |
| [Selection/continuous sizes](superpowers/specs/2026-11-04-selection-and-continuous-sizes-design.md): bulk selection and scan lifetime | [Panel interaction/layout](superpowers/specs/2026-11-05-panel-interaction-layout-design.md) adds range selection, immediate bounded feedback, panel swapping and the splitter. |
| [Panel search/metadata](superpowers/specs/2026-11-05-panel-search-and-metadata-design.md): prefix search, link/compression metadata, tooltips and generated-code reuse | Search contracts remain; schema 6 and `_build/` supersede its older preference/output descriptions. The code-generation measurements are historical checkpoints. |
| Earlier wheel interpolation and read-only milestone descriptions | [Rename/delete/scrolling](superpowers/specs/2026-11-05-rename-delete-scroll-design.md), [empty-file creation](superpowers/specs/2026-11-05-empty-file-design.md), and [native scroll verification](NATIVE_SCROLL_VERIFICATION.md) describe the later delivered behavior. |
| [Mouse activation](superpowers/specs/2026-11-04-mouse-activation-design.md) | Its explicit activation and frozen-identity contract remains; “no mutations” describes that increment only. |
| [Panel views/thumbnails](superpowers/specs/2026-11-05-panel-views-thumbnails-design.md) | Detailed, Brief and icon-based Thumbnails are implemented. Decoder processes, media libraries and thumbnail caches in the same document are future work. |

The [follow-up stages](superpowers/plans/2026-11-05-follow-up-stages.md) define the
accepted implementation order. Stage 1 includes the delivered mutation and
empty-file work; the next stage is the
[viewer/F4 plan](superpowers/plans/2026-11-05-viewer-plugins.md). Its already
checked Shift+F4 task must not be implemented again.

For planned viewer/filesystem work, apply these documents in this order:

1. [Viewer implementation extension](superpowers/specs/2026-11-05-viewer-implementation-extension.md)
   defines three independent basic/source/Markdown packages, F3/Alt+F3/F4,
   renderer gates and viewer-first delivery. It supersedes the older combined
   text/source package and Markdown-after-archives ordering.
2. [Dynamic plugin interface](superpowers/specs/2026-11-05-dynamic-viewer-filesystem-plugins-design.md)
   retains the ABI, ownership, trust, loader and independent-project contracts
   except where the extension explicitly refines them.
3. [Original viewer/VFS design](superpowers/specs/2026-11-04-viewer-vfs-design.md)
   retains content, Unicode, provider identity, Markdown and integrity rationale;
   its Rust sketches are not the public plugin ABI.

Other future contracts are
[copy concurrency](superpowers/specs/2026-11-04-copy-concurrency-design.md),
[terminal-session reuse](superpowers/specs/2026-11-04-terminal-sessions-design.md)
and [network discovery](superpowers/specs/2026-11-05-network-filesystem-design.md).
Their suggested resource limits and measurements are implementation gates,
not completed functionality or measured performance guarantees.

## Verification records are snapshot evidence

The [10–11 November main-branch audit](verification/main-audit.md) records the
review of all 395 baseline files, corrected concurrency and presentation defects,
data-structure experiments, per-test Windows/Linux timings, dependency findings
and qualification gaps. It applies to its named source commits; the newer
viewer/plugin development branch is outside that audit's scope.

[Operations, modes and branding](verification/midnight-explorer-operations.md)
and [native scrolling](NATIVE_SCROLL_VERIFICATION.md) are the latest pre-audit
milestone records. Their reported counts, binary sizes and measurements belong
to those recorded source states and environments. A later change requires its
own verification; neither a plan nor CI configuration proves a completed run.

Earlier records preserve the evidence for their corresponding increments:

- [Stage 1](verification/stage-1.md)
- [Commander controls](verification/commander-controls.md)
- [Mouse activation](verification/mouse-activation.md)
- [Compact tooltips](verification/compact-tooltips.md)
- [Navigation/appearance](verification/navigation-and-appearance.md)
- [Selection/continuous sizes](verification/selection-and-continuous-sizes.md)
- [Native controls/full path](verification/native-controls-and-full-path.md)
- [Compact command/size feedback](verification/compact-command-and-size-feedback.md)
- [Panel search/reuse](verification/panel-search-and-reuse.md)
- [Slint code-generation analysis](verification/slint-code-generation-size.md)
- [Panel navigation/details](verification/panel-navigation-and-smooth-scrolling.md)
- [Panel interaction/layout](verification/panel-interaction-and-layout.md)

Keep historical `fm-*` package names, `target/`/`.tools/` paths, test totals and
recorded timings as evidence of those snapshots. They are not current build
instructions. New generated output and QA evidence belong under `_build/`;
toolchain/download caches belong under `.tools/`. Untracked local logs cannot
be assumed present in a fresh clone. Native Windows captures, headless Linux
UI tests and Linux backend checks establish different kinds of evidence;
none implies separately qualified Mint/macOS or native Linux desktop visuals.

The corrected native measurement method in the scrolling record supersedes
the notifier-based resize timings in the interaction record. Generated Rust
text size, executable size, CPU time, render submission and display latency
are different measurements and must remain identified separately.
