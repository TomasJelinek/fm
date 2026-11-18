# HTML viewer idle wakeups — 18 November 2026

Real-Chromium qualification on Windows of the event-driven web viewer host session
(design WP4-5) and of CEF's external message pump in the HTML provider (WP4-10a). The
host change is adopted. The external pump did not reach its acceptance bar and is not
used; the provider is unchanged. Linux was not measured (WP4-10b, backlog VIEW-04).

## Setup

- Windows 11 Enterprise 26200, CEF 156.0.26 debug provider package (413 MB, 535 files)
  built offline with `scripts/build-html-viewer.ps1` from the pinned, checksum-verified
  minimal archive.
- Hosts: the `html_idle`, `probe` and `html_wheel_latency` examples built from the
  baseline session code (`93d36411`) and from the event-driven one (`b23a535e`).
  SHA-256 of `html_idle.exe`: baseline `5A2BDFBC…14AC`, event-driven `6C2D0764…EA5B`.
- Providers (`midnight-viewer-html.dll`): baseline `631B6047…C729`; external pump with
  cefclient's 33 ms clamp and fallback `99D04E17…2EC1`; `diagnostic-timing` builds
  that trace every pump, without (`9B3D03AE…C1DC`) and with (`942204C6…B5BC93`) the
  external pump. The diagnostic build without it runs the baseline's 2 ms loop.
- Installation: a private-ACL directory under the user profile with an empty `me.exe`
  and the package under `viewers/`. The checkout's D: ancestors grant Authenticated
  Users Modify, so discovery refuses an installation there. Providers were swapped by
  copying the DLL, retrying until the previous browser tree released the file, and
  verifying its hash before each run. The installation was removed afterwards.
- Owned fixtures: a static page (text and a colored block, no script, animation or
  focused input), the probe page (two "Chromium needle" texts and a script that writes
  "Script executed"), a script-free wheel page that starts with an opaque `#123456`
  section, a 60 fps CSS transform animation and a 100 ms `setInterval` page.
- The machine was shared with concurrent Cargo builds, Defender scanning and a WSL VM;
  it was not quiet. Variants ran interleaved, so the load affected both alike.
- Metrics: per-thread context switches and CPU time (`NtQuerySystemInformation`) at the
  example's `idle_start` and `idle_end` lines, and the provider's own trace, which
  records one `pump_turn` per `CefDoMessageLoopWork`.

## Host session (adopted)

Static fixture, 10 s idle after the first frame, 800 × 600 at scale 1, baseline
provider, three interleaved rounds:

| Round | Baseline host: switches/s (CPU) | Event-driven host: switches/s (CPU) |
| --- | ---: | ---: |
| 1 | 172.8 (1.72 %) | 0.7 (0 %) |
| 2 | 175.0 (0.31 %) | 0.9 (0 %) |
| 3 | 173.7 (0.31 %) | 0.8 (0 %) |

The baseline's actor waits 5 ms and its supervisor sleeps 10 ms; the event-driven host
blocks until there is work. The remaining switches come from the example's own
threads. One baseline close took 24.5 s (profile removal under load); the other
closes took 0.3–0.7 s for both hosts.

`html_wheel_latency` with the wheel fixture at 2705 × 1550 and scale 2.25 (the
[frame-pacing](2026-11-12-html-frame-pacing.md) viewport), three interleaved rounds of
12 gestures per host: pooled median wheel-to-first-changed-frame latency 96.9 ms for
the baseline host (quartiles 82.6 / 415.2 ms) and 87.4 ms for the event-driven host
(65.7 / 430.0 ms). Per-round medians were 96.0, 222.0 and 96.9 ms against 95.2, 73.4
and 165.0 ms. The event-driven host is not slower; the long upper quartiles in both
are the machine load.

`probe` (Find, offline script denial, online scripts in a recreated context, a
fractional-DPI resize and close) passed with both hosts: first content 1593 ms and
close 3444 ms with the baseline host, 1130 ms and 370 ms with the event-driven host
(single runs).

## Provider external message pump (not adopted)

Pumps per second from the provider's trace, static fixture:

- 2 ms loop: 63–74 per second throughout; consecutive pumps are 15.6–16 ms apart, the
  Windows timer period that the 2 ms timeout rounds up to. Four paired runs measured
  64–65 per second over their last 3 s.
- External pump: 21–84 per second during the first 8 s after start (CEF-requested
  bursts 0–6 ms apart between 47 ms gaps), then a steady 21–23 per second: 47 ms
  apart, the 33 ms bound rounded up to three timer periods.
- Over 20 s idle the browser process made 210 context switches per second with 1.87 %
  CPU on the 2 ms loop, and 144 per second with 2.03 % CPU with the external pump.

That is about 3x fewer main-loop pumps once idle and 1.5x fewer browser-process
switches, with no CPU saving, below the required 5x. The provider therefore keeps its
2 ms pump on both platforms. Because the decision failed on idle wakeups, the
animation, script-timer, wheel and opt-in `native_runtime` comparisons were not run
for that candidate. The trace does not record the delays CEF requests, so it cannot
tell whether the steady 47 ms pumps are the fallback or clamped CEF requests; that
decides whether a longer idle bound could help (backlog VIEW-04).

## Measurement notes

The switch sampler was checked against a 5 ms sleep loop (71.8 per second) and against
.NET thread CPU times. Its browser-process figures nevertheless disagreed with the
provider's trace in several runs (0.5–2.5 switches per second while the trace of the
same run showed about 64 pumps per second), and early runs also used a stale provider
when a DLL swap was refused while the previous browser tree was still exiting. Those
runs were discarded; provider conclusions use the trace counts. The host figures come
from fresh example processes and need no swap.

Local evidence is under `_build/qa/html-pump/` and `_build/qa/html-host/`:
`measure.ps1`, `switches.ps1`, `locate.ps1`, `run-host-ab.ps1`,
`build-diagnostic.ps1`, `host-idle.jsonl`, `host-ab.jsonl`, `trace-*.csv`,
`wheel-*.log`, `probe-*.log`, the fixtures, the example executables and
`wp4-10a-external-pump.patch` (the unadopted provider change).
