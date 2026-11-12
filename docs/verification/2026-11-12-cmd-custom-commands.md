# Native Cmd Custom Command qualification

This task qualifies a language-specific direct external-argv contract. It does
not claim that one encoding makes arbitrary Cmd script reparsing safe.

## Actual native investigation

The standalone probes use the installed system Cmd and the existing owned native
helper, which records actual cwd, argv and raw environment as UTF-16 byte records.
All outputs and sentinels are confined to `_build/qa/cmd-custom-20261112/`.

- `probe.json`: ordinary quoted delayed references preserved nonempty metacharacter
  data and raw environment, but lost trailing-backslash/quote argv and left empty
  references literal. This ruled out simply removing the Unsupported fences.
- `probe-encoded.json` and `probe-controls.json`: backend-style quoted companions
  used as unquoted delayed references preserved exact external argv for Unicode,
  percent, bang, ampersand, caret, parentheses, embedded quotes, empty arguments,
  trailing backslashes, unpaired UTF-16, CR/LF and tabs. No sentinel was executed.
- `probe-call.json`: CALL re-expanded percent variables and changed carets; a
  quoted-data invocation failed. This is concrete evidence against universal
  quoting claims. `probe-pipe.json` passed the tested simple external pipeline,
  which does not qualify every grouped/builtin pipeline or nested shell.
- `source-lines.json`: both LF and CRLF source ran only the first owned helper,
  ignoring the second physical line. The adapter therefore refuses multiline Cmd
  source rather than rewriting it or silently omitting commands.

## Implementation

The source remains literal. Exact raw declared values remain in the child
Unicode environment. Private fixed `ME_*_ARG` companions reuse the same Windows
C-runtime argv encoder as ordinary process launching. Unknown/caller-supplied
companion keys, NULs and duplicates remain invalid. Inherited undeclared raw and
companion keys are removed case-insensitively.

Custom Cmd selects `/D /U /E:ON /V:ON /S /K` through the existing native launch
path. Ordinary command-bar Cmd remains `/V:OFF`. Cmd record/initial command/fixed
reference limits are checked without truncation; repeated references are budgeted
case-insensitively with saturating arithmetic. This is not a parser for dynamically
constructed user-authored script variables. No parent environment, elevation,
process quota, cleanup thread or cancellation/no-kill policy changed.

The picker inserts `!ME_FILE_PATH_ARG!` without surrounding quotes and declares
its raw scalar requirement. Its help explicitly limits companions to external
program arguments and distinguishes builtin and CALL semantics.

## Regression evidence

`red.log`: the new actual native argv test failed exactly at the existing
Unsupported preflight (1 failed, 188 filtered out in 0.28 s; build 44.27 s).
The first implementation run passed the direct argv/raw environment vectors,
size/cancellation checks and retained PowerShell test. Its builtin fixture
incorrectly expected `echo value>file & next` to omit the source's trailing space;
`first-green-fixture-whitespace.log` preserves that failure. The fixture source
now uses prefix redirection and leaves the exact byte expectation unchanged.

The second builtin fixture failure was an incorrect byte-encoding expectation:
Cmd `/U` makes TYPE output UTF-16. `second-green-fixture-type-encoding.log`
preserves that observation. Its corrected expectation checks exact UTF-16 content;
ECHO content and final builtin CD directory remain exact assertions.

## Working-directory review and regression

Independent parent review found a real native boundary: Cmd accepts process
startup with a verbatim DOS cwd, then silently changes it to `C:\Windows`.
`cwd-red.log` reproduces that through production preparation and the native helper
(1 failed in 1.51 s). `cwd-probe.json` separately records ordinary, verbatim and
long-directory observations. No helper wrote outside the owned QA fixture.

The correction removes only a typed VerbatimDisk prefix and accepts the DOS
spelling only after two simultaneously live directory handles have equal volume
and file identities. Actual trailing-dot and trailing-space fixture directories
prove that changed DOS semantics are refused. UNC/device prefixes are refused.
Native tests cover ordinary/verbatim Unicode cwd, a drive root, and a junction
alias. Captured raw `ME_ACTIVE_PATH` retains its original verbatim spelling.
The native long-cwd test accepts either exact successful cwd or explicit native
267/206 refusal, never fallback/truncation. This machine returned 267 for long
startup; generic long file-manager paths remain supported.

The final surrogate fixture separates a high and low surrogate with an ASCII
character; these are genuinely isolated UTF-16 units, not a valid pair.

## Final targeted Windows results

All commands used the current locked SDK, the shared `_build` target and private
same-user QA TEMP. No additional native process tracker or cleanup path was added.

| Log under `_build/qa/cmd-custom-20261112/` | Result |
| --- | --- |
| `green.log` | Native custom terminal tests: 9 passed in 2.95 s; build 3.75 s. Includes retained PowerShell, direct argv/raw environment, builtins, cancellation, size/line refusal and cwd regressions. |
| `environment-green.log` | 4 passed in 0.00 s; build 2.98 s. Includes stale companion sanitation and malformed/duplicate/NUL refusal. |
| `native-process-green.log` | 4 passed in 0.46 s; 2 ignored subprocess fixture entrypoints are invoked by their enabled parents. Exact ordinary Cmd tail and native console ownership remain covered. |
| `core-green.log` | 1 passed in 0.00 s; build 2.03 s. Cmd metadata admission still rejects caller-owned companion keys and checks cancellation. |

The actual UI picker regression
`cmd_picker_declares_raw_scalar_and_inserts_complete_native_argument_companion`
passed (1 test in 0.00 s; build 0.46 s) in
`_build/qa/tree-rename-20261112/cmd-form-green.log`. Final combined all-target
Clippy passed on Windows and Linux, as did the affected backend/UI gates. The
[combined checkpoint](2026-11-12-config-rename-cmd-checkpoint.md) records exact
scope, timings and the newly built debug executable. The Cmd-specific execution
contract is qualified on Windows; the Linux run covers shared code and UI.

## Primary references

Microsoft documents [Cmd switches and delayed expansion](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/cmd)
and [the native command/environment length limit](https://learn.microsoft.com/en-us/troubleshoot/windows-client/shell-experience/command-line-string-limitation).
Those documents motivate explicit options and bounds; the actual native argv,
control-character, builtin and reparse observations above are separate evidence.
