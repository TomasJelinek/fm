# Troubleshooting

The problems that a first-time user of Midnight Explorer most often meets, each with the
message the application (or the system) shows and what to do. Messages are quoted as
the application writes them; `…` stands for a name or path that it fills in. Terms such
as [panel](glossary.md#panel) and [worker](glossary.md#worker) are defined in the
[glossary](glossary.md). For anything not listed here, see [Get help](#get-help).

## Installing and starting

### Windows: VCRUNTIME140.dll is missing

This applies to 0.6.3 and earlier. From 0.6.4 on, the application and its plugins
include the C runtime they use and need no Visual C++ runtime.

**Symptom:** `midnight-explorer.exe` 0.6.3 or earlier does not start, or a viewer,
thumbnail or filesystem plugin of such a release fails to load, and Windows reports that
`VCRUNTIME140.dll` was not found.

**Cause:** the Microsoft Visual C++ x64 runtime is not installed. Neither the
[release ZIP](glossary.md#release-zip) nor the MSI installer of those releases brings it.

**Fix:** install 0.6.4 or later from the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases). To keep the older
release, install Microsoft's
[supported Visual C++ x64 Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist),
not a single DLL from another site, then start Midnight Explorer again.

### Windows: the 0.6.2 installer stops with error 2228

**Symptom:** `midnight-explorer-0.6.2-x86_64-pc-windows-msvc.msi` stops before its first
screen with Windows Installer error 2228.

**Cause:** that package lacks a table that Windows Installer needs; 0.6.3 fixed it. No
0.6.2 [MSI installation](glossary.md#msi-installation) exists.

**Fix:** use the MSI of 0.6.3 or later from the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases), or the ZIP. A 0.6.1
MSI installation is upgraded in place by the newer MSI's **Just for me** choice.

### Windows: removing one of two MSI installations leaves its files

**Symptom:** Midnight Explorer was installed both **Just for me** and **For all users**.
After one of them is removed in **Installed apps**, its folder
(`%LOCALAPPDATA%\Programs\Midnight Explorer (MSI)` or
`C:\Program Files\Midnight Explorer (MSI)`), its Start menu entry and its registry key
under `Software\Midnight Explorer Installer` are still there.

**Cause:** both installations register the same Windows Installer components, so
Windows Installer keeps the files while the other installation still uses those
components. The MSI supports one kind of installation at a time; 0.6.3 and earlier
still installed the second kind beside an earlier version, and from 0.6.4 the installer
refuses it ([next entry](#windows-the-installer-says-midnight-explorer-is-already-installed)).

**Fix:** remove the other installation too, then delete what is left: the folder, the
Start menu entry (`Midnight Explorer` in your or the all-users Start menu) and the
registry key. For a Just for me copy, in PowerShell:
`reg delete "HKCU\Software\Midnight Explorer Installer" /f`; for an all-users copy, in a
PowerShell started with **Run as administrator**:
`reg delete "HKLM\SOFTWARE\Midnight Explorer Installer" /f /reg:64`. The 0.6.4 installer
counts a leftover key as an installed copy until it is deleted. Then install again with
one choice only. Your settings in `%LOCALAPPDATA%\Midnight Explorer` are not affected.

### Windows: the installer says Midnight Explorer is already installed

**Symptom:** the MSI of 0.6.4 or later says "Midnight Explorer is already installed for
all users. Remove it in Settings > Apps > Installed apps before installing it just for
you." (or the same with just for you and for all users exchanged), and the Install
button of that choice is disabled. A silent installation ends with exit code 1603 and
that message in its log; an update from the update dialog stops with it.

**Cause:** the other kind of installation is installed. The MSI supports one kind at a
time, because two copies would share Windows Installer's records of their files.

**Fix:** to update the installed copy, choose its kind instead. To switch, remove the
installed copy in **Settings › Apps › Installed apps** first, then install; your settings
stay. If **Installed apps** lists only one Midnight Explorer and installing that same
kind is refused, the copy that the message names was removed by 0.6.3 or earlier while
both kinds were installed, and its files and registry key were left: delete them as in
[the previous entry](#windows-removing-one-of-two-msi-installations-leaves-its-files).

### Windows: SmartScreen warns before the installer or the program runs

**Cause:** the executable and the MSI are not code-signed yet.

**Fix:** check the download against the release's signed checksums as described in
[Check a download by hand](#check-a-download-by-hand), then let it run.

### Linux: the program does not start because a library is missing

**Symptom:** started from a terminal, `./midnight-explorer` stops with the dynamic
loader's message `error while loading shared libraries: … cannot open shared object
file: No such file or directory`.

**Fix:** install the runtime packages that
[system requirements](system-requirements.md#runtime-software) list for your
distribution. In a source checkout, `sh scripts/setup-linux.sh --install` installs them.

### The panels show no files

**Symptom:** the panels stay empty or report `Native filesystem plugin is unavailable`.

**Cause:** the application browses only through its native
[filesystem plugin](glossary.md#filesystem-plugin), which is missing beside the
executable: the executable was copied without its folders, or a raw
`cargo build` built only the host.

**Fix:** keep the `viewers`, `filesystems` and `thumbnailers` folders beside the
executable; extract the complete archive again. For a source build, use `build.bat` or
`./build.sh`, which build every module ([Build and run](user-guide.md#build-and-run)).

### The window stays blank, or startup fails in the renderer

**Symptom:** the window does not draw, draws wrongly, or a terminal shows
`Midnight Explorer: Cannot initialize renderer: …`.

**Cause:** the default renderer needs a working GPU driver: Vulkan or Direct3D 12 on
Windows, Vulkan on Linux. When it fails before the first window is shown, the application retries once
with the software renderer, which draws on the CPU, by itself; an explicit
`SLINT_BACKEND` turns that retry off.

**Fix:** try these one at a time; the
[environment variables](system-requirements.md#environment-variables) table describes
each.

1. Start with the software renderer: `midnight-explorer --software`.
2. Use one GPU device per window: set `MIDNIGHT_SHARED_WGPU=0`.
3. Choose another graphics interface: set `WGPU_BACKEND=dx12` (Windows) or
   `WGPU_BACKEND=gl` (Linux).
4. Prefer the discrete GPU on a laptop: set `WGPU_POWER_PREF=high`.

In PowerShell, set a variable for one start like this:

```powershell
$env:MIDNIGHT_SHARED_WGPU = '0'; .\midnight-explorer.exe
```

On Linux: `MIDNIGHT_SHARED_WGPU=0 ./midnight-explorer`. Update the graphics driver if
only `--software` works.

## Folders and files

### Cannot open folder: access denied

**Symptom:** the **Cannot open folder** message says `Access to this folder was
denied.`

**Fix when the message offers Retry with authorization:** the message also says
`Your operating system will ask for permission to read this folder.` Choose **Retry
with authorization** and accept the UAC prompt (Windows) or the polkit password prompt
(Linux). The [elevated listing helper](glossary.md#elevated-listing-helper) lists that
folder and, while the panel stays inside it, its subfolders without asking again for up
to 30 minutes (5 minutes after the last listing); it opens no file and changes nothing.
Files in the folder still cannot be opened, viewed or copied: only the listing is
authorized.

**When the message says** `Access to this folder was denied. This folder is outside the
authorized folder …: it is reached through a link or mount point that leads elsewhere.`:
the folder's name lies inside a folder you authorized, but it is a link, junction or
mount point to another place, which that authorization does not cover. **Retry with
authorization** asks for that folder on its own.

**When the retry is not offered,** the message says why after
`Opening it needs administrator permission, which this installation cannot ask for:`
(on Linux, `root permission`). The reasons, and what to do:

| The message continues with | What to do |
| --- | --- |
| `its authorization helper (midnight-elevated-listing.exe) is missing. Reinstall Midnight Explorer to restore it.` (on Linux the name has no `.exe`) | Reinstall, or extract the complete release again. A private installation that `build.bat` deployed has no helper; install a build as [Build and run](user-guide.md#build-and-run) describes. |
| `its authorization helper (midnight-elevated-listing.exe) is not the one this version shipped with. Reinstall Midnight Explorer to restore it.` | Reinstall; something replaced the helper. |
| `other accounts on this computer can change its installation folder. Install Midnight Explorer where only you and administrators can change it.` | Move the installation into your profile (**Move to my profile…**, see [Shared installation notice](#shared-installation-notice)), or use the MSI. On Linux the text ends `only you and root`. |
| `Midnight Explorer must be installed where only administrators can change it, such as C:\Program Files (the installer's For all users choice does this).` | A build without the helper's digest (a development build) needs such a folder. On Linux: `only root can change it, such as /opt/midnight-explorer installed with sudo`. |

On Linux the message can instead say `Opening it needs root permission, which this
system cannot ask for: the polkit program pkexec is missing or not owned by root.`:
install the distribution's polkit package. On Linux the retry also needs a desktop
polkit authentication agent, which is known only when the retry asks.

**Details** in the message names the refused file or folder; **Copy details** copies
the whole report. Background: [directory authorization helper](../SECURITY.md#directory-authorization-helper).

### The panel goes back to the previous folder after Retry with authorization

**Symptom (0.6.2 and earlier):** after **Retry with authorization** the folder's files
appeared, and about a second later the panel showed the folder you came from.

**Cause:** the panel's change watch checked the folder with your own rights. The denied
check counted as a change, and the automatic re-read that followed was denied too and
went back to the parent folder.

**Fix:** use 0.6.3 or later. A folder listed with authorization stays as listed, and the
status row says that it was listed with authorization and is not updated automatically.
Refresh (Ctrl+R) lists it again without leaving it: through the same authorization while
it lasts, otherwise with your own rights first, and while access is still denied the
files stay and **Cannot open folder** offers **Retry with authorization** again.

### Every subfolder of an authorized folder asks for authorization again

**Symptom (0.6.3 and earlier):** after **Retry with authorization** listed a folder,
entering each of its subfolders that your account may not read showed **Cannot open
folder** and asked for authorization again.

**Fix:** use a later version. One consent covers the folder and its subfolders while the
panel stays inside it, for up to 30 minutes (5 minutes after the last listing), see
[Build and run](user-guide.md#build-and-run). When a subfolder asks again within that
time, the panel left the authorized folder in between, the authorization expired, or the
subfolder is a link or mount point that leads elsewhere (the message then says so).

### Delete or F8 asks to delete permanently instead of moving to the Trash

**Symptom:** Delete or F8 opens the permanent-deletion confirmation, the same one as
Shift+Delete, instead of moving the items to the Trash (on Windows, the Recycle Bin).

**Cause:** no Trash takes the selected items. The line above `Permanent deletion cannot
be undone.` says why:

| The line starts with | Meaning |
| --- | --- |
| `The Recycle Bin is turned off for C:` | That drive's Recycle Bin is set to "Don't move files to the Recycle Bin. Remove files immediately when deleted." To recycle again, open the Recycle Bin's **Properties**, select the drive and choose **Custom size**. |
| `A policy turns off the Recycle Bin` | The Group Policy "Do not move deleted files to the Recycle Bin" (User Configuration › Administrative Templates › Windows Components › File Explorer) is set; ask your administrator. |
| `There is no Recycle Bin on E:` (or on `\\server\share`) | Network shares, mapped network drives, USB sticks, memory cards, CD/DVD and RAM drives have no Recycle Bin on Windows. |
| `There is no Trash on /run/user/1000` (Linux) | Midnight Explorer uses your home Trash (`~/.local/share/Trash`, or `$XDG_DATA_HOME/Trash`) only for items on the same filesystem; items on tmpfs, a USB stick or another partition have none. |
| `There is no Trash, because the home folder is unknown` (Linux) | `HOME` is not set. |

**Fix:** choose **Cancel** (or press Escape) to keep everything, or confirm to delete
permanently. If you selected items from several places (a tree, Find results) and only
some cannot go to the Trash, confirming deletes all of them permanently: cancel and
delete those separately.

### Delete stops with "The Trash did not take this item, and it was kept"

**Cause:** the Trash refused an item after the Delete started: the Recycle Bin setting
was changed after F8, or the item is too large or its path too long for the Recycle Bin.

**Fix:** the item is kept. Choose **Retry** once the Trash takes it, **Skip** to keep it,
or delete it with Shift+Delete. Midnight Explorer never deletes an item permanently
because the Trash refused it.

### Linux: Copy to a USB stick or network share fails

**Symptom:** Copy stops with `This filesystem cannot create anonymous O_TMPFILE copies;
verified Copy on Linux needs a local filesystem such as ext4, XFS, Btrfs or tmpfs.`

**Cause:** verified Copy writes each file through an anonymous temporary file, which
FAT, exFAT and NTFS media, NFS and SMB shares, sshfs and GVfs mounts cannot create.
Copying from them works.

**Fix:** copy to a local filesystem instead, or use another tool for such a
destination. See [Linux distributions and filesystems](system-requirements.md#linux-distributions-and-filesystems).

### Linux: Rename or Delete on an NFS share or sshfs mount fails

**Symptom:** Rename, Delete or the Trash stops with `This filesystem cannot rename an
entry without replacing another (renameat2 RENAME_NOREPLACE; NFS and sshfs, for
example), which Rename, Delete and Trash need to keep the selected item safe`.

**Cause:** these operations first move the item aside with a rename that never replaces
another entry. The Linux NFS client, sshfs and some other FUSE file systems (ntfs-3g)
cannot do that rename. Before 0.6.4 the message was only `Invalid argument (os error 22)`.

**Fix:** rename or delete such items with another tool, or on the server. Reading,
viewing, Find, folder sizes and copying from the share work. See
[file systems](user-guide.md#file-systems).

### Windows: Move from a USB stick, memory card or network share is refused

**Symptom:** a Move stops with `This operation requires stable file identities, which
this filesystem does not provide.`

**Cause:** a Move checks the identity of each source across its own rename, and FAT32,
exFAT, CD and DVD drives and some network servers give files no stable ID. A Move to such
a drive works.

**Fix:** copy the items (Copy reads them without IDs), then delete the originals. See
[file systems](user-guide.md#file-systems).

### An archive does not open as a folder

**Symptom:** Enter on an archive says `Compatible archive filesystem plugin is
unavailable`.

**Fix:** keep the `filesystems` folder, with its archive worker, beside the executable.
When the file opens in another application instead, its extension's archive
association is off: turn it on in **Options › Archives**
([associations](archive-formats.md#associations)).

### "Two archives are already open"

**Cause:** panels, tabs and viewers keep at most two archives opened from files on disk
open at once.

**Fix:** choose one of the offered **Close …** buttons; the tabs that show that archive
go to its folder, and the new archive opens. See
[archive operation limits](archive-formats.md#current-operation-limits).

## Plugins and viewers

### Shared installation notice

**Symptom:** a notice above the panels says `Other accounts on this PC can change the
files in …. On a shared PC, secure the folder or copy Midnight Explorer into your
profile.` (On Linux: `this computer`.)

**Cause:** the installation is in a folder that other accounts can change, such as a
folder created directly below `C:\`. Plugins still load, but anyone who can change those
files could replace Midnight Explorer itself.

**Fix:** choose **Secure this folder** (for a folder you own; no administrator rights
needed) or **Move to my profile…**, which copies and verifies the installation and
offers **Restart from the copy**. **Dismiss** hides the notice for that folder. See
[shared installations](user-guide.md#shared-installations).

### A plugin is refused

**Symptom:** a viewer, thumbnailer or filesystem is missing, and **Options › Plugins**
lists its package as unavailable, for example with
`Plugin path is owned by another account: …` or (Linux)
`Plugin path is redirected or owned by another account: …`.

**Cause:** the plugin loader refuses a package in which a file or folder belongs to
another account, or a link leads out of the installation.

**Fix:** extract or install the complete release again into a folder of your own, or
install it as an administrator into a protected folder. **Rescan all plugins** reads the
catalogs again ([viewer file types](viewer-associations.md#archives-and-rescanning)).

### Windows: video previews or playback fail with "access denied"

**Symptom:** the thumbnail notice or the video viewer says a helper `could not load its
libraries (access denied)`.

**Cause:** the video helpers run in an [AppContainer](glossary.md#appcontainer) that
must be allowed to read their libraries. The application grants that itself only in a
folder that no other account can change.

**Fix:** **Secure this folder** or **Move to my profile…** in the
[shared installation notice](#shared-installation-notice); the next preview or video
works without a restart.

### Linux: the HTML viewer shows a message instead of the page

| The viewer says | What to do |
| --- | --- |
| `The HTML viewer needs permission for Chromium's sandbox on this system (Ubuntu restricts unprivileged user namespaces).` | Choose **Install…** in the notice and authorize it; it installs the viewer's AppArmor profile and opens the page again. |
| `The HTML viewer cannot use Chromium's sandbox on this system: it denies user namespaces to programs without administrator rights.` | An administrator changes the kernel setting that the message names or, for an installation that only root can change, makes its `chrome-sandbox` setuid root; an AppArmor profile does not help. |
| `The HTML viewer needs system libraries that are not installed: ….` | Run the install command that the message gives. |

Chromium never runs without its sandbox. Details:
[HTML viewer sandbox on Linux](system-requirements.md#html-viewer-sandbox-on-linux).

### Linux: dragging files to or from other applications does nothing

**Cause:** in a native Wayland session, drag and drop with other applications is off;
the panels still drag between each other. Under X11 and XWayland it works.

**Fix:** start Midnight Explorer with `MIDNIGHT_WAYLAND_DND=1` to turn on the native
Wayland transport, which is not yet qualified on a compositor, or use an X11 session.

## Updates

### The update check fails

**Cause:** the check needs HTTPS access to `api.github.com`, and downloads to
`github.com` and GitHub's download servers. A proxy or firewall that blocks them makes
only the check fail.

**Fix:** allow that access, or download the new version from the
[GitHub Releases page](https://github.com/TomasJelinek/fm/releases).

### The update dialog refuses the download

The updater installs only a download whose checksum file carries the project's
signature and whose checksum matches ([SHA256SUMS](glossary.md#sha256sums)). It refuses
the update and installs nothing when it says, for example:

- `The release's checksum list does not match its signature.`
- `The release is signed with a key this version does not trust.`
- `… does not match its signed checksum: the download is damaged or was changed.`
- `This release is not signed, so it cannot be installed automatically.`

**Fix:** choose **Check now** and download again later; if the message stays, download
the release from its page and [check it by hand](#check-a-download-by-hand).

### The update dialog offers only the release page

| The dialog says | Why |
| --- | --- |
| `This copy is installed in a folder you cannot change (…). Download the new version from the release page, or ask an administrator to replace it.` | An administrator extracted this copy, for example below `C:\Program Files` or `/opt`. |
| `This copy is not an extracted release folder, so it cannot update itself.` | A development build or a copy from another package manager. |
| `This build has no update signing key, so it cannot install updates. Download new versions from the release page.` | A build made without the project's release key. |

An MSI installation of 0.6.1 also offers only the release page: run the newer MSI once
by hand with the same choice as before. From 0.6.3 on, MSI installations update
themselves ([Updates](user-guide.md#updates)).

### Check a download by hand

1. Download `SHA256SUMS` and `SHA256SUMS.minisig` from the same release as the file.
2. Compute the file's SHA-256 and compare it with its line in `SHA256SUMS`. In
   PowerShell: `Get-FileHash .\midnight-explorer-0.6.3-x86_64-pc-windows-msvc.msi`. On
   Linux: `sha256sum --check --ignore-missing SHA256SUMS`.
3. Check the signature with [minisign](https://jedisct1.github.io/minisign/) and the
   project's public key
   [`update-signing.pub`](../packaging/update-signing.pub):
   `minisign -Vm SHA256SUMS -p update-signing.pub`.

## Get help

Search the [issues](https://github.com/TomasJelinek/fm/issues) of the GitHub repository,
and open a new issue when yours is not there. Include:

- the version (the window title and **About** show it) and how you installed it (ZIP,
  MSI, tar.gz or a source build);
- the operating system, its version and, on Linux, the desktop and whether it runs X11
  or Wayland;
- the steps that lead to the problem, and the message text (**Copy details** copies a
  message's report).

For a suspected security problem, read [SECURITY.md](../SECURITY.md) first and do not
publish exploit details in an issue.
