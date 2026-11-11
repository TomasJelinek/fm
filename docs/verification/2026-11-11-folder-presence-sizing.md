# Folder presence and linked-directory sizing qualification

The 11 November implementation keeps flat-row subfolder hints separate from tree
adoption. Requests retain the exact completed root entries/index, navigation and
viewport IDs. Native probes scan through the injected directory service without
retaining irrelevant file tokens or enriching metadata. An empty-folder hint is
published only after successful enumeration; provider errors keep their details.
Actual expansion still validates child identities and its ordinary tree session.

Native size scans follow directory targets with stable filesystem identities.
Active ancestry detects self/ancestor cycles; independent noncyclic aliases count
their logical contents separately. Cycle state is proportional to active depth,
with no arbitrary directory/depth limit. The existing pending-path budget and
worker/I/O capacities remain. Selected native directory links also belong to the
UI's captured size scope, so their progress and final receipts are accepted.

The coordinated Windows Rust-only follow-up build passed all six focused groups:

| Scope | Passed |
| --- | ---: |
| Backend folder presence, including blocked latest-only worker | 5 |
| Native presence, including 100,001-file empty and folder-tail cases | 4 |
| Folder sizes, including native links/cycles and retained warning details | 36 |
| Drag/drop, including six new hover-preview cases and blocked cancellation | 23 |
| UI folder presence, including malformed/stale receipts and cache eviction | 6 |
| UI selected-folder sizes, including directory-link progress and final receipt | 6 |

Evidence is retained under `_build/qa/dialog-polish-20261111/` in
`rust-followup-build.log` and the corresponding `v3-engine-*` / `v3-ui-*` logs.
Earlier shared qualification also passed native-host sizing integration (3 tests)
and the read-only Rust probe resolving `C:\Documents and Settings` and enumerating
at most one target entry.

The final Make File List run passed all 14 presenter tests, including
`cancelling_file_list_conflict_blocks_confirmation_while_cancel_queue_is_full`
for both Replace and Append (`file-list-final.log`). Once Cancel begins closing,
the presenter refuses later confirmation and the Rust window manager hides that
action while preserving the draft until cancellation is admitted. No Slint binding
change was required. The native owned-dialog test also passed
(`native-owner-final.log`): the dialog has the expected HWND owner and no
`WS_EX_APPWINDOW`, restores its owner's enabled state, and leaves independent
working tools separate. Salamander's modal output-admission references are recorded
in [Make File List services](../developer/MAKE_FILE_LIST.md).

Three warning-detail tests initially failed because the shared
`_build/tmp/MidnightExplorer-size-warnings` directory belonged to
`NV-8NGHX14\CodexSandboxOffline`, while the test process ran under another
execution account. Its private owner/system DACL correctly prevented reuse;
counts survived and reports explicitly stated that details could not be stored.
Rerunning all four warning tests with a fresh run-owned `TEMP`/`TMP` directory
passed. No user ACLs or private-storage validation were changed. Qualification
must use a fresh owned temp directory when changing execution accounts.

Focused filters for the final patch:

- `midnight-engine --lib folder_presence`
- `midnight-engine --lib native::presence`
- `midnight-engine --lib drag_drop`
- `midnight-engine --lib folder_sizes`
- `midnight-ui --lib folder_presence`
- `midnight-ui --lib selected_folder_sizes`

Run these serially with the shared build owner. Linux selected-link classification
and its actual-listing regression are qualified separately by the Linux platform
and native-plugin checks.
