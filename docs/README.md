# Documentation map

This index lists the current documents of Midnight Explorer 0.5. What is implemented is
in [feature status](FEATURE_STATUS.md), the remaining work in the
[implementation backlog](IMPLEMENTATION_BACKLOG.md) and the work in flight in
[active work](ACTIVE_WORK.md). Dated records describe the commit and environment named
in them; they are evidence, not current documentation, and a design or plan does not
show that a feature exists.

## Using Midnight Explorer

| Document | Purpose |
| --- | --- |
| [Project README](../README.md) | Overview, downloads, building from source and the license. |
| [User guide](USER_GUIDE.md) | Building, installing and running the application, and its controls. |
| [Feature status](FEATURE_STATUS.md) | What each feature area implements on Windows and Linux, and where it is verified. |
| [Keyboard shortcuts](KEYBOARD_SHORTCUTS.md) | The default shortcuts and the configurable actions. |
| [System requirements](SYSTEM_REQUIREMENTS.md) | Supported systems, required packages and runtime prerequisites. |
| [Export and import configuration](SETTINGS_FILES.md) | Backing up and moving settings, and the settings file format. |
| [Archive formats](ARCHIVE_FORMATS.md) | Archive formats, panel associations, passwords and operation limits. |
| [Viewer file types](VIEWER_ASSOCIATIONS.md) | Options › Plugins: which viewer, filesystem, archive format and thumbnailer handles a file type. |
| [Make File List](MAKE_FILE_LIST.md) | Exporting a selection's details with templates (Ctrl+M). |
| [Drive information](DRIVE_INFORMATION.md) | The Drive information window (Ctrl+F1 or Ctrl+L). |
| [Disk operations](DISK_OPERATIONS.md) | The Disk I/O concurrency setting. |
| [Limits](LIMITS.md) | Every window count, file size, item count and memory limit, in one reference. |

## Status and planning

| Document | Purpose |
| --- | --- |
| [Active work](ACTIVE_WORK.md) | The current checkpoint, the work in flight and where evidence is kept. |
| [Implementation backlog](IMPLEMENTATION_BACKLOG.md) | Open work and known limits, by stable ID. |
| [0.1.0 release notes](releases/0.1.0.md) | The notes of the 0.1.0 preview release. |

## Security, dependencies and provenance

| Document | Purpose |
| --- | --- |
| [Security requirements](../SECURITY.md) | Security, Unicode, identity, cancellation and integrity requirements. |
| [Code style](../CODE_STYLE.md) | Rust style, platform separation, the source-size limit and the checks. |
| [Dependency policy](DEPENDENCY_POLICY.md) | Rules for third-party dependencies, including no mandatory fees. |
| [Third-party libraries](THIRD_PARTY.md) | Third-party code and data with their versions and licences. |
| [Direct dependencies](dependencies/DIRECT_DEPENDENCIES.md) | The direct third-party dependencies of each workspace. |
| [Cargo library inventory](dependencies/CARGO_LIBRARIES.md) | Every third-party Cargo package that the dependency trees select. |
| [Lock file entries outside the inventory](dependencies/LOCKFILE_ONLY.md) | Lock file entries that the inventory leaves out, and why. |
| [Machine-readable inventory](dependencies/cargo-licenses.csv) | The Cargo package inventory as CSV. |
| [Office preview notices](dependencies/OFFICE_PREVIEW_NOTICES.md) | Dependency notices of the Office contents preview. |
| [Executable-size diagnostic](dependencies/CARGO_BLOAT.md) | The optional `cargo-bloat` diagnostic. |
| [Asset provenance](ASSET_PROVENANCE.md) | Where the application's artwork and runtime icons come from. |
| [Action icon artwork](ACTION_ICON_PROVENANCE.md) | The origin of the command icons. |
| [Theme provenance](THEME_PROVENANCE.md) | The origin of the theme palettes and their contrast contract. |
| [Theme licences](licenses/themes/) | Licence texts of the palettes that themes adapt. |

## Building, testing and development

| Document | Purpose |
| --- | --- |
| [Release builds](RELEASE_BUILD.md) | Release builds, their optimization settings and qualification limits. |
| [Developer guide](developer/README.md) | The plugin APIs and the internal contracts, with an index of every developer document. |
| [Test execution](developer/TEST_EXECUTION.md) | Validation scripts, CI jobs and parallel targeted tests. |
| [Acceptance suite](developer/ACCEPTANCE.md) | The opt-in end-to-end suite; its area records are in [developer/acceptance](developer/acceptance/). |
| [Agent workflow](developer/AGENT_WORKFLOW.md) | Roles and the working agreement for coordinated development. |
| [Archive worker development](archive-filesystem-development.md) | Building and testing the independent archive worker plugin. |

## Dated records

| Location | Purpose |
| --- | --- |
| [verification/](verification/) | Dated verification records; each describes the commit and environment it names. |
| [Native wheel scrolling investigation](verification/2026-11-11-native-wheel-scrolling.md) | Measurements of the native wheel scrolling work of 11 and 21 November (a dated record). |
| [superpowers/specs/](superpowers/specs/) and [superpowers/plans/](superpowers/plans/) | Dated designs and implementation plans; later accepted requirements supersede them. The [main architecture](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md) is the first. |
| [Main-branch audit](verification/main-audit.md) | The audit of 10–11 November with its CSV measurements; the [dependency capture](third-party-inventory.csv) belongs to it and is not the current inventory. |
