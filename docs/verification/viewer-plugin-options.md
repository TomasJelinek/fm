# Viewer plugin options

Options > Plugins lists the executable-relative viewer packages discovered by the
host runtime. Each row shows its description, stable ID, version, package location
and current load status. Invalid packages show their diagnostic and cannot be
enabled. Refresh performs bounded backend discovery without loading plugin code;
catalog errors clear old availability rows instead of leaving stale statuses.

Checkboxes edit the disabled viewer IDs in preferences version 10. Versions 1–9
migrate with no disabled IDs. IDs follow the same ASCII identifier rules as viewer
manifests, must be unique, and have limits of 64 IDs and 128 bytes per ID. Missing
disabled packages remain visible so their saved disabled preference can be removed.
Changes use the existing settings save/retry behavior and show a restart notice.

The presenter captures the saved disabled-ID list once when startup preferences
load successfully. Every viewer launch uses that immutable snapshot. Editing the
checkboxes affects the next application start; it neither changes existing viewers
nor silently enables a plugin during the current session. Viewer launch is withheld
until saved preferences are available. The Plugins tab uses a separate Slint global
and component, keeping catalog rows out of MainWindow's property list.

Evidence is under `_build/qa/viewer-implementation/editor/`:

- `plugins-settings-red.log`: version-10 roundtrip initially failed because the
  panel-filter migration branch still stopped at version 9.
- `settings-v10.log`: the complete common settings/runtime suite passes after all
  version branches and migration fixtures were updated.
- `plugins-catalog-red.log`: catalog discovery initially dropped an invalid package
  instead of publishing its diagnostic or installation-security refusal.
- `plugins-ui-red.log`: startup routing and next-start preferences tests initially
  failed, including premature launch and invalid/unknown toggle acceptance.
- `plugins-ui-green.log`, `plugins-catalog-green.log`, and `options-integration.log`
  record the focused presenter, backend, and real Slint control checks.

The host runtime owns manifest, installation-permission and native-load validation.
The Options catalog does not execute a package to validate it. A package with a
valid manifest may still report a native ABI or loading failure when first opened;
the load status distinguishes an available manifest from a loaded library.
