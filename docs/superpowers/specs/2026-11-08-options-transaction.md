# Options: OK, Cancel and reversible previews

Options is one edit session across all pages. Appearance, font, density and other
live previews stay in memory until OK. Page-level Apply/Assign/Add operations
update this session only. OK validates retained editor, filename-color and Custom
Command drafts even if another page is selected, then submits one backend settings
save and closes only after its successful acknowledgement. Validation, queue or
storage errors keep the dialog and draft open. The offending page is selected.

Cancel, Escape and X discard every settings change since opening, including
previously applied page drafts and appearance previews. Nested color/key capture
first consumes Escape itself. Cancel invalidates outstanding validation/picker
results so they cannot restore discarded edits. Reopening starts from accepted
settings. While the final disk write is in flight, editing/closing is temporarily
disabled; failure enables retry or cancellation. No blocking I/O runs in the UI.
This waiting state tracks writes actually admitted by the backend queue, separately
from the latest local edit. A rejected independent session save cannot leave
Options locked after its earlier accepted write fails; admitted successors still
keep the transaction frozen until an acknowledgement resolves it.

Session data such as window geometry, tabs, bookmarks, terminal flavor and viewer
state continue to persist independently. Their saves must use the accepted options
values, never a preview. Commands completing during Options keep their history
entries; cancelling a staged history clear restores the original history together
with new executions. Shutdown also excludes unaccepted previews. Before the
initial preferences load finishes, option editing is disabled; read errors never
authorize overwriting an unreadable settings file.

Reset to defaults stages defaults for every Options page, including appearance,
fonts, shortcuts, editor, filename coloring, previews (including thumbnail size)
and archive associations.
It preserves tabs, command history, hot paths and custom commands, including an
unfinished custom-command draft. Session window geometry, terminal choice,
panel view/filter state and viewer preferences remain independent. Reset discards
unfinished editor/color drafts and invalidates their late validation responses.
It does not write preferences: OK saves the staged defaults; Cancel restores the
original settings, including a previewed theme. While initial loading, native
picker selection, validation for OK or the final save is in progress, Reset
cannot race those operations. Its fixed footer button remains reachable in the
minimum window size, where an icon and tooltip replace the longer label.

Export configuration and Import configuration sit in the same footer and belong to
the session (added 20 November 2026; user guide:
[Export and import configuration](../../SETTINGS_FILES.md)). They are optional
footer actions: with labels where everything fits, with icons only where only
that fits, and otherwise at the end of the General page. OK must never scroll out
of the footer, because Slint 1.18 drops the focus of a control clipped out of view
on the next key, so Escape or Return would reach no handler; a focused Export or
Import hands the focus to OK before it moves. Export writes what OK
would save at that moment, in the settings-file serialization, through a backend
worker; it is not a settings edit and leaves the session open. Import is one
staged edit of the whole settings snapshot: the backend reads and validates the
complete file first, and a refused file changes nothing. An accepted file previews
every Options-owned field like Reset does, discarding unfinished page drafts, and
stages the remaining session fields (tabs, hot paths, window placement, panel
split/view/filter state, viewer, terminal, checksum and Make File List choices).
OK's save includes them; only its successful acknowledgement applies them to the
running session, restoring tabs through the startup restoration path. Cancel
discards both parts, and Reset discards the staged session part. `restore_fields`
and `restore_session_fields` classify every preference exhaustively. OK and Reset
wait while an import is being read; a result from a closed session never stages.

Explicit maintenance actions such as clearing thumbnail cache are immediate
actions, not settings edits; label their behavior accordingly. Custom Command
preview must leave the Options transaction open rather than implicitly accepting
or cancelling it.

Validation covers all close routes, theme/font rollback, cross-page drafts,
asynchronous validation/save failures, stale callbacks, unrelated session saves,
late initial load, and one successful save per accepted edit session.

The foreground/background color swatch opens a separate modal Slint popup above
Options. The existing HSV/RGB controls and presets occupy a scrollable body;
Use color and Cancel remain fixed at the bottom. The popup can be moved and resized,
fits the current viewport (including 320×200), and scrolls instead of enlarging
the settings form. X, Escape and Cancel discard only the chooser's draft and
return to Options. Use color stages the chosen value in the color-rule editor;
the outer Options OK/Cancel still controls persistence. Reopening resets the
chooser to that field's current value. Changing the rule/page or closing Options
invalidates the chooser. This follows Salamander's separate `ChooseColor` dialog
in `src/dialogs4.cpp`, while retaining the shared multiplatform Slint UI and
avoiding another dependency.
