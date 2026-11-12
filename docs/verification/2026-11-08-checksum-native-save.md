# Checksum native Save dialog — 8 November 2026

The Results-page **Save manifest…** action (compact label **Export…**) opens the
existing system Save-file picker directly, with the currently selected checksum
algorithm and its native extension filter. Cancelling leaves the current page and
does not enqueue a manifest write. An accepted choice opens the existing export
confirmation, displaying the destination as read-only text. **Choose file…** opens
the picker again; there is no destination line editor.

The confirmation retains the included/excluded counts and required partial-results
acknowledgement. Its explicit **Save** uses the existing manifest service, including
exclusive staging, independent SHA-256 readback and refusal to replace an existing
file. This follows SECURITY.md's rule that a native picker choice fills a draft.
The raw picked `PathBuf` remains the operation identity; display text is not used
to reconstruct it. No dependencies or native platform implementations changed.

The component regression exercises the real **Save manifest…** action, selected
CRC32/SFV algorithm, busy-picker gating and read-only confirmation. The lifecycle
regression drives the installed presenter/window callbacks through picker cancel,
accepted Unicode destination, explicit confirmation and the resulting exact-path
save command. Existing partial-consent, algorithm-choice and minimum-window layout
coverage remains applicable.

The targeted regression failed against the original behavior: clicking
**Save manifest…** produced no picker callback instead of the expected Save/SFV
choice (`checksum-native-save-red.log`). The coordinated Windows and Linux runs
passed all 6 checksum-polish and all 5 checksum-window tests, including that
regression and the native-choice/cancel lifecycle. Logs and render artifacts belong under
`_build/qa/metadata-review/`. An earlier build attempt stopped before test execution
on concurrent Slint edits; that compilation error is not regression evidence.
