# User viewer associations

Options > Plugins lets the user select an installed viewer, edit its literal
extensions and restore manifest defaults. A priority list resolves overlapping
extensions across native text/Markdown plugins and isolated image/PDF workers.
Manifest files, plugin ABI and trust checks are unchanged. This is routing only;
an extension cannot grant a decoder a new capability.

Store overrides by stable plugin ID, separately from the ordered preference list.
An absent override inherits the manifest; an empty list removes explicit extension
matches. The installed Basic viewer remains the fallback for otherwise unmatched
files. Alt+F3 ignores extension choices and uses Basic. Disabled, unavailable and
duplicate-identity providers cannot be selected. Missing plugin preferences survive
uninstallation and take effect only if the same valid ID is installed again.

Extensions are case insensitive literal suffixes, including compound suffixes.
The editor accepts `rs`, `.rs` and `*.rs`, separated by spaces, commas or semicolons.
Wildcards, paths and control characters are rejected. Specific matches beat Basic
fallback; longer suffixes beat shorter suffixes, then the explicit user order,
then the existing provider-mode/manifest priority and stable ID decide.

These choices share existing restart-based plugin enablement. OK persists them;
Cancel restores the previous draft. Reset defaults removes routing overrides and
priority preferences. Existing viewer sessions and plugin packages stay unchanged.
All discovery and dispatch remain backend work; UI handles only bounded settings.

Validation covers normalization, ambiguous suffixes, priority, empty overrides,
disabled/missing providers, Alt+F3, strict settings migration, and OK/Cancel/reset.
