# Documentation map

This index lists the current documents of Midnight Explorer 0.6.3. Start with the
[project README](../README.md) to install and run the application; when something goes
wrong, see [Troubleshooting](troubleshooting.md), and look up project terms in the
[glossary](glossary.md). What is implemented is in [feature status](feature-status.md), the remaining work in the
[implementation backlog](implementation-backlog.md) and the work in flight in
[active work](active-work.md). Dated records describe the commit and environment named
in them; they are evidence, not current documentation, and a design or plan does not
show that a feature exists.

## Using Midnight Explorer

| Document | Purpose |
| --- | --- |
| [Project README](../README.md) | Overview, downloads, the first run, building from source, releases, support and the license. |
| [Troubleshooting](troubleshooting.md) | The problems of a first start and their messages: missing runtime, denied folders, refused updates and plugins, renderer and Linux issues, and how to get help. |
| [Glossary](glossary.md) | The project's terms, such as panel, Hot Paths, spool, worker and MSI installation. |
| [User guide](user-guide.md) | Building, installing and running the application, and its controls. |
| [Feature status](feature-status.md) | What each feature area implements on Windows and Linux, and where it is verified. |
| [Keyboard shortcuts](keyboard-shortcuts.md) | The default shortcuts, the key-combination syntax and every configurable action ID. |
| [System requirements](system-requirements.md) | Supported systems, required packages, runtime prerequisites and environment variables. |
| [Export and import configuration](settings-files.md) | Backing up and moving settings, and the settings file format. |
| [Archive formats](archive-formats.md) | Archive formats, panel associations, passwords and operation limits. |
| [Viewer file types](viewer-associations.md) | Options › Plugins: which viewer, filesystem, archive format and thumbnailer handles a file type. |
| [Make File List](make-file-list.md) | Exporting a selection's details with templates (Ctrl+M). |
| [Drive information](drive-information.md) | The Drive information window (Ctrl+F1 or Ctrl+L). |
| [Disk operations](disk-operations.md) | The Disk I/O concurrency setting. |
| [Limits](limits.md) | Every window count, file size, item count and memory limit, in one reference. |

## Status and planning

| Document | Purpose |
| --- | --- |
| [Active work](active-work.md) | The current checkpoint, the work in flight and where evidence is kept. |
| [Implementation backlog](implementation-backlog.md) | Open work and known limits, by stable ID. |
| Release notes | The notes of each preview release: [0.1.0](releases/0.1.0.md), [0.5.0](releases/0.5.0.md), [0.5.1](releases/0.5.1.md), [0.5.2](releases/0.5.2.md), [0.5.3](releases/0.5.3.md), [0.5.4](releases/0.5.4.md), [0.6.0](releases/0.6.0.md), [0.6.1](releases/0.6.1.md), [0.6.2](releases/0.6.2.md) and [0.6.3](releases/0.6.3.md). |

## Security, dependencies and provenance

| Document | Purpose |
| --- | --- |
| [Security requirements](../SECURITY.md) | Security, Unicode, identity, cancellation and integrity requirements. |
| [Code style](../CODE_STYLE.md) | Rust style, platform separation, the source-size limit and the checks. |
| [Dependency policy](dependency-policy.md) | Rules for third-party dependencies, including no mandatory fees. |
| [Third-party libraries](third-party.md) | Third-party code and data with their versions and licenses. |
| [Direct dependencies](dependencies/direct-dependencies.md) | The direct third-party dependencies of each workspace. |
| [Cargo library inventory](dependencies/cargo-libraries.md) | Every third-party Cargo package that the dependency trees select. |
| [Lockfile entries outside the inventory](dependencies/lockfile-only.md) | Lockfile entries that the inventory leaves out, and why. |
| [Machine-readable inventory](dependencies/cargo-licenses.csv) | The Cargo package inventory as CSV. |
| [Office preview notices](dependencies/office-preview-notices.md) | Dependency notices of the Office contents preview. |
| [Executable-size diagnostic](dependencies/cargo-bloat.md) | The optional `cargo-bloat` diagnostic. |
| [Asset provenance](asset-provenance.md) | Where the application's artwork and runtime icons come from. |
| [Action icon artwork](action-icon-provenance.md) | The origin of the command icons. |
| [Theme provenance](theme-provenance.md) | The origin of the theme palettes and their contrast contract. |
| [Theme licenses](licenses/themes/) | License texts of the palettes that themes adapt. |

## Building, testing and development

| Document | Purpose |
| --- | --- |
| [Release builds](release-build.md) | Release builds, their optimization settings and qualification limits. |
| [Developer guide](developer/README.md) | The plugin APIs and the internal contracts, with an index of every developer document. |
| [Test execution](developer/test-execution.md) | Validation scripts, CI jobs and parallel targeted tests. |
| [Acceptance suite](developer/acceptance.md) | The opt-in end-to-end suite; its area records are in [developer/acceptance](developer/acceptance/). |
| [Agent workflow](developer/agent-workflow.md) | Roles and the working agreement for coordinated development. |
| [Archive worker development](archive-filesystem-development.md) | Building and testing the independent archive worker plugin. |

## Dated records

| Location | Purpose |
| --- | --- |
| [verification/](verification/) | Dated verification records; each describes the commit and environment it names. |
| [Native wheel scrolling investigation](verification/2026-11-11-native-wheel-scrolling.md) | Measurements of the native wheel scrolling work of 11 and 21 November (a dated record). |
| [superpowers/specs/](superpowers/specs/) and [superpowers/plans/](superpowers/plans/) | Dated designs and implementation plans; later accepted requirements supersede them. The [main architecture](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md) is the first. |
| [Main-branch audit](verification/main-audit.md) | The audit of 10–11 November with its CSV measurements; the [dependency capture](third-party-inventory.csv) belongs to it and is not the current inventory. |
