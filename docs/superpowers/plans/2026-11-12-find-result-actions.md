# Find result actions

This implementation follows the approved recursive Find design and its explicit
execution-boundary requirement. The modeless result session remains independent
of panel navigation. Salamander `src/finddlg1.cpp:2163-2165` and `:3633-3673`
provide the behavioral reference: separate Focus/Open/View/AltView/Edit commands
and regular-file-only View/Edit. No reference source, fixed path buffers or ANSI
handling is copied.

1. Add a separately captured Find file capability using retained IDs. Keep the
   ordinary navigation receipt read-only. Revalidate identity/revision and the
   exact current session generation on backend workers.
2. Bound retained captures to sixteen and share the existing startup retirement
   domain. Carry provider, result and cleanup ownership through rejection,
   cancellation, stale delivery and shutdown without native final drop on the UI.
3. Route View through the installed native capture provider and existing viewer
   manager; validate around capture and compare stable native identity/revision.
   Route explicit Open/Edit through bounded desktop launch/process services.
4. Add result controls and use current configured View/Plain text/Edit shortcuts.
   Keep Enter/double-click as Go to file and folder actions as navigation.
5. Verify native changes, close/restart/Stop, bounded captures, stale UI replies,
   editor preference snapshots, native worker ordering, cancellation/retirement,
   configured keys and existing modeless result interactions.

Ownership: Find modules/tests/UI and desktop Find adapter are isolated from the
central core protocol, engine lifetime/dispatch and viewer integration owned by
the coordinating task. Tests and generated artifacts remain under `_build`.
