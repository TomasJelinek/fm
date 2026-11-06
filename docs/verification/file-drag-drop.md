# File drag/drop verification

This supplements the [design](../superpowers/specs/2026-11-06-file-drag-drop-design.md)
and [security contract](../../SECURITY.md#file-drag-and-drop). Native protocol tests
are distinct from UI gesture tests and from verified-copy integrity tests.

## Reproducing native Linux checks

Install the README build prerequisites plus `xvfb` and `xauth`. Run from the repo
root on Linux with the pinned toolchain:

```sh
xvfb-run -a -s "-screen 0 1280x1024x24 -nolisten tcp" \
    env MIDNIGHT_DND_TEST_ISOLATED=1 \
    cargo test -p midnight-platform-linux --lib --locked \
    file_drag_drop:: -- --ignored --test-threads=1
```

These tests deliberately use XTEST input on their private X server. Do not set
`MIDNIGHT_DND_TEST_ISOLATED=1` on your working desktop display. The repository's
Linux CI includes this isolated invocation; ordinary `cargo test --workspace`
alone skips these native scenarios. Xvfb is a test dependency, not an application
runtime requirement.

Checks exercise verified proxy installation/restoration and removal of the host's
legacy XDND advertisement; real incoming and outgoing peers; raw filename bytes;
bounded incremental transfers; stale destinations, rejected admission and changed
selection ownership; Escape; delayed release of the UI connection's pointer grab;
and Copy negotiation when the source proposes Move but explicitly permits Copy.
The self-proxy scenario transfers 1,000 paths through INCR.

The 2026-11-06 local platform run passed **117 tests** (106 library, 3 metadata,
8 transfer), including all **10 native drag/drop scenarios**, with zero skipped or
failed tests. Strict platform Clippy also passed. This run used isolated Xvfb
`:97`, Ubuntu 24.04.4/WSL2 and a local Czech locale fixture. Logs:
`_build/qa/drag-drop/linux-package-final.log` and `linux-clippy-final.log`.
RED/GREEN evidence also records proxy-advertisement, Copy-negotiation and
cross-connection pointer-handoff regressions. The full workspace run is recorded
separately once integration completes.

## Reproducing native Windows checks

Normal native unit tests include real COM file objects and a production session
registered against a window owned by another STA:

```powershell
cargo test -p midnight-platform-windows --lib --locked file_drag_drop::
```

The registration test marshals the actual IDropTarget and verifies that payload
admission occurs on the backend STA, then checks safe revocation/re-registration.
It does not move the mouse, change clipboard ownership or copy files. Other tests
cover raw unpaired UTF-16, malformed/oversized payloads, target coordinates,
refusal/admission effects, lifetime cancellation and swapped primary mouse-button
mapping without changing the machine setting.

An opt-in modal-loop smoke test briefly creates a tiny non-activating window on
an interactive test desktop. It invokes the real OLE loop with a test input source
delegating continuation policy to the production source; the target records
admission only. It injects no physical input and performs no filesystem operation:

```powershell
cargo test -p midnight-platform-windows --lib --locked `
    native_modal_copy_and_idle_cancellation -- --ignored --nocapture
```

The final native source harness passed **7 ordinary tests**. The explicit desktop
modal smoke also passed (0.64 seconds). The initial idle-cancellation watchdog
failure exposed a real OLE idle-loop problem: a timer alone does not reevaluate
the continuation callback. Cancellation now posts a wake only to the worker's
owned helper window; teardown drains that helper's pending wake before another
drag can start. No physical keyboard input is injected. The swapped-button and
stale-wake regressions were observed failing before their fixes and passing after.
Logs: `_build/qa/dnd/native-harness-green.log`, `windows-modal-green.log`,
`cancel-wake-red.log`, and `primary-input-{red,green}.log`.

## UI, queue and application integration

The first Windows release UI run passed **303 checks**: 299 library tests, a gesture
test across Detailed/Brief/Thumbnails, the full mouse regression and two compact
header/long-path checks. A later focused run passed four installed-bridge checks
plus nine Commander edge checks: internal fallback over a folder row uses the
containing panel, release/Escape/navigation prevent late revival, swapping/zoom/
modal UI update regions, and touch/right-button movement does not start dragging.
The highlighted full-panel copy target was visually reviewed.

Independent review identified pending-admission ordering, cancellation-before-
enqueue, transient-native-failure retry and swapped primary-button regressions.
The subsequent release run passed **306 checks**: 300 library tests, four installed
drag tests and two public queue regressions. Later clipboard copies cannot
overtake an acknowledged pending drop, and cancellation rejects its reserved token
before a backend job exists. Transient native failure permits a fresh native
gesture. The engine source harness passed **151 tests**, with one intentional
skip, including eight drag/drop and shared-copy-budget regressions. A bounded
cancelled-history regression reproduced 20 terminal jobs against the limit of 16;
the final history fix passes in both OS integration runs. The final Windows
release UI run passed **310 tests**: 301 library, four installed drag checks,
three public queue/history regressions, one footer and one eight-theme test.
An additional operation-ID exhaustion regression verifies rejection without
creating a job, wrapping the counter or admitting a replayed token. The current
Linux library/affected integration run passed **606 tests**, with 15 intentional
skips; the separate native protocol checks above cover the opt-in transport tests.
Independent source review found no remaining code issue.

The exact final Windows release application passed the filesystem and default
Direct3D UI smokes, including snapshot review and unchanged native preferences.
Its embedded moon icon's nine resources match the source ICO frame hashes.
Final integrated evidence and the broad-run contrast correction are recorded in
[the integration checkpoint](2026-11-06-themes-and-commander-tools.md#final-integration-regression-checks).
Logs include `_build/qa/clean-rebuild/windows-final-ui-tests.log` and
`linux-final-affected-tests.log`.

The fresh release build, graphics backend diagnosis and startup memory observation
are recorded in [the rebuild checkpoint](2026-11-06-themes-and-commander-tools.md#clean-release-and-requirements-checkpoint).
Native Wayland external transport, legacy X11 clients without proxy support,
Move/Link operations and another application's copy integrity are outside the
implemented transport contract; internal panel copying remains available.
