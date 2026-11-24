# Progress heartbeat 250 ms → 125 ms — 2026-11-24

## Change

Copy, Move and Delete sample their activity and publish their live status on one heartbeat
(`CADENCE`, `operation_throughput.rs`), which the progress windows, the Jobs list, the
speed chart and the per-file bar follow. It was 250 ms (4 Hz); it is now 125 ms (8 Hz).
Nothing else assumed the old length: the remaining-time averages weigh intervals by their
duration (`SMOOTHING`, 8 s), the chart's scale follows the last minute of samples by time
(`job_speed.rs`), and the chart keeps at most 240 intervals of the whole job, combining
them as before (it reaches its first combination after 30 s instead of 60 s). The UI has
no throttle of its own: each heartbeat wakes it once.

## Method

Windows 11, NTFS on a local SSD, one export of the code, with other agents' builds and
tests running on the machine throughout (so absolute numbers drift by up to ±30 % between
runs). A release build (opt-level 3, thin LTO, debug assertions on) carried a temporary
override of the heartbeat for the measurement only, so both cadences ran from the same
binary; the committed code has the 125 ms constant and no override.

- Engine and throughput: `copy_operations::tests::cadence_measure::measure_progress_cadence`
  (ignored measurement test) copies a folder through the copy actor with the
  application's event channel (32 events) and disk-I/O limit (4), draining events as the
  UI does; the process CPU time came from the exited process. Cases: a 1 GiB file; 100,000
  files of 1 KiB in 100 folders; two jobs at once, each a 384 MiB file and 2,000 files of
  1 KiB. Runs alternated the cadences.
- UI thread: `native_job_windows::measure_progress_event_cost` (ignored, midnight-ui
  integration tests, release) installs the real presenter and job windows, opens the Jobs
  window and each job's progress window, and per heartbeat applies a running job's
  progress (240-sample chart) and software-renders every open window; the cost per
  heartbeat times the heartbeat rate is the UI thread's share. The application renders on
  the GPU, so this is an upper bound.

The real application could not be driven for this: its windows expose no UI Automation
tree and synthetic keystrokes reach whatever window is in front on a shared desktop.

## Results

### UI thread (software rendering, upper bound)

| Open windows | Cost per heartbeat | At 4 Hz (250 ms) | At 8 Hz (125 ms) | Added |
|---|---|---|---|---|
| Jobs + 1 progress window, 1 job | 3.6–5.1 ms | 1.4–2.0 % of a core | 2.9–4.1 % | +1.4–2.0 % |
| Jobs + 2 progress windows, 2 jobs | 6.1–6.8 ms (both jobs) | 2.4–2.7 % | 4.9–5.4 % | +2.4–2.7 % |

### Engine, throughput and process CPU (copy actor, release)

| Case | Cadence | Runs | Wall time (s) | Throughput | Process CPU (s) | Events per job per s |
|---|---|---|---|---|---|---|
| 1 GiB file | 250 ms | 4 | 3.4–6.1 (mean 4.5) | 167–298 MiB/s | 2.72–3.12 (mean 2.98) | 4.6–4.9 |
| 1 GiB file | 125 ms | 4 | 3.4–4.9 (mean 4.2) | 209–301 MiB/s | 2.72–3.20 (mean 3.00) | 8.6–9.3 |
| Two jobs | 250 ms | 4 | 12.2–18.3 (mean 15.2) | 219–329 files/s | 19.4–24.2 (mean 22.3) | 4.1–4.2 |
| Two jobs | 125 ms | 4 | 12.5–17.0 (mean 14.1) | 235–322 files/s | 17.6–23.3 (mean 20.1) | 8.2 |
| 100,000 files | 250 ms | 4 | 307–474 (mean 421) | 211–326 files/s | 258–338 (mean 313) | 4.0 |
| 100,000 files | 125 ms | 3 | 381–516 (mean 467) | 194–262 files/s | 329–377 (mean 358) | 8.0 |

(A fourth 125 ms run of 100,000 files collided with the cleanup of the run before it and
is left out.)

### 100,000 files repeated in interleaved pairs

Because the first 100,000-file runs used more process CPU at 125 ms, the case was repeated
in four pairs from the same release binary, alternating which cadence ran first
(250/125, 125/250, 125/250, 250/125) so that drift between runs falls on both cadences
alike. Each row also records the process's user and kernel time, the machine's average
CPU load during the run (sampled every 10 s, including the run itself) and the most
compiler and linker processes other agents ran at once.

| Pair | Order | Cadence | Wall time (s) | Files/s | Process CPU (s) | User (s) | Kernel (s) | Machine load | Other builds |
|---|---|---|---|---|---|---|---|---|---|
| 1 | first | 250 ms | 378 | 264 | 264.6 | 30.4 | 234.2 | 25 % | up to 4 |
| 1 | second | 125 ms | 217 | 460 | 196.2 | 27.2 | 169.0 | 12 % | none |
| 2 | first | 125 ms | 239 | 419 | 200.3 | 25.5 | 174.8 | 16 % | none |
| 2 | second | 250 ms | 262 | 381 | 200.3 | 23.7 | 176.7 | 9 % | none |
| 3 | first | 125 ms | 289 | 346 | 200.6 | 30.4 | 170.2 | 21 % | none |
| 3 | second | 250 ms | 199 | 502 | 178.5 | 26.5 | 152.0 | 15 % | none |
| 4 | first | 250 ms | 320 | 312 | 255.4 | 34.9 | 220.5 | 39 % | up to 12 |
| 4 | second | 125 ms | 303 | 330 | 266.1 | 40.7 | 225.4 | 51 % | up to 21 |

The 125 ms run's process CPU differed from its pair's 250 ms run by −68.4 s, 0.0 s,
+22.1 s and +10.7 s (means 215.8 s at 125 ms and 224.7 s at 250 ms). In the two pairs
without other builds the difference was 0.0 s and +22.1 s. In pair 3 nearly all of that
difference (18 s) was kernel time spent creating and writing files, which the heartbeat
does not do; user time differed by 1.8–3.9 s in those pairs. Kernel time is 85–89 % of the
process's CPU in every run, and the runs with other builds are the slowest at either
cadence. Every 125 ms run published 8.0 events per second and every 250 ms run 4.0.

## Conclusion

Accepted at 125 ms. The UI thread's added share is about 1.5–2.7 % of one core with the
Jobs and progress windows open, measured with software rendering, so less with the GPU
renderer. The large-file and two-job runs show no throughput or CPU difference beyond the
machine's drift. The higher process CPU of the first 100,000-file runs at 125 ms did not
repeat in interleaved pairs: the differences changed sign, followed the machine's load
and lay in kernel file work rather than in the heartbeat's own user-mode work (one
sample and one status snapshot per job per heartbeat). Current speed labels follow
125 ms intervals and so vary more from one heartbeat to the next; the remaining-time
estimate does not.
