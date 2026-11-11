# Measured metadata columns

Size, Modified and Time now measure bounded display samples using the actual
panel font. The remaining width belongs to filenames. Ext sits immediately beside
Name in the shared filename header; metadata headers and row values retain their
existing alignment and sort identities. Narrow panes proportionally constrain
metadata while retaining usable filename space.

DirectoryIndex collects samples during its existing backend construction loop.
Numeric date/time formats share a hash and allocate only when a new format is
retained. At most 16 locale format representatives per field cover punctuation,
AM/PM conventions, and named months. The original displayed metadata remains
unchanged. Extremely unusual provider formats may still elide in a constrained
pane; no filename or operation identity is shortened.

Size sampling retains one maximum for each readable binary unit, since 1023.9 MiB
can be wider than 2.0 GiB. Native accepted size results update a small numeric
cache; tree/archive snapshots calculate the same bounded cache on backend workers.
The UI never scans folder-size maps or file-row models to determine widths.

Presentation expands each bounded numeric sample into ten digit variants; Slint
measures them with the current font, including proportional numeral glyphs.
Samples are cached by index generation, locale, size format and folder totals.
Scrolling and repainting reuse the sample strings. Tree payload budgets include
all retained sample allocations.

Coverage includes a 100,000-entry repeated-format sampling check, per-unit
regressions, localized long-month and AM/PM layout, size/header alignment, narrow
large-font direction icons, and Name/Ext header proximity. Integrated build/test
results are recorded by the parent task.

The fresh Windows core suite passed all 56 tests. In its unoptimized test profile,
100,000 repeated metadata observations took 79.0876 ms and retained less than
1 KiB of sample storage. The seven software-rendered row layout tests also passed
at multiple fonts and viewport widths. Evidence is under
`_build/qa/viewer-settings-20261111/final-core-lib.log` and
`early-editor-ui-panel_row_layout.log` in the same directory.
