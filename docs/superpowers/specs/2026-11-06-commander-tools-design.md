# Commander comparison, filtering, modifier toolbar and metadata editor

The user requests Alt+Up/Down command history, Ctrl+F10 directory comparison,
Ctrl-dependent function buttons, Ctrl+F11 panel zoom, Ctrl+F12 panel filtering,
and Salamander-style filename case conversion merged into an attributes/time editor.
All application actions remain configurable. All scanning, matching, preparation,
comparison, metadata updates and renames run on backend workers.

## Salamander references and intentional behavior

The local reference is `D:/experiments/salamander-main/src/`: `salamand.rc:63`,
`toolbar4.cpp:1251`, `mainwnd5.cpp:1139`, `dialogs6.cpp:1452`, `dialogs3.cpp:24`,
`fileswn5.cpp:185` and `fileswn6.cpp:2283`. Its default Ctrl row is:

| Key | Action |
| --- | --- |
| Ctrl+F1 | Drive information |
| Ctrl+F2 | Attributes, times and case editor |
| Ctrl+F3–F6 | Sort by name, extension, time, size |
| Ctrl+F7 | Same editor, case section enabled |
| Ctrl+F8 | Unassigned unless configured |
| Ctrl+F9 | Refresh |
| Ctrl+F10 | Compare directories |
| Ctrl+F11 | Toggle active-panel zoom |
| Ctrl+F12 | Panel filter |

## Comparison

Implement comparison of the two displayed directories by file size and modified
time, both enabled by default. Match exact native names; provide explicit
case-insensitive matching without using locale sort keys as identities. Ambiguous
case-insensitive groups must not be silently paired. Time tolerance defaults to
two seconds, matching Salamander; dates are compared at whole-second precision.
Unmatched files are selected on their own side; same-name pairs select the larger
and/or newer side. Optional unmatched-directory selection does not recurse.
Unknown metadata is reported, never treated as equality. Describe equality as
matching metadata, not verified identical content. Recursive/content comparison
remains an explicit later extension, not an advertised working option.

The comparison uses persistent-filter-visible rows and clears transient prefix
search before capture. A coalescing worker uses immutable captured source/index
identities and selection revisions. Publish both resulting selections together
only if both panes still match the capture. Cancellation, errors, navigation,
filter changes and later selection edits preserve current selections. Update the
selection worker's authoritative seed revision so the next toggle extends the
comparison result instead of resurrecting the earlier selection.

## Panel filters

Keep quick prefix search separate from a saved per-panel filter. A dialog switches
between wildcard masks and Rust regular expressions; supports enable/disable,
Apply and Cancel, and shows validation errors without changing the active filter.
Wildcard masks follow Salamander: `*`, `?`, semicolon alternatives, `|` exclusions,
`;;` for a literal semicolon, and `*.*` matching extensionless files too.
Directories remain visible. Keep case behavior explicit in the dialog. Names
that cannot be decoded as Unicode remain visible and retain their native identity.

Combine persistent filtering and prefix search in one backend projection so the
two cannot publish independently stale row models. Preserve all index vectors,
including sizes, and exclude hidden entries from bulk selection and operations.
Bound expression size and compiled-regex resources; cancellation and invalid
patterns cannot silently replace a previous valid projection. Save independent
panel filters through the existing versioned settings service with migration.

## Command history, Ctrl toolbar and zoom

Alt+Up walks toward older stored commands and Alt+Down toward newer commands,
wrapping through the stored history. History selection only fills the editor and
preserves the selected terminal flavor. Enter executes through the existing
submission path, retaining the 100-entry deduplicated persisted history. No
history entries means no action. Editing/replacing the draft resets traversal.

While Ctrl is held, derive all twelve function-button cells from effective
configurable Ctrl+F1–F12 bindings. Labels, hints, enabled states and click actions
must agree. Ctrl+F8 is blank unless assigned. Clear modifier state on release and
focus loss; AltGr must not masquerade as Ctrl. Capture the displayed action and
pane on mouse press so a modifier release cannot retarget a pending click.

Zoom retains both panel models and the previous split ratio, maximizes the active
panel and leaves an inactive-panel activation rail. Switching panels transfers
zoom. Toggling again restores the previous ratio; centering the splitter exits
zoom. Preserve these semantics after swapping panels and resizing the window.

## Attributes, timestamps and case

Use one editor with Attributes, Times and Case sections and a bounded backend
preview. Ctrl+F2 starts with every edit set to Leave unchanged and case disabled;
Ctrl+F7 opens the Case section with conversion enabled. Mixed current values and
requested changes are different states. An empty patch cannot be applied.

Provide lowercase, uppercase, mixed/title case, and mixed basename with lowercase
extension; scopes are the entire name, basename or extension. Default conversion
is lowercase/whole name and recursion off. Use full Unicode case mappings and
document word boundaries. Preserve native bytes/code units outside the transformed
component; if that component is invalid Unicode, report it as unsupported instead
of replacing it lossily. Dotfiles have no extension and compound extensions use
the final suffix. Expanded names still pass native rename validation.

Windows supports Read-only, Hidden, System, Archive, Created/Modified/Accessed,
and handle-based NTFS compression where available. Show encryption state but do
not expose unsafe pathname-based EFS mutation. Linux exposes native permissions
and Modified/Accessed; creation-time mutation and Windows attributes are unavailable.
Keep special permission bits unless explicitly edited. Time entry names its zone;
reject invalid or ambiguous local times unless an explicit offset resolves them.
Set Current captures one backend instant for the entire patch.

Prepare immutable selected roots and native identities; a preview ID/revision
must match Apply. Reuse the bounded mutation queue, progress/cancel channel and
mutation gate. Keep native write handles through rename and metadata changes.
Do not substitute a pathname write after validating a descriptor. On Linux, use
descriptor-relative metadata APIs with identity-checked usable-handle fallbacks
where available; return unsupported/access errors when no safe fallback exists.

Recursive processing never follows symlinks/reparse points or crosses mounts.
Snapshot each directory's names before renaming them, process children first,
and apply directory permissions/times last. Known and late name collisions offer
Skip/Skip all/Cancel, never replacement or invented alternate names. Case-only
renames must retain the same object identity; a different hard-link directory
entry is still a conflict. Explain metadata effects on other hard links in preview.
Finish or restore an in-progress staging rename before acknowledging cancellation.
Report partial per-item changes accurately; do not promise transactional rollback.
Bound handles, enumeration buffers, errors and preview rows with visible resource
errors rather than silently truncated selections. The initial implementation
shares 256 pinned targets across all prepared metadata jobs and recursive child
snapshots, limits depth to 64 and a directory name snapshot to 4 MiB, and displays
at most 32 preview/result items. These resource budgets do not impose a path-length
limit. Two preparation/preview workers share one budget; native mutation uses the
existing serialized gate. A collision wait remains cancellable under event-queue
backpressure. Completed partial results are retained in a bounded job-owned cell
and published after the native worker releases its gate, even when the user cancels.

The editor has Attributes, Times, Case and Preview tabs. Editing invalidates the
previous preview immediately; changing a draft while a preview is running cannot
make its stale result applicable. Apply queues only the reviewed revision. Jobs
exposes pending collision decisions again after the user dismisses their dialog.
Bounded failure details are retained separately from the first 32 outcomes, so a
late resource or permission error is still visible. Recursive outcome paths are
labelled as the paths at the time of each edit; a later parent rename can change them.

## Validation

Cover stale two-pane results, compare then toggle, unknown metadata, time tolerance,
ambiguous names, filter syntax/cancellation and preservation of file-size vectors.
Exercise real history keys, Ctrl transitions/AltGr/focus loss, remapped footer
actions, mouse modifier release and swapped/zoomed panels. For writes, test Unicode
expansions and contextual casing, raw invalid names, long paths, collisions,
case-only changes, replaced parents, links, mounts, Keep semantics, timestamp
precision, postorder directory updates, cancellation and resource exhaustion.
Render dialogs at both densities and large fonts; run Windows/Linux checks and
critical native mutation tests in release mode before final commit and push.
