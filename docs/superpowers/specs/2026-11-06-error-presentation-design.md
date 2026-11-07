# Readable error presentation

Errors must be recognizable at first glance and retain enough detail to act on them.
Use a shared error glyph and themed error card; never mark successful information or
unfinished size counting as an error. A textual label accompanies the icon for accessibility.

Present the failure reason first. Put the full path under its own `Path` label on a
separate line, followed by the operation. Preserve backend error text and Unicode;
do not parse a formatted OS error to recover a path. Empty paths omit that section.
This presentation does not alter native paths or the diagnostic `FsError::Display`.

The message dialog has a distinct heading, readable spacing, a scrollable multiline
body, Copy details and Close. Escape closes it. Copy details uses the existing backend
clipboard command and includes the complete message. Resize the dialog within the
application bounds; never hide actions below a long message. Inline errors in panels,
filter/comparison/settings and metadata editors use the same bounded, scrollable card.
Failed jobs show the error glyph and separately formatted error samples; ordinary
progress and name-conflict decisions retain their own status.

Reusing two small Slint components avoids duplicating layout across dialogs. Keeping
the existing single-line diagnostic formatter is preferable to changing logging and
all backend consumers. Tests cover actual failed actions, long Unicode paths, error
versus information state, copy dispatch, Escape and rendered dialogs at multiple sizes.
