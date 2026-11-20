"""Directory links, hidden folders and file holds shared by the build script tests."""
import os
from pathlib import Path
import stat
import threading


def directory_link(link, target):
    """A link to a directory: a junction on Windows (no privilege needed), else a symlink."""
    link, target = Path(link), Path(target)
    if os.name == "nt":
        import _winapi
        _winapi.CreateJunction(str(target), str(link))
    else:
        link.symlink_to(target, target_is_directory=True)
    return link


def hidden_directory(path):
    """A new directory hidden like a profile's AppData (an ordinary directory elsewhere)."""
    path = Path(path)
    path.mkdir(parents=True)
    if os.name == "nt":
        import ctypes
        if not ctypes.windll.kernel32.SetFileAttributesW(str(path), stat.FILE_ATTRIBUTE_HIDDEN):
            raise OSError(f"Cannot hide fixture directory: {path}")
    return path


def hold(path, seconds, share_delete=True):
    """Keep `path` open for `seconds`, as endpoint protection does with a file just written.

    While the handle is open, Windows refuses to rename a folder above the file (access
    denied, 5); without delete sharing it also refuses to rename or delete the file
    itself (sharing violation, 32). Returns the timer that closes the handle.
    """
    import _winapi
    share = 7 if share_delete else 3  # FILE_SHARE_READ | FILE_SHARE_WRITE [| FILE_SHARE_DELETE]
    handle = _winapi.CreateFile(str(path), 0x80000000, share, 0, 3, 0, 0)  # GENERIC_READ, OPEN_EXISTING
    timer = threading.Timer(seconds, _winapi.CloseHandle, (handle,))
    timer.start()
    return timer


# The same hold from inside a PowerShell process, for scripts that write their own output.
POWERSHELL_HOLD = r'''
if (-not ('MidnightFreshHold' -as [type])) { Add-Type -TypeDefinition @"
using System.IO;
using System.Threading;
public static class MidnightFreshHold {
    public static void Hold(string path, int milliseconds, bool shareDelete) {
        FileStream stream = new FileStream(path, FileMode.Open, FileAccess.Read,
            shareDelete ? FileShare.ReadWrite | FileShare.Delete : FileShare.ReadWrite);
        Thread release = new Thread(delegate() { Thread.Sleep(milliseconds); stream.Dispose(); });
        release.IsBackground = true;
        release.Start();
    }
}
"@ }
'''
