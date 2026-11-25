# Parallel acceptance runs, 25 November 2026

Measurements use the full fixture scale on one Windows workstation and its Ubuntu
24.04 WSL2 guest. Linux and Windows ran different four-scenario samples. Times
are per run and include fixture generation. They exclude compilation and Windows
package installation.

The complete Linux suite went from **61 min 54 s** (117 scenarios, serial) to
**20 min 14 s** (118 scenarios, four workers). The latter ran alongside Windows
on the same workstation. It exited successfully with 115 passes, one warning
and two skips. The 117 shared cases retained their previous statuses and all
1,790 screenshot names. The new folder-dialog case added four images. The
warning flagged HTML first-content latency of 997 and 840 ms against the
500 ms review threshold. The browser functionality checks passed.

The complete Windows run took **27 min 58 s** with four workers and the
installed 0.5.3 packages. All 118 scenarios were accounted for, with 114
passes, one warning and three skips. The previous day's serial run took
**60 min 19 s** on an earlier revision. The table below gives a controlled
four-scenario comparison. [Previous Windows measurement](2026-11-24-acceptance-time.md)

Windows run `20261125-125547` reached four simultaneous scenarios. The last
of the 112 parallel-safe scenarios finished at 1,543.098 s and the first of
six exclusive scenarios started at 1,543.100 s. The report linked all 1,775
expected screenshots, and each file had a valid PNG signature. HTML and PDF
first content took 2,532 and 1,823 ms, triggering the 500 ms review threshold.
Both passed the 10 s functional deadline. The skipped scenarios were the
account Recycle Bin, a Linux symbolic-link and permission journey, and the
opt-in full theme matrix. Some archive subchecks also lacked external tools.

| Four-scenario sample | Serial | Four workers | Elapsed time saved | Result in both runs |
| --- | ---: | ---: | ---: | --- |
| Windows Options | 30.5 s | 18.9 s | 38% | 4 passed, 13 screenshots |
| Linux menu, Options and shortcuts | 66.6 s | 29.0 s | 57% | 4 passed, 7 screenshots |

The Windows serial measurement used the original runner. The Linux pair used
the same executable with different worker limits. To repeat the samples, run
each command once with `--jobs 1` and once with `--jobs 4`.

```text
pwsh -File scripts/acceptance.ps1 --only options_general,options_keyboard,options_colors,options_comparators --jobs 4
bash scripts/acceptance.sh --only tools_main_menu,options_editors,options_comparators,shortcuts_footer --jobs 4
```

## Worker isolation

On Windows the two-worker run of `files_clipboard_drag`, `about_box` and
`panels_properties_several` passed with seven screenshots. The synthetic drag
and About scenarios started at 2.081 and 2.084 seconds. The real Shell property
sheet started at 30.165 seconds, after the longer worker finished at 30.164.

A separate two-worker Windows run exercised both phases of `options_previews_plugins`
and the full `options_palette` check. Both passed. The aggregate report retained
six screenshots from Previews and Plugins and four from the palette and migration.

`scripts/tests/test_acceptance_schedule.py` takes a prebuilt executable via
`MIDNIGHT_ACCEPTANCE_TEST_BINARY`. Its three checks passed on Windows. They
exercise the platform classifications, argument validation and precedence,
worker limit, exclusive barrier, report order and PNG collection.

## Slow scenarios

The following three cases passed in both the original serial suite and the
four-worker Linux run measured without the concurrent Windows suite. That
four-worker run took 13 min 16 s and exposed a separate Find fixture failure.
Its time is excluded from the successful full-suite comparison above.

| Linux scenario | Previous serial | Four workers | Elapsed time saved | Matching screenshot names |
| --- | ---: | ---: | ---: | ---: |
| `ui_polish_dialogs` | 469.2 s | 422.6 s | 10% | 35 |
| `viewers_scroll` | 475.5 s | 436.2 s | 8% | 31 |
| `ui_review` | 225.4 s | 141.5 s | 37% | 489 |

The Windows serial `ui_review` run took 118.0 s after the change, down from
204.5 s, and retained all 488 screenshot names. Captures now check for active
animations after rendering the window before waiting for a transition.

The Windows serial `viewers_scroll` run took 473.5 s, down from 492.0 s. Both
runs passed, captured the same 31 screenshots and reported the same two
unsupported input checks. The probe now starts the quiet interval at the first
ready sample. A change of position or rendered content restarts that interval.

The corrected Windows dialog sweep passed with 35 screenshots in 490.6 s.
The pre-change Windows sweep stopped at Options → Comparators after its 24th
screenshot. Adding a rule could remove the focused toolbar button and leave
Escape inoperative until Tab restored focus. The comparator acceptance scenario
now checks Add followed directly by Escape for both one and two rules.

The Linux dialog sweep exercised the same 172 actions and retained the same
35 screenshot names as the previous full run. Reusing each surface's first
opening reduced scratch restores from 312 to 278 and their time from 94.7 to
78.7 s. The 34 surface captures took 2.2 s, down from 11.2 s. The scrolling
scenario ran all 64 subchecks, including HTML, and reported the same two
unsupported input checks.

The Find count-limit fixture now lives in its scenario's outer work folder.
The path which had caused the earlier failure reached all 100,000 rows and
reported the result-count limit. A deliberately deep path reached 84,095 rows
and correctly reported the retained-memory limit. Both produced all seven
expected images.

On Windows the duplicate-candidate fixture had been extending 100,001 files of
distinct lengths without setting their NTFS sparse flag. Their logical sizes
sum to 5,000,150,001 bytes. The fixture now marks each open file sparse before
extending it. A 254-file allocation probe verified the lengths, zero contents
and sparse attribute, with 399 allocated data bytes for 13,704,352 logical bytes.
