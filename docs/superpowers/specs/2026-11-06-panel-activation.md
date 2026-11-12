# Panel activation by pointer

Any pointer press inside either panel activates that panel, including its path,
tabs, toolbar, drive bar, file list and unused space. The change is idempotent.
Shared footer/command controls, the splitter and detached popup surfaces do not
implicitly change panel ownership. Hidden panels have no active hit region.

Keep the focus of the clicked editor or dropdown. Observing the dispatched native
pointer event lets existing controls process their click and then updates panel
ownership, without putting an overlay over every control or duplicating handlers.
Clicks on blank panel space return keyboard input to that panel's file list.

Tests dispatch native pointer events for blank space, editors, toolbar/dropdowns,
right and middle buttons, popup/footer/splitter exclusions, swapped panels, resize
and zoom. Path drafts must remain intact when panel activation changes.
