# Documentation map

This index covers Midnight Explorer 0.2. Current feature availability and known
limitations are tracked in [Feature status](FEATURE_STATUS.md); dated design
plans and old verification reports do not establish current functionality.

## Product and developer guidance

| Subject | Entry point |
| --- | --- |
| Build, install, run, platforms and output locations | [Project README](../README.md) |
| Implemented features and remaining work | [Feature status](FEATURE_STATUS.md) |
| Current default shortcuts and configurable actions | [Keyboard reference](KEYBOARD_SHORTCUTS.md) |
| Security, Unicode, identity, cancellation and integrity | [Security requirements](../SECURITY.md) |
| Rust style, platform separation and source-size limits | [Code style](../CODE_STYLE.md) |
| Public APIs, plugins and internal contracts | [Developer documentation](developer/README.md) |
| Dependency selection and free-use requirements | [Dependency policy](DEPENDENCY_POLICY.md) |
| Current pinned libraries, features and licenses | [Third-party inventory](THIRD_PARTY.md), [machine-readable inventory](dependencies/cargo-licenses.csv) |
| Release optimizations and qualification limits | [Release build](RELEASE_BUILD.md) |
| Native and inline panel navigation/actions | [Tree panel contract](developer/TREE_PANEL.md) |
| Copy/Move integrity and limitations | [Transfers](developer/TRANSFERS.md) |
| Disk operations | [Maximum parallel background disk work](DISK_OPERATIONS.md) |
| File coloring and default palette | [Filename colors](developer/FILENAME_COLOR_OPTIONS.md) |
| Parallel targeted test execution | [Test execution](developer/TEST_EXECUTION.md) |
| Agent roles and coordinated integration | [Agent workflow](developer/AGENT_WORKFLOW.md) |

## Designs and historical evidence

The [main architecture](superpowers/specs/2026-11-04-dual-pane-file-manager-design.md)
is extended by later dated specifications. Later accepted requirements supersede
earlier descriptions. Consult current contracts before implementing an old plan.

The [10–11 November main-branch audit](verification/main-audit.md) and its CSV
measurements describe their named source commits. The older
[dependency capture](third-party-inventory.csv) belongs to that audit; it is not
the current plugin-inclusive inventory. Audit corrections were reconciled with
the later viewer/plugin branch during the 12 November integration.

The [12 November panel interaction qualification](verification/2026-11-12-panel-interaction-polish.md)
records the tree-admission, captured actions, error-dialog and visual checks for
that batch, including its explicit limitations. Other files under `verification/`
remain evidence for the exact commits and environments stated in each report.
Neither CI configuration nor a checked plan is proof of a successful test run.
