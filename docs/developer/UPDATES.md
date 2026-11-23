# Updates

Midnight Explorer can check GitHub for a newer release, download it, verify it against
the project's release key and install it into its own folder, keeping the user's
settings, panels and tabs. This document describes the protocol, the trust model, the
installation swap and rollback, and how the release key is handled. The user-facing
description is in the [user guide](../USER_GUIDE.md#updates); the network and
signature summary is in [SECURITY.md](../../SECURITY.md#update-check).

All update code is in `crates/midnight-update`, the application's only network code.
It runs on its own thread (`midnight-update`); the UI thread only queues requests and
shows snapshots. `crates/midnight-explorer/src/updates.rs` connects it to the window,
and `crates/midnight-ui/ui/update-controls.slint` draws the button and the dialog.

## User interface

- **Toolbar button** (between the toolbar's actions and Options, beside About): the
  tooltip and accessible name follow the state: "Check for updates" (idle or up to
  date), "Checking for updates…" with a moving line along the button's bottom edge,
  "Version X is available" with an accent dot, "Downloading version X: 45%" with a
  progress line, "Verifying version X…", "Restart to update to version X" with an
  accent dot, and "Update failed: reason" with an error-coloured dot. Clicking opens
  the dialog and, when idle, up to date or failed, checks again. A toolbar too narrow to
  keep a useful strip of actions beside three buttons (touch spacing at 320 px) leaves
  it out; About's Check for updates remains.
- **Dialog** (Software update, an embedded dialog like About): a headline for the
  state, the download progress, the failure, the installed and newest version
  (preview, publication date, skipped), the time of the last check, why this copy or
  release is not installed automatically, the release notes as plain text (headings,
  emphasis and link targets removed, at most 1,600 characters, "…" when cut) and a note
  on what the check sends. The footer has one primary action for the state: Check now,
  Download and install, Restart now or Open release page, or, disabled, the step under
  way (Checking…, Downloading…, Verifying…); Skip this version beside it while a version is offered;
  and Later or Close (Escape). Opening focuses the primary action, and when the state
  changes the keyboard moves to the new one.
- **About** has a Check for updates button beside View license; it closes About and
  opens the dialog with a check.
- **Options › General › Updates**: "Check for updates at startup" and "Include preview
  releases", side by side, both on by default and part of the Options OK/Cancel draft.

Downloading starts only when the user clicks Download and install, and installing only
when the user clicks Restart now. A found update is never downloaded automatically:
an archive is over 100 MB and the user may be on a metered connection. A verified,
staged update survives a restart of the application and is offered as ready ("Restart
now") at the next start; closing the window does not install it.

## Check

`GET https://api.github.com/repos/TomasJelinek/fm/releases?per_page=30` with
`Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28` and
`User-Agent: Midnight-Explorer/<version>`; no cookies, credentials or other headers.
Drafts and tags that are not semantic versions (`v0.5.1`, `0.6.0-beta.1`) are left
out. The newest version above the running one by semantic-version order is offered.
A release is a *preview* when GitHub marks it as a pre-release or its version has a
pre-release part; previews are offered only with "Include preview releases".

That option is on by default because every release so far (0.1.0 and 0.5.0) is a
GitHub pre-release: with it off, no current user would be offered anything until the
first stable release. Users who want only stable releases turn it off.

The startup check runs on the update thread 5 seconds after the window shows with the
user's settings, never before or during startup, and only when "Check for updates at
startup" is on. It asks at most once in 20 hours (`update-state.json` records the last
answered check). It shows nothing unless it finds a version the user has not skipped:
a failure leaves the button idle and backs off for 30 minutes, doubling up to a day.
GitHub's unauthenticated limit (60 requests an hour per address) answers 403 with
`x-ratelimit-remaining: 0` or 429; the check then waits until `x-ratelimit-reset` or
`Retry-After` (an hour without either) before asking again at startup. Check now always
asks and shows the failure.

**Skip this version** records the version in `update-state.json`. The startup check
then stays quiet about exactly that version; Check now still shows it, marked as
skipped, so it can be installed later. A newer version is offered again.

## Transport

- **Windows:** WinHTTP through `windows-sys`: the system's TLS stack, certificate
  store and proxy settings (`WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`), TLS 1.2 or 1.3,
  cookies off. No TLS crate is linked.
- **Linux:** `ureq` 3.5.2 over `rustls` 0.23 with the `ring` provider, trusting the
  distribution's certificate store (`rustls-native-certs`, which reads the usual
  bundle files and honours `SSL_CERT_FILE`/`SSL_CERT_DIR`). No OpenSSL is linked and no
  certificate bundle ships with the application. `HTTPS_PROXY`/`ALL_PROXY` are honoured.
- Both transports have redirects off. The client follows at most 5 redirects itself and
  checks every address against the policy: `https://` on port 443 to `api.github.com`,
  `github.com`, `objects.githubusercontent.com` or `release-assets.githubusercontent.com`
  only, without user information. Plain HTTP and other hosts are refused before any
  connection, also by the transport itself.
- Timeouts: 15 s to connect, 30 s for each answer and each read. Size limits: 4 MiB for
  the release list, 64 KiB for `SHA256SUMS`, 4 KiB for its signature, the size GitHub
  lists for the archive (at most 2 GiB). A longer body is cut off and refused.
- Downloads go to `<private folder>/download/<version>/` as `<name>.part`. A download
  that breaks off resumes with `Range: bytes=N-` (up to four attempts); a 206 answer is
  accepted only from the requested byte, a 200 answer starts over. The private folder
  is `%LOCALAPPDATA%\Midnight Explorer\updates` on Windows and
  `$XDG_CACHE_HOME/midnight-explorer/updates` (`~/.cache/...`, mode 0700) on Linux, so
  no other account can change a file between its check and its use.

### Test-only endpoint

Debug builds read `MIDNIGHT_UPDATE_TEST_SERVER=http://127.0.0.1:PORT` (the release list
at `/releases`) and `MIDNIGHT_UPDATE_TEST_PUBLIC_KEY=<file>` (a test key in place of
the built-in one) for local end-to-end runs. Only `127.0.0.1` over plain HTTP is
accepted there. `MIDNIGHT_TEST_DATA_ROOT=<absolute folder>` (debug builds only) moves
the profile there: the Windows settings and command artwork
(`<folder>\Midnight Explorer\`) and the update folder on both systems; with
`LOCALAPPDATA` (Windows) or the `XDG_*` variables (Linux) pointed at the same folder the
thumbnail cache follows, so an end-to-end run never reads or writes the user's profile.
Release builds ignore all three: `Endpoints::loopback_test` returns
`None` and `TrustAnchor::built_in` ignores the variable when `debug_assertions` is off,
which `cargo test -p midnight-update --release` checks
(`only_debug_builds_accept_a_loopback_test_server`).

## Trust

The project's release key is an Ed25519 key in [minisign](https://jedisct1.github.io/minisign/)
format. Its public half is `packaging/update-signing.pub`, compiled into the
application (`include_str!`); the secret half exists only with the project owner,
outside every repository and checkout. Each release's `SHA256SUMS` is signed with it
into `SHA256SUMS.minisig` (pre-hashed BLAKE2b-512, minisign's default), with the trusted
comment `timestamp:<unix> file:SHA256SUMS release:midnight-explorer-<version>`.
Verification uses `minisign-verify` 0.3.0 (MIT, no dependencies).

An update is installed only if every check passes, in this order:

1. The build has a key (a build from the placeholder file has none: the dialog says
   that this build installs no updates and offers the release page).
2. `SHA256SUMS.minisig` is a pre-hashed minisign signature by that key (key ID and
   both Ed25519 signatures, over the list and over the trusted comment). Legacy
   signatures are refused.
3. The signed trusted comment names exactly the offered release
   (`release:midnight-explorer-0.5.1`), so an old signed list cannot be replayed as a
   newer release.
4. `SHA256SUMS` lists the archive `midnight-explorer-<version>-<target>.<zip|tar.gz>`,
   whose name carries the offered version and this platform's target.
5. The downloaded archive's SHA-256 equals its signed line. Only now is it opened.
6. The archive holds only regular files and folders below the single folder
   `midnight-explorer-<version>-<target>/` (no links, `..`, absolute paths, names
   Windows refuses or duplicates; at most 65,536 entries and 6 GiB unpacked).
7. Its `BUILD-INFO.json` names the offered version and this target, its executable
   exists and matches `binary_sha256`, and every module file it lists (viewers,
   filesystem and thumbnail packages, the authorization helper) matches its size and
   SHA-256.

The signature and checksum list are fetched and checked before the archive, so an
unsigned or forged release downloads nothing large. A GitHub account or release that
someone else controls cannot deliver an installable update without the secret key.
Each refusal names its check ("The release is signed with a key this version does not
trust.", "… does not match its signed checksum: the download is damaged or was
changed.", "The release archive holds 0.5.0 for …, not the offered 0.5.1 for …").

Releases without `SHA256SUMS.minisig` (0.1.0 and 0.5.0) or without this platform's
archive are shown with the reason and the release page instead of Download and
install.

## Staging

After the checks, the archive is extracted into
`<installation>/.update/staging/<release folder>/` (the installation's volume, so the
swap is a rename), checked as in step 7 and recorded in `staging/ready.json`; the
downloaded archive is then deleted. The installation must be an extracted release
folder: `BUILD-INFO.json` beside the running executable, naming this target and this
executable, and a folder this user can create, rename and remove entries in.
Otherwise the dialog explains why and offers the release page:

- a development build or a package that another installer manages has no
  `BUILD-INFO.json`;
- a folder that only administrators can change (for example below
  `C:\Program Files` or `/opt`) is left to an administrator: the application never
  asks for elevation to install an update.

## Installing: the swap

"Restart now" closes the window the ordinary way (running jobs are offered first,
settings and the session with its panels and tabs are saved). After every worker has
stopped, the application copies its own executable to `.update/updater/` and starts that
copy with `--apply-update <installation> <staged folder> <working folder> -- <original
arguments>`, then exits. The updater is the previous version's code, which has proved
that it runs on this machine, so it can restore the previous version whatever the new
one does; and it runs from a copy, so the installation's files are not in use. It opens
no window. It:

1. Waits up to 60 s for an exclusive lock on `.update/instances.lock`. Every copy
   started from the folder holds a shared lock for its whole life, so this waits until
   all have exited. If another window stays open, nothing changes, the previous version
   starts again, and the dialog says why.
2. Checks the staged release against its `BUILD-INFO.json` again.
3. Writes `pending.json` (`from`, `to`), moves every entry of the installation folder
   except `.update` into `.update/backup/`, then every entry of the staged release into
   the installation folder. These are renames on one volume. A rename that a scanner or
   a closing process holds up (Windows sharing or lock violations, Linux `EBUSY`) is
   tried again with backoff for 15 s; when one fails for good, everything already
   moved is moved back and the previous version starts again.
4. Removes the used-up staged copy and `ready.json` (so the new version does not offer
   it again), releases the lock and starts the new executable, the name that the new
   `BUILD-INFO.json` gives (`me.exe` now, a renamed executable later), with the original
   arguments in the original working folder.

The user's settings are not in the installation folder (`%LOCALAPPDATA%\Midnight
Explorer\preferences.json`, `~/.config/midnight-explorer/preferences.json`), so the new
version reads the same settings and restores the same panels and tabs; explicit
`--left`/`--right` arguments are passed again.

## Startup check and rollback

The new version holds the instance lock and finds `pending.json` naming its own
version. Once its window shows with the user's settings read, it writes
`.update/confirmed`. The updater waits up to 180 s:

- **Confirmed:** it removes the backup and `pending.json`, writes `result.json` and
  exits. The new version's dialog says "Updated to version Y."; a later start does not
  repeat it.
- **The new version exits before confirming** (it crashed, could not create its
  window, or refused to start): the updater moves the new files to `.update/failed/`,
  moves the backup back, removes `pending.json`, writes `result.json` and then starts
  the previous version with the same arguments, which reads it and says "Version Y did
  not start correctly, so the previous version was restored." with the reason. That
  start also removes `failed/`.
- **No answer in time while the new version runs:** the backup is kept and the updater
  exits; the next start after a confirmation removes it.

`update.log` in `.update/` records every step with Unix timestamps (at most 1 MiB).
A start that finds no `pending.json` removes what earlier updates left (`failed/`,
`updater/`, a backup whose update was confirmed). While `pending.json` names another
version, a start leaves the updater's folders alone.

If the computer loses power during step 3, the folder can hold a mix of both versions;
the previous version's files are in `.update/backup/` and the new one's in
`.update/staging/`. Moving the backup's entries back restores the previous version.

## Keys

The owner creates the key pair once, on their own machine:

```powershell
python -B scripts/release-signing.py generate-key --secret-key <path outside every checkout>\midnight-explorer-release.key
```

It refuses a secret-key path inside the repository and an existing file, asks for a
password (at least 12 characters; the key is encrypted with scrypt using libsodium's
"sensitive" limits, about 1 GiB of memory for a few seconds), writes the secret key
(mode 0600 on Linux), writes the public key into `packaging/update-signing.pub`
(replacing only the placeholder) and prints both paths. Commit the public key; builds
from that commit on trust only releases signed with this key. Back up the secret key
and its password: without them, releases can no longer update older copies, which
would then need a manual download of a version with a new key.

The tool needs only Python 3.11 or later: Ed25519 is implemented in the script (RFC 8032,
checked against the RFC's test vectors in `scripts/tests/test_release_signing.py`),
BLAKE2b and scrypt come from `hashlib`. It is not constant-time, so use it only on the
owner's machine. Its files are minisign-compatible, so the official `minisign` tool
(`minisign -S -s KEY -m SHA256SUMS -t "<trusted comment>"`) can sign as well.

Signing a release is in [Release build](../RELEASE_BUILD.md#signing-the-release).
A test key, checksum list and signature made by the script are in
`crates/midnight-update/tests/fixtures/` (the test key's secret half was discarded);
`the_release_scripts_signature_verifies` checks that the application accepts them.

## Tests

`cargo test -p midnight-update` (no test touches the network; the transport is
injected):

- `release`: semantic-version order with previews, drafts and non-version tags left out,
  the preview option, skipped versions, release notes as plain text.
- `trust`: a good signature; another key, a changed list, a changed trusted comment, a
  signature for another release, a legacy signature, no key, a damaged signature file,
  a changed archive and malformed lists are each refused with their reason; the release
  script's signature verifies.
- `client`: rate limits (403 with `x-ratelimit-remaining: 0`, 429), offline, redirects
  only to GitHub over HTTPS, a redirect loop, size limits, a broken download resumed
  from its partial file, cancellation, the URL policy, and the debug-only test endpoint.
- `archive`: extraction and `BUILD-INFO.json` checks: renamed executables, another
  version, a changed module or executable, escaping entries, no `BUILD-INFO.json`.
- `apply`: in temporary folders, a confirmed update, a renamed executable, a rollback
  when the new version exits before confirming, another running copy, a staged copy
  changed after staging, a staged folder elsewhere, settings outside the installation
  untouched. `apply_windows`: a file held open without delete sharing (as a running
  program's files can be) that is released in time, one that stays held (everything
  moves back), and a running executable that can be renamed.
- `controller`: requests return at once while the transport blocks, the whole check →
  download → verify → stage flow, a tampered archive, another key (no archive
  downloaded), a build without a key, the startup interval and skip, rate limits and
  offline at startup, and a staged update offered as ready after a restart.

`scripts/tests/test_release_signing.py` covers the release tool, the UI's states are
covered by `crates/midnight-ui/tests/update_dialog.rs`, and the settings schema by
`crates/midnight-platform-common/src/settings/updates_tests.rs`.
