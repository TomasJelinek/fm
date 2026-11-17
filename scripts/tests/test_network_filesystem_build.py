"""Network and native packages obey the same safe library publication contract."""
import test_native_filesystem_build_linux as linux
import test_native_filesystem_build_windows as windows


class NetworkFilesystemBuildLinux(linux.NativeFilesystemBuildLinux):
    provider = "network"


class NetworkFilesystemBuildWindows(windows.NativeFilesystemBuildWindows):
    provider = "network"


if __name__ == "__main__":
    import unittest
    unittest.main()
