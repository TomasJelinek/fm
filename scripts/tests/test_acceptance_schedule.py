"""Set MIDNIGHT_ACCEPTANCE_TEST_BINARY to a built acceptance executable to run these tests."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
BINARY = os.environ.get("MIDNIGHT_ACCEPTANCE_TEST_BINARY")
EXCLUSIVE = "files_links_volumes"
SAMPLE = ("create_rename", "find_by_mask", "about_box", EXCLUSIVE)


@unittest.skipUnless(BINARY, "set MIDNIGHT_ACCEPTANCE_TEST_BINARY to the built harness")
class AcceptanceSchedule(unittest.TestCase):
    def invoke(self, *args, jobs=None, code=0, environment=None):
        # A user's filter, shared-volume fixture or staged-child marker must not
        # change which scenarios this private run exercises.
        env = {k: v for k, v in os.environ.items() if not k.startswith("MIDNIGHT_ACCEPTANCE_")}
        if jobs is not None:
            env["MIDNIGHT_ACCEPTANCE_JOBS"] = jobs
        if environment:
            env.update(environment)
        result = subprocess.run(
            [BINARY, *args], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", timeout=180,
        )
        self.assertEqual(result.returncode, code, result.stdout)
        return result.stdout

    def test_classification(self):
        rows = [line.split(maxsplit=3) for line in self.invoke("--list").splitlines()]
        self.assertTrue(all(len(row) == 4 for row in rows))
        policy = {name: isolation for name, _, isolation, _ in rows}
        self.assertEqual(len(policy), len(rows))
        self.assertEqual(set(policy.values()), {"parallel", "exclusive"})
        self.assertEqual(policy[EXCLUSIVE], "exclusive")
        self.assertEqual(policy["viewers_video"], "exclusive")
        self.assertEqual(policy["panels_properties_several"],
                         "exclusive" if os.name == "nt" else "parallel")
        needs_desktop = (
            sys.platform.startswith("linux") and (os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"))
            and not all(shutil.which(tool) for tool in ("Xvfb", "xvfb-run", "xauth"))
        )
        self.assertEqual(policy["viewers_scroll"], "exclusive" if needs_desktop else "parallel")
        for name in ("viewers_files", "viewers_office", "viewers_combinations"):
            self.assertEqual(policy[name], "exclusive" if os.name == "nt" or needs_desktop else "parallel")
        for name in (*SAMPLE[:-1], "files_clipboard_drag", "ui_polish_dialogs"):
            self.assertEqual(policy[name], "parallel", name)

    @unittest.skipUnless(sys.platform.startswith("linux"), "Linux X11 and Wayland policy")
    def test_plugins_without_an_isolated_x_server(self):
        # Listing reads the policy without connecting to a desktop. Hide Xvfb
        # and check both a headless host and each supported desktop variable.
        for display, wayland, expected in (("", "", "parallel"), (":1", "", "exclusive"),
                                           ("", "wayland-0", "exclusive")):
            with self.subTest(display=display, wayland=wayland):
                listing = self.invoke(
                    "--list", "viewers_scroll", "ui_polish_dialogs",
                    environment={"PATH": "/dev/null", "DISPLAY": display, "WAYLAND_DISPLAY": wayland},
                )
                policy = {name: isolation for name, _, isolation, _ in
                          (line.split(maxsplit=3) for line in listing.splitlines())}
                self.assertEqual(policy, {"viewers_scroll": expected, "ui_polish_dialogs": "parallel"})

    def test_jobs_validation_and_cli_precedence(self):
        for invalid in ("0", "5", "two"):
            with self.subTest(invalid=invalid):
                self.assertIn("jobs", self.invoke("--list", "--jobs", invalid, code=2))
        self.assertIn("jobs", self.invoke("--list", jobs="invalid", code=2))
        self.assertIn("find_by_mask", self.invoke("--list", "find_by_mask", "--jobs", "2",
                                                 jobs="invalid"))

    def test_schedule_and_merged_artifacts(self):
        scratch = ROOT / "_build"
        scratch.mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory(prefix="schedule-", dir=scratch) as private:
            root = Path(private)
            self.invoke("--scale", "smoke", "--run-dir", str(root), "--jobs", "2",
                        *reversed(SAMPLE), jobs="1")
            reports = list(root.glob("*/summary.json"))
            self.assertEqual(len(reports), 1)
            run = reports[0].parent
            rows = json.loads(reports[0].read_text(encoding="utf-8"))["scenarios"]
            self.assertEqual([row["name"] for row in rows], list(SAMPLE))
            safe, exclusive = rows[:-1], rows[-1]
            self.assertEqual((exclusive["isolation"], exclusive["status"]), ("exclusive", "SKIP"))
            self.assertTrue(all(row["isolation"] == "parallel" and row["status"] == "PASS"
                                for row in safe), safe)

            # The JSON rounds to milliseconds. Ignore a millisecond at either end
            # of each interval so a shared, rounded boundary cannot count as overlap.
            intervals = [(r["start_seconds"] + .001, r["start_seconds"] + r["seconds"] - .001)
                         for r in safe]
            self.assertTrue(all(start < end for start, end in intervals))
            peak = max(sum(start <= t < end for start, end in intervals) for t, _ in intervals)
            self.assertEqual(peak, 2, intervals)
            at = exclusive["start_seconds"]
            until = at + exclusive["seconds"]
            for start, end in intervals:
                self.assertTrue(until <= start or at >= end, (start, end, at, until))

            html = (run / "index.html").read_text(encoding="utf-8")
            for row in safe:
                shots = list((run / "shots" / row["name"]).glob("*.png"))
                self.assertGreater(row["screenshots"], 0, row["name"])
                self.assertEqual(len(shots), row["screenshots"], row["name"])
                for shot in shots:
                    with shot.open("rb") as image:
                        self.assertEqual(image.read(8), b"\x89PNG\r\n\x1a\n")
                    self.assertIn(f'src="{shot.relative_to(run).as_posix()}"', html)


if __name__ == "__main__":
    unittest.main()
