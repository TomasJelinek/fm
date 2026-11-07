# User Commands (design; not yet implemented)

## Purpose and interaction

Run a user-defined command in a chosen working directory with one click. A
dedicated toolbar sits immediately above the bottom command line. It occupies
no space when there are no configured commands. Each item has an icon and an
optional label to its right; a tooltip shows its name and launch summary.
Overflow uses a final menu, without making the command line taller.

Options gains a **Custom Commands** page with an ordered, selectable list,
Add/Edit/Remove, move up/down and a preview. Right-click anywhere on this toolbar
opens a context menu containing **Settings…**, which opens that exact page and
selects the clicked command. An empty toolbar is configured from Options. A
settings selection, preview, icon hover or history recall never launches a command.

## Salamander reference and adaptation

Reviewed `D:/experiments/salamander-main/src/usermenu.h` (CUserMenuItem),
`dialogs4.cpp` (CCfgPageUserMenu), `mainwnd4.cpp` (ExpandCommand2) and
`execute.cpp` (ValidateUserMenuArguments/ExpandUserMenuArguments). Salamander
separates name, executable, arguments, initial directory and icon, supports
shell/terminal/window choices, toolbar visibility, groups/separators and
selection-dependent expansion. Its validation prevents mixing grouped and
per-item expansion. Its fixed ANSI buffers and short-path conversion are not
appropriate here: retain native Unicode paths and explicit platform boundaries.
This is behavioral inspiration; no source code is copied.

## Model and execution

`UserCommand` has a stable ID, name, enabled flag, optional toolbar text, icon
reference, working-directory template, invocation mode and arguments. Keep a
bounded list (128 commands, 64 arguments per direct launch); these are resource
budgets, not global path length limits. Settings are versioned and written by the
existing backend settings service. Duplicate IDs are rejected; duplicate labels
are allowed. Shortcut actions use `user-command.<id>` in the configurable map.

Two explicit modes avoid ambiguous quoting:

- **Program:** native executable path plus an argument list. Pass each expanded
  argument separately to the platform process API; never join it into a shell
  command. Use this as the default. Executable choices are explicit, independent
  of the browsed directory's PATH or file associations.
- **Terminal command:** choose an available shell/terminal profile and enter its
  script text. Run as ordinary user, with the same terminal service as the command
  bar. Script syntax is deliberately user-authored. A directory path is passed
  as the working directory, never concatenated into `cd ... && ...`.

Templates support `${active.path}`, `${other.path}`, `${file.path}`,
`${file.name}`, `${file.stem}`, `${file.extension}` and `${selected.paths}`.
The last expands only as whole argument-list elements in Program mode. A
per-item launch option explicitly executes once per captured selected item;
group and per-item expansion cannot be combined. Shell mode initially supports
scalar variables only through backend-created process environment values, with
shell-specific reference snippets inserted by the picker. Never interpolate
untrusted file names directly into script syntax. Environment values are data,
not subject to recursive template expansion. Missing variables and embedded NUL
fail before execution. Preview shows resolved values with native identities
retained separately; it is never an executable command reconstruction.

Capture panel/selection/working directory at click time, resolve and validate on
a bounded backend worker, then launch through separate Windows/Linux/macOS
implementations. Archive member placeholders require an explicit future export
policy and initially give an unsupported-source message; displayed archive paths
are not executable native paths. Do not silently elevate or save credentials.
Use the existing job/status error presentation for failures. Never log full
command text or persist it into command history without the user's action.

## Icons, accessibility and testing

Prefer an explicitly chosen local icon, otherwise the executable's system icon,
otherwise the app's generic command icon. Resolve and decode icons on backend
workers with the established image limits/cache. Store a path/reference rather
than unbounded embedded data. Theme and density determine presentation; keyboard
focus, accessible labels and disabled states are required.

Implementation tests must cover Unicode/long native paths, quote/semicolon/dollar
characters, raw non-Unicode Unix names, missing working directories, stale
selection, empty selections, argument expansion cardinality, cancellation before
launch, malformed settings/icons, right-click settings routing, hidden empty
toolbar and compact overflow. Each platform tests the actual argv and working
directory received by a helper process; it must not test a simulated shell string.
