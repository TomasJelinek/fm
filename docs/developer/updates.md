# Updates

Midnight Explorer can check GitHub for a newer release, download it, verify it against
the project's release key and install it into its own folder, keeping the user's
settings, panels and tabs. An extracted release folder replaces its own files; an
MSI installation (a copy installed with the release's Windows Installer package, the
MSI) hands the verified MSI to Windows Installer. This document describes the protocol,
the trust model, the installation swap and rollback, the Windows Installer hand-off,
the files that record an update's state, and how the release key is handled. The user-facing
description is in the [user guide](../user-guide.md#updates); the network and
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
  progress line, "Verifying version X…", "Restart to update to version X" (for an
  MSI installation "Install version X") with an
  accent dot, and "Update failed: reason" with an error-colored dot. Clicking opens
  the dialog and, when idle, up to date or failed, checks again. A toolbar too narrow to
  keep a useful strip of actions beside three buttons (touch spacing at 320 px) leaves
  it out; About's Check for updates remains.
- **Dialog** (Software update, an embedded dialog like About): a headline for the
  state, the download progress, the failure, the installed and newest version
  (preview, publication date, skipped), the time of the last check, why this copy or
  release is not installed automatically, the release notes as plain text (headings,
  emphasis and link targets removed, at most 1,600 characters, "…" when cut) and a note
  on what the check sends. The footer has one primary action for the state: Check now,
  Download and install, Restart now (Install now for an MSI installation, whose
  headline and hint also say when Windows asks for administrator approval) or Open
  release page, or, disabled, the step under
  way (Checking…, Downloading…, Verifying…); Skip this version beside it while a version is offered;
  and Later or Close (Escape). Opening focuses the primary action, and when the state
  changes the keyboard moves to the new one.
- **About** has a Check for updates button beside View license; it closes About and
  opens the dialog with a check.
- **Options › General › Updates**: "Check for updates at startup" and "Include preview
  releases", side by side, both on by default and part of the Options OK/Cancel draft.
  Settings schema 40 stores them only when one is off
  ([recent schema versions](../settings-files.md#recent-schema-versions)); every
  earlier file reads as both on, so existing settings get the startup check.

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

That option is on by default because every release so far (0.1.0, then 0.5.0 to 0.6.3)
is a preview, published as a GitHub pre-release: with it off, no current user would be
offered anything until the first stable release. Users who want only stable releases
turn it off.

The startup check runs on the update thread 5 seconds after the window shows, once the
user's settings are read (it is skipped in a run whose settings are not read within two
minutes), never before or during startup, and only when "Check for updates at
startup" is on. It asks at most once in 20 hours ([update-state.json](#update-statejson)
records the last answered check). It shows nothing unless it finds a version the user has not skipped:
a failure leaves the button idle and backs off for 30 minutes, doubling up to a day.
GitHub's unauthenticated limit (60 requests an hour per address) answers 403 with
`x-ratelimit-remaining: 0` or 429; the check then waits until `x-ratelimit-reset` or
`Retry-After` (an hour without either) before asking again at startup. Check now always
asks and shows the failure.

**Skip this version** records the version in `update-state.json`. The startup check
then stays quiet about exactly that version; Check now still shows it, marked as
skipped, so it can be installed later. A newer version is offered again.

### update-state.json

What the check remembers between runs, in the private folder
([Transport](#transport)): `<private folder>/update-state.json`. It is not a setting
and is not in an exported settings file. Every field is optional; a missing field, a
damaged file or one of 64 KiB or more reads as the defaults, so losing the file only
means one extra check.

| Field | Type | Meaning | Default |
| --- | --- | --- | --- |
| `last_check` | number | Unix seconds of the last answered check. | `0` |
| `retry_after` | number | No startup request before this Unix second: GitHub's rate limit or a backoff. | `0` |
| `failures` | number | Failed automatic checks in a row; the backoff doubles with each. | `0` |
| `skipped` | string or `null` | The version chosen with **Skip this version**, for example `"0.6.3"`. | `null` |

## Transport

- **Windows:** WinHTTP through `windows-sys`: the system's TLS stack, certificate
  store and proxy settings (`WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`), TLS 1.2 or 1.3,
  cookies off. No TLS crate is linked.
- **Linux:** `ureq` 3.5.2 over `rustls` 0.23 with the `ring` provider, trusting the
  distribution's certificate store (`rustls-native-certs`, which reads the usual
  bundle files and honors `SSL_CERT_FILE`/`SSL_CERT_DIR`). No OpenSSL is linked and no
  certificate bundle ships with the application. `HTTPS_PROXY`/`ALL_PROXY` are honored.
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

Debug builds read three environment variables for local end-to-end runs:

| Variable | Value | Effect |
| --- | --- | --- |
| `MIDNIGHT_UPDATE_TEST_SERVER` | `http://127.0.0.1:PORT`, for example `http://127.0.0.1:8080` | The release list is fetched from `/releases` on that server instead of GitHub. Only `127.0.0.1` over plain HTTP is accepted. |
| `MIDNIGHT_UPDATE_TEST_PUBLIC_KEY` | path of a minisign public key file | A test key replaces the built-in release key. |
| `MIDNIGHT_TEST_DATA_ROOT` | an absolute folder | Moves the profile there: the Windows settings and command artwork (`<folder>\Midnight Explorer\`) and the update folder on both systems (`<folder>\Midnight Explorer\updates` on Windows, `<folder>/midnight-explorer/updates` on Linux). |

With `LOCALAPPDATA` (Windows) or the `XDG_*` variables (Linux) pointed at the same
folder as `MIDNIGHT_TEST_DATA_ROOT`, the thumbnail cache follows, so an end-to-end
run never reads or writes the user's profile. Release builds ignore all three: `Endpoints::loopback_test` returns
`None` and `TrustAnchor::built_in` ignores the variable when `debug_assertions` is off,
which `cargo test -p midnight-update --release` checks
(`only_debug_builds_accept_a_loopback_test_server`). CI and full validation run the
tests in the debug test profile only, so that release-build check is a manual step.

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
   - For an MSI installation the list must name the MSI
     `midnight-explorer-<version>-x86_64-pc-windows-msvc.msi` instead, and checks 6
     and 7 do not apply ([MSI installations](#msi-installations)).
5. The downloaded archive's SHA-256 equals its signed line.
   - Only now is it opened as an archive, through the handle whose content was
     checked; on Windows that handle keeps other programs from changing, renaming or
     deleting the file meanwhile.
6. The archive holds only regular files and folders below the single folder
   `midnight-explorer-<version>-<target>/` (no links, `..`, absolute paths, names
   Windows refuses or duplicates; at most 65,536 entries and 6 GiB unpacked).
7. Its [BUILD-INFO.json](../release-build.md#build-infojson) names the offered version
   and this target, its executable exists and matches `binary_sha256`, and every
   module file it lists (viewers, filesystem and thumbnail packages, the
   [authorization helper](../glossary.md#elevated-listing-helper)) matches its size
   and SHA-256.

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
swap is a rename), checked as in check 7 of [Trust](#trust) and recorded in
`staging/ready.json` ([Update files](#update-files)); the downloaded archive is then
deleted. The installation must be an extracted release
folder: `BUILD-INFO.json` beside the running executable, naming this target and this
executable, and a folder this user can create, rename and remove entries in.
Otherwise the dialog explains why and offers the release page:

- a development build or a package that another installer manages has no
  `BUILD-INFO.json` (an MSI installation updates with the MSI instead:
  [MSI installations](#msi-installations));
- a folder that only administrators can change (for example below
  `C:\Program Files` or `/opt`) is left to an administrator: the application never
  asks for elevation to install an update.

## Installing: the swap

"Restart now" closes the window the ordinary way (running jobs are offered first,
settings and the session with its panels and tabs are saved). After every
[worker](../glossary.md#worker) has stopped, the application copies its own executable
to `.update/updater/` and starts that copy with
`--apply-update <installation> <staged folder> <working folder> -- <original arguments>`,
then exits. The updater is the previous version's code, which has proved
that it runs on this machine, so it can restore the previous version whatever the new
one does; and it runs from a copy, so the installation's files are not in use. It opens
no window. It:

1. Waits up to 60 s for an exclusive lock on `.update/instances.lock`.
   - Every copy started from the folder holds a shared lock for its whole life, so
     this waits until all have exited.
   - If another window stays open, nothing changes, the previous version starts
     again, and the dialog says why.
2. Checks the staged release against its `BUILD-INFO.json` again.
3. Writes `.update/pending.json` with the previous and the new version
   ([Update files](#update-files)).
4. Moves every entry of the installation folder except `.update` into
   `.update/backup/`.
5. Moves every entry of the staged release into the installation folder.
   - Steps 4 and 5 are renames on one volume.
   - A rename that a scanner or a closing process holds up (Windows sharing or lock
     violations, Linux `EBUSY`) is tried again with backoff for 15 s.
   - When one fails for good, everything already moved is moved back and the
     previous version starts again.
6. Removes the used-up staged copy and `ready.json`, so the new version does not offer
   it again.
7. Releases the lock.
8. Starts the new executable with the original arguments in the original working
   folder.
   - The executable is the one the new `BUILD-INFO.json` names
     (`midnight-explorer.exe` or `midnight-explorer` now, a renamed executable later).

The user's settings are not in the installation folder (`%LOCALAPPDATA%\Midnight
Explorer\preferences.json`, `~/.config/midnight-explorer/preferences.json`), so the new
version reads the same settings and restores the same panels and tabs; explicit
`--left`/`--right` arguments are passed again.

## Startup check and rollback

The new version holds the instance lock and finds `pending.json` naming its own
version. Once its main window shows and its event loop runs, it writes
`.update/confirmed`: the new version starts and is usable. Reading the user's settings
does not hold the confirmation back; a read that is refused or slower than the
updater's wait leaves the defaults in place, and the startup check (which follows the
user's choices) waits for the settings for up to two minutes after the window shows
and is skipped in that run if they have not arrived. The 500 ms timer that drives both
stops once the start is confirmed and the startup check ran or was skipped
(`StartupSchedule` in `crates/midnight-explorer/src/updates.rs`). The updater waits up
to 180 s:

- **Confirmed:** it removes the backup and `pending.json`, writes `result.json` and
  exits. The new version's dialog says "Updated to version Y."; a later start does not
  repeat it.
- **The new version exits before confirming** (it crashed, could not create its
  window, or refused to start): the updater moves the new files to `.update/failed/`,
  moves the backup back, removes `pending.json`, writes `result.json` and then starts
  the previous version with the same arguments, which reads it and says "Version Y did
  not start correctly, so the previous version was restored." with the reason. That
  start also removes `failed/`.
- **No answer in time while the new version runs:** the backup is kept, the updater
  marks `pending.json` unattended and exits. Until the new version confirms, each of
  its starts confirms (and says "Updated to version Y."); the first start that finds
  the confirmation removes the backup, `confirmed` and `pending.json`, and says nothing.

`update.log` in `.update/` records every step with Unix timestamps (at most 1 MiB).
A start that finds no `pending.json` removes what earlier updates left (`failed/`,
`updater/`, a backup whose update was confirmed). While `pending.json` names another
version, a start leaves the updater's folders alone.

If the computer loses power during steps 4 and 5 of the swap, the folder can hold a mix
of both versions; the previous version's files are in `.update/backup/` and the new
one's in `.update/staging/`. Moving the backup's entries back restores the previous
version.

## Update files

The swap keeps its state in `<installation>/.update/`, the hand-off of an MSI
installation in the private folder ([Transport](#transport)). All JSON files are
UTF-8, written to a `.tmp` file, synced and renamed into place.

| File | Written by | Content |
| --- | --- | --- |
| `.update/instances.lock` | every running copy of an extracted release folder | Empty lock file: a shared lock while the copy runs, the updater's exclusive lock during the swap. |
| `.update/staging/<release folder>/` | the update thread | The verified new version. |
| `.update/staging/ready.json` | the update thread, after staging | The staged release ([ready.json](#readyjson)). |
| `.update/pending.json` | the updater, before the swap | The update in progress ([pending.json](#pendingjson)). |
| `.update/backup/`, `.update/failed/` | the updater | The previous version's files while the new one proves it starts; the new version's files after a rollback. |
| `.update/confirmed` | the new version | Its own version as plain text, for example `0.6.3`, once its main window shows and its event loop runs. |
| `.update/result.json` | the updater | How the update went ([result.json](#resultjson)). |
| `.update/update.log` | the updater | Every step with Unix timestamps, at most 1 MiB. |
| `<private folder>/instances/<key>.lock` | every running copy of an MSI installation | Lock file of one installation; the key is 16 hexadecimal digits of the SHA-256 of the lowercase installation folder without a trailing separator. |
| `<private folder>/installer/` | the application before the hand-off | The copy of the executable that runs the hand-off; the next start removes it. |
| `<private folder>/installer.log` | `msiexec`, per user only | Windows Installer's verbose log. |
| `<private folder>/installer-result.json` | the hand-off | How the MSI installation went ([installer-result.json](#installer-resultjson)). |

### ready.json

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | string | The staged version, for example `"0.6.3"`. |
| `folder` | string | The release folder's name inside `staging/`, for example `"midnight-explorer-0.6.3-x86_64-pc-windows-msvc"`. |
| `page_url` | string | The release page. |

A start offers the staged update as ready only while that folder exists.

### pending.json

| Field | Type | Meaning |
| --- | --- | --- |
| `from` | string | The previous version. |
| `to` | string | The new version; the new version confirms when this is its own version. |
| `unattended` | boolean, optional | Written as `true` when the updater stopped waiting for the confirmation: the first start that finds the confirmation then finishes the update. |

### result.json

One object whose `outcome` names what happened; the other fields depend on it.

| `outcome` | Other fields | Meaning |
| --- | --- | --- |
| `"updated"` | `from`, `to` | The new version confirmed its start. |
| `"rolled-back"` | `from`, `to`, `reason` | The new version did not pass its startup check; the previous one is back. |
| `"not-applied"` | `reason` | Nothing changed, for example because another window stayed open. |
| `"unconfirmed"` | `from`, `to` | The new version runs but did not confirm in time; the backup is kept. |

### installer-result.json

One object: `folder` (string, the installation folder), `outcome` and `to` (string,
the version the hand-off installed or tried to install), and for a failure `reason`.
Only the copy started from the recorded folder reads it, once, and removes it.

| `outcome` | `msiexec` status | Notice at the next start |
| --- | --- | --- |
| `"installed"` | 0 | "Updated to version Y." |
| `"restart-required"` | 3010, 1641 | "Version Y is installed. Restart Windows to finish the update." |
| `"cancelled"` | 1602 (cancelled), 1223 (UAC declined) | "The update to version Y was cancelled. Nothing changed." |
| `"busy"` | 1618 | "Version Y was not installed because another installation was in progress. Nothing changed; try again when it has finished." |
| `"failed"` | 1603, 1619, 1620, 1625, 1633, 1638, others, or a hand-off that refused to start `msiexec` | "Version Y was not installed." with the `reason` (and the log's path per user) and "Nothing changed." |

<a id="windows-installer-copies"></a>

## MSI installations

The release MSI (`scripts/package-msi.py`, [Release build](../release-build.md#windows-installer-msi))
installs every file of the ZIP except `BUILD-INFO.json`, Just for me
(`%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)`) or For all users
(`C:\Program Files\Midnight Explorer (MSI)`). Windows Installer owns those files, their
registration and the Add/Remove Programs entry, so such a copy is never swapped: a newer
MSI of the same `UpgradeCode` replaces it (`crates/midnight-update/src/installer.rs`).

**Detection** (on the update thread, when no release folder is found):
`installer::detect` asks msi.dll for the products of the MSI's fixed `UpgradeCode`
`{DDF2A2BA-6901-484C-97B2-4EF662BF5E72}` (`MsiEnumRelatedProductsW`) and, for each, its
`InstallLocation` and `VersionString` in the per-user (`MSIINSTALLCONTEXT_USERUNMANAGED`)
and the per-machine context (`MsiGetProductInfoExW`). The copy is an MSI installation
when exactly one of them names the running executable's folder (compared after
resolving both paths, so case and a trailing separator do not matter), the executable is
`midnight-explorer.exe`, no `BUILD-INFO.json` lies beside it and the recorded version is
the running one; the context is its scope. More than one match or another version shows
the reason and the release page. The MSI's `SetInstallLocation` action records
`InstallLocation`, so a copy installed by 0.6.1 is found too; no marker file
is needed, and none in the folder could make a copy count as installed.

**Download:** with an MSI installation the controller fetches and verifies `SHA256SUMS` exactly
as for an archive, then downloads the MSI, whose line the signed list must contain, to
`<private folder>/download/<version>/` and checks its SHA-256 through a handle that
denies writing. A release without the MSI, or whose list does not name it, is offered
with the reason and the release page. Nothing is unpacked; the MSI stays in the private
folder and the dialog offers **Install now**. A verified MSI is not remembered across a
restart: Download and install then finds the downloaded file, verifies the list and the
file again and offers Install now without downloading it again.

**Install now** closes the window the ordinary way. After every worker has stopped, the
application copies its executable to `<private folder>/installer/` and starts the copy
with
`--install-update <msi> <sha256> <user|machine> <installation folder> <version> <private folder> <working folder> -- <original arguments>`,
then exits. The copy runs because Windows Installer replaces the installation's own
files, and opens no window. It:

1. Waits up to 60 s for an exclusive lock on
   `<private folder>/instances/<key>.lock` ([Update files](#update-files)).
   - Every copy of this user's started from that MSI folder holds the lock shared for
     its life; an installation for all users cannot keep the lock in its own folder.
   - A window that stays open ends the hand-off with "Another Midnight Explorer window
     of this installation is still open".
   - Copies of other accounts are left to Windows Installer, which reports a needed
     restart when it must replace files in use.
2. Opens the MSI for reading with read sharing only, and its folder without delete
   sharing.
   - Neither may be a link.
   - No program can then change, rename or delete the MSI or rename its folder.
3. Checks the MSI's SHA-256 against the signed digest through that handle.
   - A changed file is refused before `msiexec` starts.
4. Starts `msiexec.exe` from the system folder with
   `/i "<msi>" /passive /norestart INSTALLDIR="<installation folder>"` and the copy's
   scope:
   - Per user: `ALLUSERS=2 MSIINSTALLPERUSER=1` and `/l*v "<private folder>\installer.log"`,
     without elevation.
   - Per machine: `ALLUSERS=1`, through `ShellExecuteExW` with the `runas` verb, so
     Windows shows its UAC prompt (consent, or an administrator's credentials for a
     standard user); the application never answers it.
   - The elevated `msiexec` writes no log: an elevated write to a path that the user's
     programs can replace with a link could overwrite a system file.
   - `/passive` shows Windows Installer's progress window and asks nothing;
     `/norestart` never restarts Windows.
   - The MSI's major upgrade removes the installed version and installs the new one in
     the same folder; Windows Installer rolls back completely on failure.
5. Waits for `msiexec` to exit.
6. Closes the MSI and its folder.
7. Writes `<private folder>/installer-result.json` with the installation folder and the
   outcome ([installer-result.json](#installer-resultjson)).
8. Removes the MSI after a successful installation.
9. Starts `<installation folder>\midnight-explorer.exe` with the original arguments in
   the original working folder, with the user's rights: the new version, or the
   previous one when nothing was installed.

The started copy takes its instance lock, removes `installer/` and reads the outcome
once (only the copy of the recorded folder does); the dialog's notice is the one that
[installer-result.json](#installer-resultjson) lists for the `msiexec` status.

The MSI runs no custom action that runs code, so the relaunch is the hand-off's, not
the installer's: a relaunch from the MSI would need a code-running custom action, and
the hand-off waits for `msiexec`'s status anyway, to tell the next start what happened. A computer that loses power during `msiexec` is
Windows Installer's to recover; the hand-off's own state is only the result file,
written after `msiexec` exits.

Copies before 0.6.3 contain no hand-off (0.6.2's MSI does not install at all): they
offer the release page, and their users install a newer MSI once by hand in the same
scope. The first update through the hand-off is from an installed 0.6.3 to the release
after it.

## Keys

The owner creates the key pair once, on their own machine, in PowerShell at the
repository root:

1. Set `MIDNIGHT_RELEASE_KEY` to the full path of the secret key file, in a folder
   outside every repository and checkout. The variable only carries the path to the
   commands of this procedure and of
   [Signing the release](../release-build.md#signing-the-release); the script reads no
   environment variable.

   ```powershell
   $env:MIDNIGHT_RELEASE_KEY = Join-Path $HOME 'midnight-release\midnight-explorer-release.key'
   ```

2. Generate the key pair:

   ```powershell
   python -B scripts/release-signing.py generate-key --secret-key $env:MIDNIGHT_RELEASE_KEY
   ```

   Result: the script asks for a password (at least 12 characters), writes the secret
   key, writes the public key into `packaging/update-signing.pub` and prints both
   paths.
3. Commit `packaging/update-signing.pub`. Builds from that commit on trust only
   releases signed with this key.
4. Back up the secret key and its password. Without them, releases can no longer
   update older copies, which would then need a manual download of a version with a
   new key.

`generate-key` refuses a secret-key path inside the repository, an existing secret-key
file and a public key file that already holds a key instead of the placeholder. It
creates the secret key's folder when needed, encrypts the key with scrypt using
libsodium's "sensitive" limits (about 1 GiB of memory for a few seconds) and writes it
with mode 0600 on Linux. The public key replaces the placeholder file, never an
existing key.

The tool needs only Python 3.11 or later: Ed25519 is implemented in the script (RFC 8032,
checked against the RFC's test vectors in `scripts/tests/test_release_signing.py`),
BLAKE2b and scrypt come from `hashlib`. It is not constant-time, so use it only on the
owner's machine. Its files are minisign-compatible, so the official `minisign` tool
(`minisign -S -s KEY -m SHA256SUMS -t "<trusted comment>"`) can sign as well.

Signing a release is in [Release build](../release-build.md#signing-the-release).
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
  when the new version exits before confirming, a late confirmation finished by the
  next start, another running copy, a staged copy
  changed after staging, a staged folder elsewhere, settings outside the installation
  untouched. `apply_windows`: a file held open without delete sharing (as a running
  program's files can be) that is released in time, one that stays held (everything
  moves back), and a running executable that can be renamed.
- `controller`: requests return at once while the transport blocks, the whole check →
  download → verify → stage flow, a tampered archive, another key (no archive
  downloaded), a build without a key, the startup interval and skip, rate limits and
  offline at startup, and a staged update offered as ready after a restart.
- `loopback`: the real system transport (WinHTTP or `ureq`) runs the whole flow against
  a local HTTP server through the debug-only test endpoint, and the GitHub policy
  refuses that server before connecting.
- `installer`: detection through a fake product registry (either scope, a trailing
  separator and case, another product elsewhere, no product, a release folder, another
  executable, another recorded version, both scopes in one folder), the MSI's name and a
  release without it, the `msiexec` parameters per scope and a refused quote, the
  request's arguments, every exit status's outcome and notice, the hand-off's order with
  a fake host (wait → pin and check → `msiexec` → release → record → relaunch; a changed
  MSI or an open window never reaches `msiexec`; a declined prompt, a busy installer and
  a needed restart), the executable copy, the instance locks, and the controller's MSI
  flow (only the MSI downloaded, a changed or unlisted MSI refused, the notice at the
  next start). On Windows the real pin refuses writing, renaming and deleting the MSI
  and renaming its folder until released, and msi.dll is asked about an unused upgrade
  code. Nothing starts `msiexec` or asks for elevation; that `msiexec` can read a
  pinned package was checked by hand with an administrative extraction
  (`msiexec /a`) of a stand-in MSI while pinned.

`scripts/tests/test_release_signing.py` covers the release tool (its key tests point the
tool at a stand-in repository folder of their own, because the tool refuses a secret
key inside the repository and CI's temporary folder lies inside the checkout), the
UI's states are covered by `crates/midnight-ui/tests/update_dialog.rs`, and the
settings schema by `crates/midnight-platform-common/src/settings/updates_tests.rs`.
Evidence: [update check record](../verification/2026-11-23-update-check.md) (it names
schema 39, the number in its export; main stores the choices as schema 40).
