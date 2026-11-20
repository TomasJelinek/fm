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


class Hold:
    """An open handle that a timer closes; `release` closes it early, `join` waits for it."""

    def __init__(self, handle, seconds):
        self._handle, self._lock = handle, threading.Lock()
        self._timer = threading.Timer(seconds, self.release)
        self._timer.start()

    def release(self):
        import _winapi
        self._timer.cancel()
        with self._lock:
            if self._handle is not None:
                _winapi.CloseHandle(self._handle)
                self._handle = None

    def join(self):
        self._timer.join()


def hold(path, seconds, share_delete=True):
    """Keep `path` open for `seconds`, as endpoint protection does with a file just written.

    While the handle is open, Windows refuses to rename a folder above the file (access
    denied, 5); without delete sharing it also refuses to rename or delete the file
    itself (sharing violation, 32).
    """
    import _winapi
    share = 7 if share_delete else 3  # FILE_SHARE_READ | FILE_SHARE_WRITE [| FILE_SHARE_DELETE]
    handle = _winapi.CreateFile(str(path), 0x80000000, share, 0, 3, 0, 0)  # GENERIC_READ, OPEN_EXISTING
    return Hold(handle, seconds)


# The same hold from inside a PowerShell process, for scripts that write their own output:
# `Start-FreshHold PATH MILLISECONDS SHARE_DELETE`. A .NET timer closes the handle, so no
# PowerShell code runs on the timer thread and nothing needs compiling; the holds stay
# referenced globally so that a garbage collection cannot close them early.
POWERSHELL_HOLD = r'''
function Start-FreshHold([string]$Path, [int]$Milliseconds, [bool]$ShareDelete) {
    $share = [IO.FileShare]::ReadWrite
    if ($ShareDelete) { $share = $share -bor [IO.FileShare]::Delete }
    $stream = [IO.FileStream]::new($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, $share)
    $release = [Threading.CancellationTokenSource]::new($Milliseconds)
    if ($null -eq $global:MidnightFreshHolds) { $global:MidnightFreshHolds = [Collections.Generic.List[object]]::new() }
    $global:MidnightFreshHolds.Add(@($stream, $release,
        $release.Token.Register([Delegate]::CreateDelegate([Action], $stream, 'Dispose'))))
}
'''
