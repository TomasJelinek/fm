# Editor associations and literal argument editing

Options > Editors follows Salamander's ordered mask-to-program configuration:
select a rule to edit its settings; compact +, −, ↑ and ↓ actions sit in the list
header. The first enabled matching rule handles F4. Disabled rules retain their
place, and the final Fallback row retains the previous custom or system editor.
The fallback cannot be removed or reordered.

Each rule selects wildcard masks or a regular expression, optional case
sensitivity, a system editor or absolute executable, literal argument rows and an
optional absolute working directory. Empty working directory means the executable
folder, preserving the existing launch behavior. Wildcards support `*`, `?`, `;`
separators and `|` exclusions; regex searches the filename and supports `^` / `$`
anchors. Matching never uses a displayed or lossy path as native identity.
Non-Unicode filenames use the preserved fallback and are still passed losslessly.

New rules start as enabled `*` rules using the system editor. Adding an argument
focuses a blank input without changing argv. Typing adds one argument; Enter on a
blank input explicitly adds an empty string. Each row is exactly one argument:
`two words` stays one value, quotation marks remain literal, and no implicit shell
parsing, environment expansion or interpolation is performed. Only a complete
`{file}` argument substitutes the captured native file path.

The settings codec migrates versions through 29 with an empty rule list and the
existing editor unchanged. Version 30 requires the list. Backend validation
checks every saved pattern, including disabled rules; errors identify the rule
number. Both Windows and Linux select the association on their backend launch
worker, validate the selected program, and use native argv/current-directory APIs.
A failed matching program never falls through to a different editor.

Drafts survive Options topic changes and use the normal OK/Cancel transaction.
Native program/directory picker results are tied to the draft revision, so late
results after selecting, reordering or removing a rule are discarded. Untouched
raw native paths survive display and settings round trips.

Targeted coverage lives in core editor validation, platform-common editor matching
and editor preferences codec tests, both native editor launch test modules, the
presenter editor draft/picker tests, and the editor/argument-list UI tests. Build
and test results are recorded by the parent integration task.
