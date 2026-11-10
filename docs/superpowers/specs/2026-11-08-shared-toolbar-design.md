# One toolbar for the active panel

The user requested one application toolbar in place of the duplicated panel
toolbars, with a visible checksum action and the other important panel actions.
The accepted design places one horizontal bar above both panels. Drives, paths
and tabs remain per-panel. The follow-up puts View and Sort back on each drive
bar; in the compact layout they share the panel's path row. Existing F-key
copy/move/delete controls remain.

The bar retains Back/Forward and history menus, Parent, Refresh, New File,
Attributes, Find, Compare Files/Directories and Hot Paths.
It adds Checksums, Folder Sizes and Properties. Buttons dispatch existing typed
callbacks for the active panel; backend operation admission remains authoritative.
Existing native/archive and parent-only enablement gates remain. Tooltips use
configured shortcut labels. The checksum symbol is an original SVG hash/check
mark, also replacing the unrelated Properties artwork in the Checksums menu.

The latest follow-up moves Options from the footer to a fixed gear button at the
right end of this toolbar. An application moon icon immediately to its right
opens About with the application version. These two application-wide controls
stay visible while the panel-action strip scrolls. The footer now contains only
the twelve function keys and Jobs, reclaiming the removed Options button's width.

Compare Directories uses two filled amber folder silhouettes above a comparison
arrow, distinct in both shape and color from Compare Files' document outlines.
Light/dark SVG variants are shared by the toolbar, F10 menu and directory
comparison dialog; disabled controls retain the existing theme treatment.

A constrained horizontal viewport with explicit overflow controls keeps the
320×200 window minimum. Wide layouts show the tools together; narrower layouts
scroll rather than enlarging the window minimum or shrinking targets below useful
sizes. Desktop/touch density and large-font geometry are verified with actual
rendered controls.

The 9 November follow-up moves Filter to each panel, immediately before View.
Its checked state and tooltip identify an applied filter; opening it activates
that panel even when the opposite panel was previously active. Network
Neighborhood also belongs to each drive bar and is enabled only when its provider
is available. Neither control is duplicated in the shared application toolbar.
At compact sizes the Filter tool remains in the panel's compact controls without
a second panel banner; Show all in its dialog clears the filter. Normal panel
filter banners have their own direct dismiss action. Clearing
disables the applied filter and preserves its expression for reuse. Configured custom commands remain in
their dedicated row normally; in compact layout the shared bar's Custom Commands
button opens the same controls in a popup bound to the captured pane/context.
This preserves a visible file row at 320×200 with 32px fonts and configured
commands. Executing/editing a command uses the existing callbacks.

Shared popup context includes the panel identity and location/model generation.
Changing that context invalidates open history/Hot Path menus, even when
both panels have identical numeric history revisions. Pressed buttons likewise
cannot change their target between pointer down/up. Native touch capture occurs
before Slint's delayed child dispatch; child-level capture alone is insufficient
for a scrolling toolbar. A separate cancellation flag rejects pointer commits
before the runtime's synthetic release, resets on the next press, and leaves
keyboard activation usable. The same boundary protects the footer. Existing hidden BookmarkMenu
dialogs remain per-panel so their callbacks retain the originating panel.
Drive picker and Hot Path keyboard routing continue to use captured choices.
View/Sort pickers bind their owning panel and location context, activate that
panel on opening, and reject a stale selection after navigation.

Alternatives considered: wrapping all tools into two rows costs browsing height,
especially at 320×200; a menu-only collapse hides the newly requested actions.
The chosen one-row scrollable bar preserves direct controls and a bounded layout.
No dependencies, native filesystem work, preferences schema or backend action
contracts change. Root approved this design under the explicit user request.
