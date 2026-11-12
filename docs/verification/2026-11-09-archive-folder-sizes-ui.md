# Archive folder size presentation

Windows source and software UI verification, 9 November 2026.

Ctrl+Shift+S and Space now send typed archive size requests with the exact
committed listing and opaque IDs. A files-only selection remains a files-only
request; it never becomes an all-folder scan. Rows read a backend snapshot by
mount/member identity, and selected-folder status uses a backend summary scoped
to the exact selection. No presenter code lists, validates, reads or extracts
archive members. The fixture mount panics if such I/O reaches the UI tests.

The archive size capability is separate from native mutation support. Its toolbar
button and F10 action can be enabled while archive delete, move, rename and native
properties remain disabled. Updates share the application's 250 ms size pulse;
size events never trigger automatic sorting. Resorting the same mount preserves
the running scan, cached totals and caret.

The independent review led to these fixes:

- Observer request IDs now live for the presenter pane lifetime, so switching
  mounts cannot reuse IDs rejected by the backend's request high-water mark.
- Invalidated mounts reject delayed cached totals and snapshots. The regression
  initially restored an old selection's bytes; it now remains empty.
- Flat-list selected totals are hidden for inline tree selections, including
  delayed replies. Tree publications have their own selection authority.
- A running scan retains its original parent directory. Navigating elsewhere in
  the same archive cannot mark unrelated folders as queued.
- Every snapshot is acknowledged by pane/mount/version, including stale mounts
  and replies received after leaving the archive. Queue saturation retains the
  acknowledgement for the 100 ms retry; large retired maps leave the UI thread.

Final command:

```text
cargo test -p midnight-ui --lib --test shared_toolbar --test main_menu --offline --locked -j2
```

Result: **595 library tests, 5 main-menu tests and 11 shared-toolbar tests passed**,
with no failures or ignored tests. Fourteen library tests cover the new archive
size presentation. The new real-toolbar test triggers the active archive panel's
size callback and checks that delete/rename remain disabled in F10. Final log:
`_build/delete-followups/archive-size-reviewed-ui.log`. Earlier RED evidence is in
`archive-size-ui-red.log` and `archive-size-invalidation-red.log` beside it.

The actual application build and installed archive hotkey workflow still require
qualification. Manual Size sorting does not yet consume measured archive folder
totals, and expanded-tree selections are not admitted by the flat-list scan.
These are outstanding integrations, not covered by the passing presenter tests.
