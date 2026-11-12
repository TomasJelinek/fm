# Linux native file clipboard

The backend preserves native Unix path bytes using local `file:` URIs. It accepts
empty or `localhost` authorities, rejects remote authorities and malformed lists,
and reads explicit GNOME/KDE copy or move intent. The whole payload must fit the
4 MiB and 4096-item budgets; invalid or excessive lists fail without truncation.

## Desktop capabilities

| Session | Copy file paths | Read file paths |
| --- | --- | --- |
| Working X11 connection | Supported | Supported |
| Wayland with a working XWayland clipboard bridge | Supported through XWayland | Supported through XWayland |
| Pure Wayland, or no working X11 bridge | Unsupported | Supported when the compositor provides the required data-control protocol |
| No available display | Unsupported | Unsupported |

When `DISPLAY` is present, the adapter tries X11 first, including in a Wayland
session. The X11 owner runs in the existing backend clipboard session and is
serviced by its event pump. Incremental transfers have a bounded count, byte
budget and expiry. A delayed ownership-loss event is checked against the current
server owner before clearing locally stored paths. Clipboard reads have a deadline
that is checked even while unrelated native events keep arriving.

Pure Wayland copying returns an explicit `Unsupported` error. The currently used
`wl-clipboard-rs` copy helper starts a detached thread for each copy and can block
while writing to an unresponsive receiver. This adapter does not call that helper.
A future Wayland owner needs bounded lifetime and cancellation before native
copying can be enabled. Wayland reads retain the payload limit and a five-second
deadline while receiving data from the transfer pipe. A live Wayland compositor
has not been exercised by the native verification described below.

## Native regression harness

`_build/qa/panel-shell/clipboard-xvfb-smoke.sh` starts a separate Xvfb server and
runs the ignored `file_clipboard::x11::tests` tests with
`MIDNIGHT_CLIPBOARD_TEST_ISOLATED=1`. The tests run serially because they share that
server's clipboard. They never connect to or replace the user's desktop clipboard.

The native cases transfer a 1000-item raw-byte list using INCR, reacquire ownership
before processing an earlier `SelectionClear`, and flood the event queue after a
read deadline has expired. Ordinary unit tests cover URI and intent codecs plus
backend selection with and without `DISPLAY`, including pure Wayland copy refusal.

On 2026-11-06, the stale-clear regression failed against the previous behavior
with `The X11 file clipboard is empty`, then passed after the current-owner check.
All three native Xvfb cases passed together (0.17 seconds), and all six ordinary
clipboard unit tests passed. Local evidence is retained in
`_build/qa/panel-shell/clipboard-x11-reacquire-{red,green}.log` and
`_build/qa/panel-shell/clipboard-review-green.log`.
`cargo clippy -p midnight-platform-linux --all-targets --offline -- -D warnings`
also passed; its output is in `clipboard-review-clippy.log` in the same directory.
