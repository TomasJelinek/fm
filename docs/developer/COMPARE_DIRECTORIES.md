# Compare directories

Ctrl+F10 (or **Compare directories** in the shared toolbar) compares the two panels'
displayed native folders and replaces both panels' selections with the differences.
The dialog closes at Compare; the comparison keeps running on a backend worker, the
footer status line shows its progress and then one result line ("Compared: …"), and
no result window appears. Opening the dialog again while it runs shows the progress
and Cancel (Escape); afterwards it shows the last summary. The design background is
the [commander tools spec](../superpowers/specs/2026-11-06-commander-tools-design.md);
expanded inline trees are covered in [TREE_PANEL.md](TREE_PANEL.md#directory-comparison-with-expanded-branches).

## Criteria

| Option | Default | Effect |
| --- | --- | --- |
| Compare file size | on | The larger file of a same-named pair is selected. |
| Compare modification time | on | The newer file is selected; whole seconds, with the tolerance (0–86400 s, default 2; a difference equal to it counts as equal). |
| Compare contents | off | Same-named files of equal known size that the enabled criteria found equal are read byte by byte; different bytes select both files. A known size difference selects both files when size is not a criterion. |
| Match names with exact case | on | Off: Unicode case folding. Ambiguous folded groups are all selected. |
| Include subfolders | off | Same-named folders are compared recursively (below). |
| Select unmatched directories | off | Also select folders without a name match. Always on, and disabled, with Include subfolders. |

Names are matched exactly (or folded); files present on one side only are selected
on their side. Unknown requested metadata, and an entry that is not a regular file
opposite a file, select both sides as unknown; nothing unknown is ever reported as
equal. The displayed rows are compared, so persistent panel filters and hidden-file
settings apply to the top level; inside compared folders every entry counts.

## Compare contents

The size and time pre-check runs first: pairs that size or time already separate,
and pairs of different known length, are never read. The remaining pairs are queued
during the folder walk (at most 100,000, charged to a 64 MiB memory budget) and then
read in a second phase with known totals, so the status line can say "Comparing
contents: 3 of 10 files · 1.2 MiB of 40 MiB…". Each file is opened through the
native search service (`FindService`), which pins it without following links. A
displayed file must still have its displayed size; a file found in a subfolder must
still be the identity and revision the walk captured. The two files are read in
128 KiB chunks into two reused buffers, compared as they arrive, and revalidated
after the read; a file that changed, grew, shrank or could not be read is selected
on both sides as not verified and counted as an error with a diagnostic. Nothing is
written.

## Include subfolders

Each same-named folder pair of the displayed rows is walked depth first through the
native search service: both folders are listed (names, kinds, sizes, times; links,
special files and mount boundaries are reported by name and never followed), the
children are grouped by name exactly as at the top level, and same-named subfolder
pairs are walked in turn. A difference anywhere inside (a file or folder on one side
only, a larger or newer file, different contents, an ambiguous name group, a link
opposite a file, an unreadable entry) marks the displayed folder:

- the displayed folder is selected on each side the difference selects: the side
  with a one-sided entry or the larger or newer file, both sides for unknown,
  ambiguous, unreadable or different-content cases;
- in an expanded inline tree (which lists folders), every displayed subfolder on the
  way to a difference is selected too, so expanding a selected folder shows where
  the differences are. A flat panel shows the selected top-level folders; entering
  one and comparing again narrows it down.

Links present on both sides are matched by name only and counted as "matched by
name only"; folder links at the top level are not followed either. Limits: 1,000,000
entries examined, 128 folder levels below a displayed folder, 64 MiB of listings,
pending folder pairs and content candidates. A reached limit ends the walk; the
folders it did not finish are selected on both sides and the result says that a
limit was reached. An unreadable folder or entry selects its folder pair on both
sides and is counted with a diagnostic (at most 8 kept).

## Result

The footer line names the counts the panels now select, then, when they apply, the
folders with differences, the pairs with different contents, the file pairs read,
unknown metadata, ambiguous name groups, unreadable entries and a reached limit. It
is shown while both panels still show the selections it made. The reopened dialog's
summary repeats the counts with the diagnostics' paths and reasons.

## Threads, cancellation and authority

The UI only captures the displayed rows (identities, projection and selection
revisions) and the options. The engine's comparison controller runs one coalescing
request at a time in `spawn_blocking`, anchored to the engine's service lifetime.
A newer request, Cancel, a panel change or engine shutdown cancels the request's
token at once: native listing and content reads stop at their next check, and a
cancelled request publishes nothing. Progress samples (`Event::ComparisonProgress`,
at most four per second, dropped when the UI queue is full) are shown only for the
running request. Selections are published only if both panels still show the
captured rows; panel paths never authorize native access, and the search service
opens every walked folder and read file itself. Without the search service, Compare
contents and Include subfolders fail with an explanation; metadata-only comparison
needs no service.

The panels show changes only once a comparison ends, and Compare contents or Include
subfolders can read for minutes. While such a comparison runs, F2 Rename, F7, Shift+F4,
Delete, the Copy/Move dialog, Attributes, Paste and Show hidden files are refused with
a "Directory comparison running" notice (`Presenter::comparison_refuses`): wait for it
to finish, or open Compare directories and cancel it. Drops, custom commands and
external changes still wait for the comparison, as do the actions of a list of search
results.

## Verification

Engine: `crates/midnight-engine/src/comparison/deep_tests.rs` (an in-memory search
service: contents, pre-checks, subfolders, links, unreadable entries, a file written
during the read, cancellation during a read, limits, expanded-tree selections, the
controller's progress events and cancellation of a blocked read). Presenter:
`presenter/comparison_tests.rs`. End to end: the `tools_compare_dirs` acceptance
scenario ([ACCEPTANCE.md](ACCEPTANCE.md)) compares identical and asymmetric trees
with Compare contents and Include subfolders.
