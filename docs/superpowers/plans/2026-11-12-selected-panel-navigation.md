# Selected-item panel navigation

User request: Alt+Up/Down moves to the previous/next selected panel item, matching Salamander, while comparator Alt+Up/Down retains difference navigation.

Reference: `D:/experiments/salamander-main/src/fileswn0.cpp`, `SelectFindNext` (lines 122–157) and Alt+Up/Down dispatch (1736–1789). Files and directories participate; skip the current row, scan displayed linear order in every layout, and never wrap. Quick-search owns its input.

Implementation:
- Stable `panel.previous-selected` / `panel.next-selected` actions in FilesIdle; preserve saved explicit bindings and existing parent/history/comparator bindings.
- Route through existing panel key callback and cursor reveal. No selection mutations, provider calls, or new backend protocol.
- Exact native/tree selection bitsets skip empty words; archive IDs map to visible positions. Pending previews and differing filter/order bases use current displayed marks. Parent/context/hidden rows cannot be targets.
- Focused core default/remap/context tests; presenter native/tree/archive/filter/pending/sparse tests; installed key bridge integration in Detailed/Brief/Thumbnails and both panes.

No new dependency. Root owns Cargo execution and final integration. No commit or deployment is part of this task.
