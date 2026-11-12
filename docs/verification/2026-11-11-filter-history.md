# Filter history and dialog actions

The native Panel filter dialog offers recent successful filters with their
matching mode and case sensitivity. Choosing history fills the editable draft;
it does not apply, validate or launch anything. Clear erases draft text and stays
available whenever that text is nonempty. Show all remains the distinct action
for removing an applied panel filter. OK still performs backend validation.

History contains at most 100 nonempty enabled entries, newest first, deduplicated
by mode, exact expression and case sensitivity. Flat filters enter history after
successful validation and captured-panel checks. Tree filters enter history only
after the correlated successful projection publication. Failed, stale, canceled,
cleared and merely typed expressions never enter history. A late initial settings
load merges already used local filters ahead of saved history. Options defaults
and cancellation leave this user history intact.

Help is a child of the full-width DialogFooter, alongside Show all and OK, so it
no longer consumes width outside the footer and accidentally forces Cancel onto
a second row. Narrow dialogs retain the shared footer scrolling/stacking behavior.

Coverage: presenter filter/history tests, tree application receipt tests, settings
codec tests and native_filter_hotpath's form tests exercise history selection,
separate draft clearing, preserved mode/case, and normal-width footer alignment.
Integration build/test evidence is recorded by the parent task.
