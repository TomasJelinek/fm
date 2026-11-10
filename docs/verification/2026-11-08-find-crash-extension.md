# Find wildcard crash and extension parity

The reported `*.*` search crash is reproduced by starting Find through the actual
button callback, publishing a streamed batch and replacing its sorted view under
the normal shared presenter refresh borrow. Previously, retiring the row model's
old order attempted `state.borrow_mut()` inside that refresh and panicked with
`RefCell already borrowed` at `find_windows.rs:313`. The regression includes
search restart and asserts that the last owner of an old permutation is retained
until backend disposal. It does not read the user's Outlook attachment contents.

Retirement now uses an interior-mutable order queue inside Find presentation
state. The normal presenter retirement drain transfers those owners to the
backend. Large permutations are not freed by a synchronous model notification.

Extension already had a visible header and command mapping. Its backend ordering
did not match panel Type sorting: absent extensions sorted as empty strings,
locale fallback groups reversed on descending order, and filename ties used the
whole raw path. The shared captured-key comparator now preserves panel semantics;
Find prepares both extension and secondary filename keys within its existing
16 MiB aggregate/4 MiB individual key limits. No locale work runs in UI callbacks.

## Evidence

Windows debug artifacts are under `_build/goal/find-crash/`:

- `red.log`: new wildcard/streamed-order UI regression fails at the original
  mutable presenter borrow, before the retirement correction.
- `extension-red.log`: all three new extension tests fail on the prior sorter,
  including parity using the actual Windows locale and secondary-key bounds.
- `backend-green.log`: 19 Find tests pass (actor 5, native backend 8, extension 3,
  prior sort 3). The synthetic attachment-style root recurses with `*.*` and has
  no content query.
- `long-path-green.log`: the strengthened native wildcard fixture passes with
  an explicit assertion that its raw root path exceeds 260 encoded bytes.
- `core-locale-green.log`: six core locale sorting tests pass after extracting
  the shared comparator, including invalid native names and deterministic ties.

The coordinated UI gate in `_build/qa/archive-alignment/green.log` passes all
five Find-window tests, including the causal streamed replacement/restart case;
the complete shared gate passes 36 tests. This is the post-fix software-window
GREEN corresponding to `red.log` above.
The final shared gate in `_build/qa/path-elision/final.log` also passes all
five Find-window tests and all 42 tests across the final coordinated UI changes.

The native `native_find` example uses winit/femtovg-wgpu, a real modeless window,
pointer Find/header clicks and synthetic batches/orders. Its Windows execution
passes with process exit 0; `native.log` records every asserted step: recursive
`*.*` StartFind, initial order, streamed order replacement during normal presenter
refresh, Extension ascending and descending commands, and the second StartFind
after completion. The final cleanup/success marker was reached. This is native
winit/femtovg-wgpu qualification of the causal UI sequence, using synthetic result
events; it is not a search of the user-reported attachment folder. The independent
native backend fixture exercises traversal under the long owned test root.

The final narrow source review found no concrete defect in the retirement handoff
or extension ordering. Scoped formatting, source-size and LF checks pass. No
dependencies or enabled dependency features changed in this Find follow-up.
