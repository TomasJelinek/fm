"""Per-user desktop registration for an application launched from its build tree."""
import configparser
import errno
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
APP_ID = "io.github.TomasJelinek.MidnightExplorer"


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux desktop integration")
class LinuxDesktopInstallTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="desktop registration ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name)
        self.checkout = self.root / "checkout"
        self.script = self.checkout / "scripts/install-linux-desktop.py"
        self.script.parent.mkdir(parents=True)
        source = ROOT / "scripts/install-linux-desktop.py"
        if source.exists():
            shutil.copyfile(source, self.script)
        for relative in ("assets/branding/midnight-explorer.png",
                         f"packaging/linux/{APP_ID}.desktop"):
            destination = self.checkout / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, destination)
        self.home = self.root / "home"
        self.data = self.root / "xdg data"
        self.desktop = self.data / f"applications/{APP_ID}.desktop"
        self.icon = self.data / f"icons/hicolor/256x256/apps/{APP_ID}.png"
        self.environment = dict(os.environ, HOME=str(self.home), XDG_DATA_HOME=str(self.data),
                                DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent", DISPLAY="",
                                WAYLAND_DISPLAY="",
                                DESKTOP_INSTALL_LOG=str(self.root / "argv.json"))

    def executable(self, relative="_build/debug/me"):
        executable = self.checkout / relative
        executable.parent.mkdir(parents=True, exist_ok=True)
        executable.write_text(
            '#!/usr/bin/python3\nimport json, os, pathlib, sys\n'
            'pathlib.Path(os.environ["DESKTOP_INSTALL_LOG"]).write_text('
            'json.dumps(sys.argv), encoding="utf-8")\n', encoding="utf-8")
        executable.chmod(0o755)
        return executable

    def run_installer(self, *arguments):
        return subprocess.run([sys.executable, "-B", str(self.script), *map(str, arguments)],
                              cwd=self.root, env=self.environment, text=True,
                              capture_output=True, timeout=10)

    def test_registers_existing_debug_without_moving_application(self):
        executable = self.executable()
        self.assertFalse(self.desktop.exists(), "build-tree launch has no registered application")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.desktop.is_file(), "GNOME needs an installed desktop entry")
        self.assertEqual(self.icon.read_bytes(),
                         (ROOT / "assets/branding/midnight-explorer.png").read_bytes())
        self.assertTrue(executable.is_file())
        self.assertFalse((self.home / ".local/bin/me").exists())
        entry = configparser.ConfigParser(interpolation=None)
        entry.read(self.desktop, encoding="utf-8")
        self.assertEqual(entry["Desktop Entry"]["StartupWMClass"], APP_ID)
        self.assertEqual(entry["Desktop Entry"]["Icon"], APP_ID)
        self.assertIn(str(executable), entry["Desktop Entry"]["Exec"])

    @unittest.skipUnless(shutil.which("gio"), "Gio desktop launcher unavailable")
    def test_real_gio_launcher_preserves_unicode_and_reserved_characters(self):
        for name in ("me '$`\"\\&;", "me '%f%u $`\"\\&;"):
            with self.subTest(name=name):
                executable = self.executable(f"build Žluťoučký 漢字/{name}")
                result = self.run_installer("--executable", executable.relative_to(self.root))
                self.assertEqual(result.returncode, 0, result.stderr)
                launched = subprocess.run(["gio", "launch", str(self.desktop)],
                                          env=self.environment, text=True, capture_output=True,
                                          timeout=10)
                self.assertEqual(launched.returncode, 0, launched.stderr)
                log = Path(self.environment["DESKTOP_INSTALL_LOG"])
                deadline = time.monotonic() + 5
                while not log.exists() and time.monotonic() < deadline:
                    time.sleep(0.01)
                self.assertTrue(log.exists(), launched.stderr)
                self.assertEqual(json.loads(log.read_text(encoding="utf-8")), [str(executable)])
                log.unlink()

    @unittest.skipUnless(shutil.which("desktop-file-validate"), "desktop-file-validate unavailable")
    def test_installed_entry_passes_desktop_entry_validator(self):
        self.executable()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        validated = subprocess.run(["desktop-file-validate", str(self.desktop)],
                                   text=True, capture_output=True, timeout=10)
        self.assertEqual(validated.returncode, 0, validated.stdout + validated.stderr)

    def test_absolute_xdg_home_does_not_require_home_environment(self):
        self.executable()
        self.environment.pop("HOME")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.desktop.exists())

    def test_invalid_or_empty_xdg_home_falls_back_to_absolute_home(self):
        self.executable()
        for value in (None, "", "relative data"):
            with self.subTest(xdg_data_home=value):
                if value is None:
                    self.environment.pop("XDG_DATA_HOME", None)
                else:
                    self.environment["XDG_DATA_HOME"] = value
                result = self.run_installer()
                self.assertEqual(result.returncode, 0, result.stderr)
                desktop = self.home / f".local/share/applications/{APP_ID}.desktop"
                self.assertTrue(desktop.exists())
                self.assertFalse((self.root / "relative data").exists())

    def test_unusable_home_without_absolute_xdg_home_makes_no_writes(self):
        self.executable()
        self.environment["XDG_DATA_HOME"] = "relative data"
        self.environment["HOME"] = "relative home"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("absolute", result.stderr)
        self.assertFalse((self.root / "relative home").exists())

    def test_missing_or_nonexecutable_file_makes_no_registration(self):
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.data.exists())
        executable = self.executable()
        executable.chmod(0o644)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("executable", result.stderr)
        self.assertFalse(self.data.exists())

    def test_spec_forbidden_equal_sign_in_executable_is_reported(self):
        executable = self.executable("name=unsupported/me")
        result = self.run_installer("--executable", executable)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("=", result.stderr)
        self.assertFalse(self.data.exists())

    def test_repeated_registration_can_select_another_existing_build(self):
        self.executable()
        first = self.run_installer()
        self.assertEqual(first.returncode, 0, first.stderr)
        default_entry = self.desktop.read_bytes()
        second = self.run_installer()
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(self.desktop.read_bytes(), default_entry)
        alternate = self.executable("_build/custom/debug/me")
        third = self.run_installer("--executable", alternate)
        self.assertEqual(third.returncode, 0, third.stderr)
        self.assertIn(str(alternate), self.desktop.read_text(encoding="utf-8"))
        self.assertFalse(list(self.data.rglob("*.tmp")))

    def test_successful_replacement_and_repeat_invalidate_existing_theme_cache(self):
        self.executable()
        first = self.run_installer()
        self.assertEqual(first.returncode, 0, first.stderr)
        hicolor = self.data / "icons/hicolor"
        old_time = 1_600_000_000
        for previous_icon in (b"older icon", self.icon.read_bytes()):
            with self.subTest(replacing_icon=previous_icon == b"older icon"):
                self.icon.write_bytes(previous_icon)
                os.utime(hicolor, (old_time, old_time))
                result = self.run_installer()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertGreater(hicolor.stat().st_mtime, old_time)

    def test_failed_preflight_does_not_invalidate_existing_theme_cache(self):
        self.executable()
        self.icon.parent.mkdir(parents=True)
        self.desktop.parent.mkdir(parents=True)
        self.desktop.write_text("unrelated", encoding="utf-8")
        hicolor = self.data / "icons/hicolor"
        old_time = 1_600_000_000
        os.utime(hicolor, (old_time, old_time))
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(hicolor.stat().st_mtime, old_time)

    def test_partial_publication_preserves_entry_and_does_not_invalidate_theme(self):
        self.executable()
        first = self.run_installer()
        self.assertEqual(first.returncode, 0, first.stderr)
        original_entry = self.desktop.read_bytes()
        self.icon.write_bytes(b"older icon")
        hicolor = self.data / "icons/hicolor"
        old_time = 1_600_000_000
        os.utime(hicolor, (old_time, old_time))
        spec = importlib.util.spec_from_file_location("desktop_installer", self.script)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        replace = os.replace

        def fail_desktop_publication(source, destination, **arguments):
            if destination == f"{APP_ID}.desktop":
                raise OSError(errno.EACCES, "injected desktop publication failure")
            return replace(source, destination, **arguments)

        alternate = self.executable("_build/custom/debug/me")
        with mock.patch.dict(os.environ, self.environment, clear=True):
            with mock.patch.object(module.os, "replace", side_effect=fail_desktop_publication):
                with self.assertRaisesRegex(OSError, "injected desktop publication failure"):
                    module.install(alternate)
        self.assertEqual(self.desktop.read_bytes(), original_entry)
        self.assertEqual(self.icon.read_bytes(),
                         (ROOT / "assets/branding/midnight-explorer.png").read_bytes())
        self.assertEqual(hicolor.stat().st_mtime, old_time)
        self.assertFalse(list(self.data.rglob("*.tmp")))

    def test_unrelated_desktop_entry_is_not_replaced(self):
        self.executable()
        self.desktop.parent.mkdir(parents=True)
        original = b"[Desktop Entry]\nName=Unrelated\nExec=unrelated\nType=Application\n"
        self.desktop.write_bytes(original)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unrelated", result.stderr.lower())
        self.assertEqual(self.desktop.read_bytes(), original)
        self.assertFalse(self.icon.exists())

    def test_symlinked_desktop_file_or_data_directory_is_not_followed(self):
        self.executable()
        outside = self.root / "outside"
        outside.mkdir()
        original = outside / "other.desktop"
        original.write_text("unrelated", encoding="utf-8")
        self.desktop.parent.mkdir(parents=True)
        self.desktop.symlink_to(original)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(original.read_text(encoding="utf-8"), "unrelated")
        self.assertTrue(self.desktop.is_symlink())
        self.desktop.unlink()
        self.desktop.parent.rmdir()
        self.desktop.parent.symlink_to(outside, target_is_directory=True)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(list(outside.iterdir()), [original])

    def test_symlinked_icon_is_rejected_before_entry_changes(self):
        self.executable()
        self.icon.parent.mkdir(parents=True)
        outside = self.root / "other.png"
        outside.write_bytes(b"unrelated")
        self.icon.symlink_to(outside)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(outside.read_bytes(), b"unrelated")
        self.assertFalse(self.desktop.exists())

    def test_fifo_desktop_file_is_rejected_without_waiting_for_a_writer(self):
        self.executable()
        self.desktop.parent.mkdir(parents=True)
        os.mkfifo(self.desktop)
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("non-regular", result.stderr)
        self.assertFalse(self.icon.exists())


if __name__ == "__main__":
    unittest.main()
