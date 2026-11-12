"""Network and native packages obey the same safe library publication contract."""
from scripts.tests import test_native_filesystem_build_linux as linux
from scripts.tests import test_native_filesystem_build_windows as windows


class NetworkFilesystemBuildLinux(linux.NativeFilesystemBuildLinux):
    provider = "network"


class NetworkFilesystemBuildWindows(windows.NativeFilesystemBuildWindows):
    provider = "network"
