# Previous captured file in viewers — 12 November 2026

The requested behavior is the reverse of Space: Backspace opens the previous
regular file in the viewer's captured panel order, wrapping from the first file
to the last. Alt+Left is an alias; Space and Alt+Right advance. All aliases belong
to the configurable shortcut catalogue and must remain absent from search/edit
contexts. Comparison's existing Alt-arrow difference actions must remain intact.

Implementation reuses one adjacent-file direction helper and the existing reopen
lifecycle: no UI path is recaptured, no extra capture worker is introduced, and a
pending identity or decoder close retains the existing bounded worker ownership.
Both directions clear explicit mode choice, selection, scrolling and retained
camera state before presenting the replacement file. A rejected enqueue keeps the
current UI state.

Ownership: tree_metadata owns the core input variant, viewer action catalogue,
engine/UI navigation, examples and focused regression tests. Root owns separate
comparison shortcut contexts and preservation of saved shortcut assignments.
find_actions owns browser-first editing arbitration and the additive public HTML
capability/event compatibility work. Shared catalogue edits are limited to each
owner's entries.

Validation uses the existing shortcut resolver, worker input boundary, image/PDF
loading and ready states, retained text/image camera tests and actual search widget.
The opt-in real-plugin integration also cycles backwards across the first/last
boundary and returns through Text, Source and Markdown. Root owns coordinated Cargo
runs and package/native qualification; no unexecuted test is counted as passing.
