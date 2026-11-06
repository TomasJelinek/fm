# Accepted follow-up stages

These user requests extend the active work; they do not replace unfinished work.
Complete and verify each stage before starting the next.

Stage 1 is implemented and verified; results and platform/profile limits are in
[the verification record](../../verification/midnight-explorer-operations.md).
Stage 2 is also implemented: see the [viewer verification record](../../verification/viewer-implementation.md)
for the independently built plugins, F4, platform checks and current limits.
The next unfinished operation stage is the complete copy/move workflow in stage 3.

1. Finish native rename/delete/F7, locale and tab persistence, smooth scrolling,
   panel modes, Midnight Explorer branding and optimized release configuration.
   Include [New empty file / Shift+F4](2026-11-05-empty-file.md), promoted into this
   stage by the latest user request. Commit and push relevant sources; keep
   generated files under `_build`.
2. Implement the [viewer plugin and F4 plan](2026-11-05-viewer-plugins.md): basic
   text/binary, source highlighting, Markdown, independent plugin projects/tests.
   The user chose one Git repository with independent Cargo workspaces under
   `plugins/`; only public API/SDK is shared, and plugin builds/tests cannot modify
   the host. Verify exported plugin projects without application sources.
   Keep empty-file creation separate from automatically launching the editor;
   explicit F4 is available through the configured external editor.
3. Implement copy and move according to the existing architecture, consulting
   Salamander. Use a bounded background operation queue, multiselection and overwrite
   prompts showing source/target metadata with per-item and remaining-conflict choices.
   Use bounded parallelism only where storage classification supports it; default
   to serial work for HDD/network/unknown storage. Hash the source stream and
   independently reread/hash the saved destination. Do not accept same-length or
   same-buffer hashing as verification. For move, begin source deletion only after
   every item in the operation has copied and verified successfully; revalidate
   source identity/version before deletion. Cancellation/conflict/failure must
   preserve sources and existing targets according to the publish contract.
4. Add a toolbar action and dialog for file attributes and timestamps, including
   multiselection. Consult Salamander's behavior. Mixed values need explicit
   unchanged/set/clear semantics; platform capabilities determine available fields.
   Run preparation/apply/progress on the backend with identity revalidation and
   per-item error reporting; no implicit recursive traversal through links.
5. Add a modeless Find dialog inspired by Salamander (distinct from panel prefix
   search): name masks, starting locations, recursive search and optional content
   query, incremental bounded results, cancellation and progress. Keep both panels
   usable while searching; opening a result must validate its current identity.
   Final detailed scope/defaults follow source inspection and tests, not UI I/O.
6. Implement filesystem plugins according to the finalized VFS/ABI design,
   including migration of native browsing behind its per-platform provider.
   Support freely available open-source implementations of ZIP, 7z, GZIP,
   TAR/TAR.GZ and suitable additional open formats. Enter and double-click on
   a recognized archive mount it for folder-like navigation; expose only operations
   the provider can implement safely. Copy/extract through the verified copy engine,
   never directly concatenate member names into native output paths. Unsupported
   archive writes remain explicit capabilities, not misleading enabled controls.
   Add a separate local-network-neighborhood provider with Windows, Linux and macOS
   implementations, consulting Salamander's network browser. Discovery produces
   incremental results and supports cancellation/manual locations; it cannot promise
   visibility of devices that do not advertise or are blocked by network policy.
   Plugin projects/tests remain independent and share only the public API/SDK.
7. Add expandable directory/archive/provider branches in both panels. `+` means
   known children in a collapsed branch, `-` collapses an expanded branch, and
   confirmed empty folders have no disclosure icon. Determine child availability
   asynchronously; unknown/loading/error must not be falsely labeled empty.
   Expanded branches contain folders/provider containers, not their regular files.
   Disclosure activation changes expansion only; Enter/double-click navigates the
   panel into the selected folder. Support multiple independent expanded branches,
   lazy enumeration, cancellation, bounded visible-row virtualization and provider
   identities across native/archive/network boundaries. Preserve selection and
   safely handle watched changes, unavailable providers and link cycles. The user
   explicitly chose inline expansion within the existing listing: regular files
   remain in the current directory's listing, while inserted branches contain only
   folders and browsable containers. This is not a separate folder-only view mode.
8. Add a panel-toolbar filter button and dialog with directly switchable wildcard,
   regex and typed-expression modes. Expressions may combine declared file fields
   such as name, size, modified time, kind and attributes; they are not executable
   scripts and have no shell, network or arbitrary filesystem access. Compile,
   validate and evaluate filters on the backend with bounded work and latest-wins
   revisions. Show syntax errors and an active-filter indicator, and provide Clear.
   Integrate with prefix search, sorting, refresh, multiselection and inline tree
   ancestor context without making hidden rows accidental mutation targets.

Each subsequent stage updates design, shortcuts, README/security where applicable,
has comprehensive owned-fixture tests, and receives independent review before
delivery. A pending stage is not claimed implemented by documenting it here.
