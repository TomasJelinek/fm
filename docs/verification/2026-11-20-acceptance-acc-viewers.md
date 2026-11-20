# Acceptance suite: viewers, comparator, thumbnails and archives

Opt-in acceptance scenarios (`viewers_files`, `viewers_controls`, `viewers_compare`,
`viewers_thumbnails`, `viewers_archives`) that drive the installed viewer, comparator,
thumbnailer and archive plugins through keyboard and pointer input only, on the
generated fixture. How they run (staged child processes, composition, latency
definition) is in [ACCEPTANCE.md](../developer/ACCEPTANCE.md#viewers-comparators-thumbnails-and-archives).

Environment: the project's Docker build image (Slint software renderer, DejaVu fonts
only), debug test profile, plugin packages as installed in `_build/debug` (viewers,
`native.linux`, `midnight.fs.archive`, thumbnailers). The tree under test is the
integration snapshot of 2026-11-20 without the parallel viewer-performance changes.
The machine (32 CPUs) was shared with other agents' builds during every run (load
average about 8, many builds writing to the same disk), which affects the timings.

## Commands

All cargo commands ran in the build image with the workspace mounted read-only and a
shared target directory:

```text
cargo test --workspace --locked --config <agent profile overrides> \
    --features midnight-acceptance/acceptance --test acceptance --no-run
<target>/debug/deps/acceptance-<hash> --scale smoke viewers_
<target>/debug/deps/acceptance-<hash> --scale full
cargo clippy --workspace --locked --config <agent profile overrides> \
    --features midnight-acceptance/acceptance --test acceptance -- -D warnings
```

`<agent profile overrides>` only sets per-package `codegen-units` so that parallel
agents keep separate artifacts; it changes no code or feature. The test binary takes
scenario filters as plain arguments (`--only` belongs to `scripts/acceptance.sh`).

## Results

Smoke iterations (`--scale smoke viewers_`), each after a rebuild:

| Run | `viewers_files` | `viewers_controls` | `viewers_compare` | `viewers_thumbnails` | `viewers_archives` |
| --- | --- | --- | --- | --- | --- |
| `20261120-024144` | FAIL: V1 only (2 of 42 checks: `page.html`, `plain.htm`) | PASS (12 shots) | PASS (9 shots) | PASS (4 shots) | FAIL: V11 (member view stopped by the archive watchdog) |
| `viewers_archives` five more times, final build | | | | | PASS ×5 (26.4–27.7 s, 8 shots each) |

Clippy (`-- -D warnings`) on the acceptance target: three lints in
`viewers_support.rs` (two `type_complexity`, one `unnecessary_sort_by`) were fixed;
reruns, the last on the final code, finished clean (exit 0).

Full-scale runs of the whole suite (`--scale full`, all 14 scenarios):

| Run | Result | Viewer scenarios |
| --- | --- | --- |
| `20261120-030204` | 11 passed, 1 warned, 2 failed, 619 s | `viewers_files` FAIL (V1 only); `viewers_compare` FAIL: its 8 MiB log pair was refused by the 4 MiB text limit (V12), so the fixture and the check were changed; the other three PASS |
| `20261120-031658` (final code) | 12 passed, 1 warned, 1 failed, 585 s | `viewers_files` FAIL, V1 only (2 of 42 checks: `docs/page.html`, `docs/plain.htm`); `viewers_controls` PASS (12 shots), `viewers_compare` PASS (10), `viewers_thumbnails` PASS (4), `viewers_archives` PASS (8) |

The other nine scenarios passed in both full runs; `ui_review` warned with 19 layout
flags (10 overlapping controls, 9 texts that do not fit their buttons), which belong to
the UI review, not to these scenarios. In `viewers_files` the other 40 checks passed;
its failure message names finding V1.

## Open latency

F3 key event to the viewer's report of its first content frame. Window creation inside
the key event took 0–1 ms for every file, so the time is the backend's first frame.
Full scale is run `20261120-030204` (the final full run `20261120-031658` measured the
same within a few milliseconds: 35 other opens 192–258 ms, repeated opens 192–212 ms),
smoke is run `20261120-024144`:

| Kind | Files | Full scale | Smoke |
| --- | --- | --- | --- |
| Text (every encoding, CRLF, 200,000-character line, empty, no final newline) | `text/*` | 192–221 ms | 192–242 ms |
| Large log (8 MiB full, 1 MiB smoke) | `text/huge.log` | 225 ms; Ctrl+End to line 74,725: 466 ms | 225 ms; Ctrl+End 482 ms |
| Source code, Markdown | `code/*`, `docs/*.md` | 192–194 ms | 192–195 ms |
| PDF | `docs/report.pdf` | 322 ms | 337 ms |
| Images (PNG, JPEG, GIF, WebP, SVG; broken PNGs report their error) | `images/*` | 240–245 ms | 240–262 ms |
| Hex view (archives, binaries up to 24 MiB) | `archives/*`, `sizes/*` | 192–242 ms | 194–210 ms |
| 15 opens of a 9-byte text file | `text/no-final-newline.txt` | min 192, median 193, max 195 ms | min 192, median 193, max 210 ms |
| 15 opens of a 4 KiB binary | `sizes/block.bin` | min 208, median 210, max 224 ms | min 209, median 210, max 226 ms |
| Comparison verdicts (final run) | small text pairs; 3.5 MiB pair; images; refusals | 326–354 ms; 1,028 ms; 592 ms; 217–227 ms | 321–356 ms; (1 MiB pair) 643 ms; 591 ms; 224 ms |
| Archive root listing | ZIP, tar, tar.gz | 218–583 ms | 216–221 ms |
| F3 on an archive member | `bundle.zip/bundle/readme.txt` | 224 ms (final run 886 ms) | 23.7 s, failed (V11) |
| Thumbnails of the first 9 visible tiles | `thumbs/` | 523 ms | 537 ms |

Intermittent stalls in earlier smoke runs of the same binary and fixture (finding V2):
`text/huge.log` 6,729 ms and `code/main.rs` 1,765 ms (run `20261120-013613`);
`sizes/block.bin` 5,958 ms and `code/config.json` 626 ms (run `20261120-015415`);
`source.tar.gz` root listing 3,903 ms and an archive member 1,736 ms (run
`20261120-022401`). No viewer open over 500 ms in the later runs, including both
full-scale runs. Other steps over 500 ms in the final run: the 3.5 MiB comparison
(1,028 ms), the image comparison (592 ms), the first nine thumbnails (520 ms) and the
archive member view (886 ms).

## Findings

Screenshots are in each run's `shots/<scenario>/` folder; "file:line" points at the
suspected code, read but not changed. Gaps marked "documented" are limitations the
README or FEATURE_STATUS states; their scenarios check the explanation instead of
failing.

| Id | Severity | Steps | Expected | Actual | Suspected cause |
| --- | --- | --- | --- | --- | --- |
| V1 | High | F3 on `docs/page.html` or `docs/plain.htm` | The page with its local stylesheet (blue heading) and local image | Blank page; the status bar says "Content resource teardown is unconfirmed"; the mode picker is empty (`shots/viewers_files/docs-page-html.png`) | `crates/midnight-viewer-engine/src/content_resources.rs:66`: the local-resource `Scope` (and its `Asset` source) do not implement `release_on_worker`, so the trait defaults (`crates/midnight-plugin-host/src/content.rs:23` and `:49`) report an error that `crates/midnight-web-viewer-host/src/resource_jobs.rs:29` turns into the viewer's failure. Production attaches the same `Scope` to every local capture (`native_content::factory_with_resources`). Forwarded to the viewer-performance work. |
| V2 | High (performance) | F3 on small and large files in sequence | Every open near the usual 0.2 s | Occasional stalls of 0.6–6.7 s on files of any size (see "Open latency"); 15 back-to-back opens of the same small file did not stall | Not profiled. Window creation is 0–1 ms, so the stall is in the backend's first frame; it may share V11's cause (disk flushes under heavy write load). |
| V3 | Medium (performance) | F3 on any small file | Well under 100 ms in a native app | A steady 192 ms floor with the text, source, Markdown and hex viewers (240 ms images, 337 ms PDF), identical across plugins and file sizes | A fixed delay in the backend's first-frame path (not located; window creation is 0–1 ms). |
| V4 | Low (UX) | F3 on `images/truncated.png` or `images/zero.png` | A clear, user-facing error | "Image decoder rejected the source [image cleanup: Confirmed]": clear, but with internal cleanup jargon | The image viewer's status text appends its cleanup state. |
| V5 | Low (UX) | Enter on `archives/not-a-zip.zip` | A clear error | "archive: invalid Zip archive: Could not find EOCD (<full path>)" in the panel's error bar; clear but technical (`shots/viewers_archives/not-a-zip.png`) | The archive provider's message is shown as is. |
| V6 | Gap (documented) | Insert on `binary-a.bin` and `binary-b.bin`, Ctrl+Shift+C | A verdict, or a clear refusal | "Text comparison requires UTF-8 or UTF-16 with a BOM; this file is binary or uses another encoding." | The comparator compares text and images only. |
| V7 | Gap | F3 on `code/style.css`, `code/Makefile` | Syntax colours | Plain text | The source viewer's 21 lexical profiles have no CSS or Makefile profile (by its manifest). |
| V8 | Gap | Alt+5 in `docs/` and `images/` | Previews where supported | PDFs and the truncated and zero-byte PNGs show placeholders; PNG, JPEG, GIF, WebP and SVG show thumbnails | Only image and video thumbnailers are installed; no PDF thumbnailer. |
| V9 | Gap (documented) | Enter on `inner.zip` inside `archives/nested.zip` | Its listing, or a clear explanation | Notice "Archive member: Use F3 to view this member or F5 to copy it to a native folder. External launch inside an archive is unavailable." | Nested archives are unavailable (README). |
| V10 | Low (UI) | Alt+5 in `thumbs/`, End | Partly visible rows drawn like in the list views | The grid draws whole rows only: a blank band where the partly visible top row would be (`shots/viewers_thumbnails/thumbnail-grid-end.png`); the request range still loads that row | The grid's paged layout in `crates/midnight-ui/ui/file-pane.slint` (by design or not: for the UI owner to decide). |
| V11 | High (intermittent stall) | Enter several archives in turn, then `bundle.zip` → `bundle` → F3 on `readme.txt` | The member in about 0.2–1.7 s | Run `20261120-024144`: after 23.7 s, "archive: Archive worker made no progress for 20 seconds and was stopped; open the archive again" (`shots/viewers_archives/view-member.png`); afterwards Backspace did not take the panel out of the archive | Hypothesis from the viewer-performance session: `File::sync_data` when the worker's spool file is finished (`crates/midnight-worker-platform/src/spool.rs`) stalls for seconds while the disk is busy, so the 20 s watchdog (`crates/midnight-filesystem-host/src/session.rs:114`) stops the worker; that sync is removed in its pending changes. The panel staying in the stopped archive is a separate candidate, routed to a presenter fix; five more runs did not reproduce the stall, and the scenario now records the panel state around each Backspace when it does. |
| V12 | Gap (undocumented) | Insert on `large-a.log` and `over-limit.log` (4.25 MiB), Ctrl+Shift+C | A comparison, or a clear refusal the documentation mentions | "Text comparison supports at most 4 MiB per file." The README's comparison section does not state the limit | `crates/midnight-compare-engine/src/text.rs:14` (`MAX_BYTES`). Found when the scale-dependent 8 MiB pair of the first full-scale run was refused; the fixture now has a 3.5 MiB pair and one over-limit file at every scale. |
