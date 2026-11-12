# Panel metadata expressions

The panel filter dialog has independent **Masks**, **Regex**, and **Expression**
modes. Masks and Regex retain their existing filename matching and keep folders
visible. Expressions evaluate captured listing entries, including folders. They
combine with quick search and persist independently for each panel. The parent
navigation row remains available.

Examples:

```text
kind = file and size >= 10MiB
name like "*.rs" or name like "*.md"
modified >= "2026-11-01T00:00:00Z" and not attributes has hidden
missing(attributes) or not attributes has hidden
kind = directory or (kind = file and present(size))
```

Keywords and units are case sensitive. `not` binds before `and`, which binds
before `or`; parentheses override this order. Quoted strings accept `\\`, `\"`,
`\n`, `\r`, and `\t` escapes. There is no script evaluation, arithmetic, implicit
local time, or file-content access.

| Field | Predicates and values |
| --- | --- |
| `name` | `=`, `!=` exact quoted name; `like` quoted `*`/`?` wildcard; `=~` quoted Rust regex |
| `size` | `=`, `!=`, `<`, `<=`, `>`, `>=` unsigned integer bytes, optionally `B`, `kB`, `MB`, `GB`, `TB`, `KiB`, `MiB`, `GiB`, `TiB` |
| `modified` | The same comparisons with quoted Gregorian timestamp `YYYY-MM-DDTHH:MM:SS[.fffffffff]Z` or explicit `+HH:MM`/`-HH:MM` offset |
| `kind` | `=` or `!=` with `file`, `directory`, `directory-link`, `symlink`, `other` |
| `attributes` | `has` with `readonly`, `hidden`, `system`, `archive`, `temporary`, `sparse`, `compressed`, `offline`, `encrypted` |
| Any field | `present(field)` or `missing(field)` |

Case sensitivity applies to name predicates; native names and operation paths
are preserved. Regex matches anywhere unless anchored. `like` matches the whole
name and uses one wildcard pattern, without the Masks mode separator/exclusion
syntax or its `*.*` exception for extensionless names. In an Expression,
`name like "*.*"` requires a dot; `name like "*"` includes extensionless names.

Missing metadata has three-valued semantics: comparisons, inequality, and their
negation remain unknown. Only a final true result is visible. An explicit true
branch of `or` can include a missing value; an explicit false branch of `and`
still excludes it. `missing(...)` makes such choices explicit. File size is
known only for regular files with a reported size; directory inode sizes and
link sizes do not become file sizes. Non-Unicode native names are unknown for
name predicates while their other metadata remains usable. Archive names are
opaque bytes without an encoding marker in the current provider contract. An
Expression name is available only when those bytes are valid UTF-8 and exactly
match the captured archive caption. Otherwise `missing(name)` is true; a lossy or
decoded display caption never becomes an authoritative name. Size and kind
predicates still work. Legacy Masks/Regex keep their existing archive behavior.

Attributes mean the provider's reported native flags. Linux listings currently
do not report these flags: a dot-name does not imply the `hidden` flag, and a
permission mode is not translated into `readonly`. Modified values retain the
listing timestamp's native precision. Refresh the panel to observe later
metadata changes. Explicit offsets compare as UTC instants independently of
display locale and timezone. Leap seconds, date-only/local times, fractional
sizes, negative sizes and overflow are rejected.

Compilation and projection run in backend workers. A failed validation leaves
the last valid filter active and preserves the dialog draft with a line/column
diagnostic. Disabled Expression drafts are also validated before saving. Queue
refusal leaves the draft retryable; canceled and obsolete validation results
cannot replace a newer draft.

Resource limits are 4,096 UTF-8 source bytes, 512 tokens, 256 expression nodes,
32 levels of parentheses/negation and 16 name patterns. Each name pattern has
a 64 KiB compiled-regex limit, 64 KiB DFA cache limit and regex nesting limit
32. These are library limits with reusable worker scratch, not an exact process
memory limit or an allocation-free guarantee. Projection checks cancellation
between bounded chunks and preserves raw entry identities and selection indices.

Preferences use schema 18 when either panel has Expression mode, including a
disabled expression. Required schema 16/17 checksum and command fields remain
present and round-trip unchanged. Old defaults retain the earlier encoding;
schemas before 18 reject Expression mode. Name-only consumers such as Find
masks and filename colors continue to accept only their existing modes.
