# Panel controls, path markers and error details

Targeted Windows software-renderer checks, 9 November 2026. These checks do not
establish native GPU performance or qualify the final packaged application.

Each panel now owns its Filter control immediately before View, plus Network
Neighborhood on its drive bar. Clicking either in the inactive panel activates
and addresses that panel. Shared toolbar actions retain their active-panel
semantics. Network remains disabled when its provider is unavailable.

Panel errors provide Copy error details and Dismiss message at normal and
320×200 sizes. Copy sends the entire Unicode diagnostic through the backend
`CopyText` command; it does not dismiss the error or submit a filesystem/terminal
action. Compact controls preserve the minimum window layout.

Path omission uses literal `...`, preserving the complete raw value for editing,
navigation and clipboard actions. The breadcrumb's leading container previously
centered itself inside the prefix-plus-marker overlay, overlapping the dots.
Checking the text value and bounding box missed that rendering fault. A new
pixel regression counted only two painted groups before the fix, and three after
anchoring the leading container at the overlay origin. The command prompt starts
with its trailing path and `>` visible and retains a leading path fragment with
an explicit marker; horizontal scrolling remains available.

The command/footer height check still selected Options after that button moved to
the top toolbar. It now measures the actual Jobs footer control; no command-row
height change was needed for that failure.

Verification:

- `cargo test -p midnight-ui --test path_elision --test native_controls
  --test command_line_layout --test error_notice --test error_notices
  --no-fail-fast --offline --locked -j2`: **16 passed**.
- Earlier per-panel controls gate: `network_navigation` **2**, `panel_toolbar`
  **2**, `shared_toolbar` **10**, all passed. Includes inactive/swapped panel
  routing and stale/cancelled control gestures.
- `cargo test -p midnight-platform-windows --lib locale::tests`: **5 passed**.
- Native WSL Ubuntu `cargo test -p midnight-platform-linux --lib locale::tests
  --offline --locked -j2`: **5 passed**. Calendar-only archive dates use the
  platform locale without inventing a timezone; invalid dates are rejected.

Logs and render artifacts remain under `_build/delete-followups/` and
`_build/qa/provider-ui/render/`.

The coherent debug application built successfully and was copied with complete
viewer/thumbnail/native/network packages and the new archive worker into a fresh
private candidate installation:
`C:/Users/tjelinek/MidnightExplorer-20261109-provider-candidate`.
Application SHA-256:
`E20D92F39DA66E7F2AA6EE877B56BA1D6554D9E8CED28F3817B3E2AD28EB5698`.
Both headless and software UI startup smoke checks exited zero, with unchanged
saved preferences and desktop shortcut. The resulting screenshot was inspected:
both populated panels, themed shared toolbar, per-panel Network/Filter/View/Sort,
command row and footer were present without a startup error. Evidence and the
repeatable script are under `_build/qa/provider-runtime/`.

This candidate retains the previous native Windows provider as the performance
baseline; the separately tested cloud-handle robustness change and thumbnail
performance fixes are not included. Full native GPU/viewer interaction acceptance,
archive folder-size counting, and the complete provider action audit remain
separate work. The build still reports 13 pre-existing unused tree-integration
warnings; it is not recorded as a warning-free build.
