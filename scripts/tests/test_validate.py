"""Full local validation (scripts/validate.*): its plan, arguments, skips and results.

The plan tests read both scripts on any host. The run tests copy one script into a
fixture checkout whose cargo, rustc and helper scripts are stubs, so they check the
step runner (exit status, summary, dependencies, prerequisites and the private test
installation), not the real Cargo builds.
"""
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
VIEWERS = ("basic", "source", "markdown")


def bash_steps(text):
    steps = []
    for identifier, job, title in re.findall(
            r"^\s*add_step (\S+) ('[^']*'|\S+) ('[^']*'|\"[^\"]*\")", text, re.MULTILINE):
        steps.append((identifier.strip('"'), job.strip("'"), title[1:-1]))
    return expand(steps, "$build_profile")


def powershell_steps(text):
    steps = [(identifier, job, title) for _, identifier, job, _, title in re.findall(
        r"Add-Step @\{ Id = (['\"])(.+?)\1; Job = '([^']*)'; Title = (['\"])(.+?)\4", text)]
    return expand(steps, "$profileName")


def expand(steps, profile):
    result = []
    for identifier, job, title in steps:
        names = VIEWERS if "$name" in identifier else (None,)
        for name in names:
            result.append((identifier.replace("$name", name or ""), job,
                           title.replace("$name", name or "").replace(profile, "<profile>")))
    return result


# Both scripts stop with usage (exit 2) before any tool runs: no step id after --only,
# a value given to a flag, unknown steps, profiles or options and a missing SDK file.
INVALID_ARGUMENTS = (("--bogus",), ("debug", "release"), ("--only",), ("--only", "no-such-step"),
                     ("--only", ","), ("--only=",), ("--only=,,",), ("--strict=no",), ("--dry-run=",),
                     ("--offline=yes",), ("--help=1",), ("--plugin-sdk", "missing.toml"))
SELECTION = r"ci-workspace-tests\.py (?:run|artifact) (\S+) ([^\s)]+)"
# A plugin's scripts/ci.* also runs its formatting and lint checks.
PLUGIN_CI_SCRIPT = r'(?:&|bash) "[^"]*/scripts/ci\.(?:ps1|sh)"'
FILESYSTEM_API_TESTS = "cargo test --manifest-path filesystem-api/Cargo.toml --workspace --locked"


def ci_selections():
    """Package and target of every post-gate test executable that CI runs."""
    workflow = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    return sorted(set(re.findall(SELECTION, workflow)))


# Each plugin whose scripts/ci.* test commands validation runs: its directory, the value
# of the step's $Plugin (validate.ps1) and $plugin (validate.sh), and the step body.
PLUGIN_TESTS = (
    ("midnight-viewer-basic", "basic", "viewerBody", "viewer_step"),
    ("midnight-viewer-source", "source", "viewerBody", "viewer_step"),
    ("midnight-viewer-markdown", "markdown", "viewerBody", "viewer_step"),
    ("midnight-viewer-image", "viewer-image", "rasterBody", "raster_step"),
    ("midnight-viewer-pdf", "viewer-pdf", "rasterBody", "raster_step"),
    ("midnight-thumbnail-image", "thumbnail-image", "rasterBody", "raster_step"),
)


def words(text, separators=r"\s"):
    """The words of a command or list; quoted strings and (...) groups stay whole."""
    result, word, depth, quote = [], "", 0, None
    for character in text:
        if quote:
            quote = None if character == quote else quote
        elif character in "'\"":
            quote = character
        elif character in "()":
            depth += 1 if character == "(" else -1
        elif depth == 0 and re.fullmatch(separators, character):
            if word:
                result.append(word)
            word = ""
            continue
        word += character
    return result + [word] if word else result


def unquote(word):
    return word[1:-1] if len(word) > 1 and word[0] == word[-1] and word[0] in "'\"" else word


def open_parentheses(text):
    depth, quote = 0, None
    for character in text:
        if quote:
            quote = None if character == quote else quote
        elif character in "'\"":
            quote = character
        elif character in "()":
            depth += 1 if character == "(" else -1
    return depth


def logical_lines(text, powershell):
    """Lines joined across continuations and, in PowerShell, open parentheses."""
    if powershell:
        text = re.sub(r"<#.*?#>", "", text, flags=re.DOTALL)
    lines, current = [], ""
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        current = f"{current} {line}" if current else line
        if current.endswith(("\\", "`")):
            current = current[:-1].rstrip()
        elif not powershell or open_parentheses(current) <= 0:
            lines.append(current)
            current = ""
    return lines + [current] if current else lines


def plugin_test_commands(text, plugin, powershell):
    """The arguments of each `cargo ... test` command in a script or step body: a sorted
    tuple per run, with variables and expressions as <value>. Assignments of argument
    lists are followed in order, those guarded by a plugin comparison only for that
    plugin, and a command in a loop over projects counts once per project."""
    if powershell:
        # A loop over Cargo operations: its command lines are the test command's.
        for variable, values in re.findall(r"foreach \(\$(\w+) in @\(([^)]*)\)\)", text):
            if "'test'" in values:
                text = re.sub(rf"\${variable}\b", "'test'", text)
    lists, loops, commands = {}, {}, []
    splat = r"@(\w+)" if powershell else r'"\$\{(\w+)\[@\]\}"'

    def expand(items):
        # A list defined elsewhere stays a splat; a test command must not use one.
        expanded = []
        for item in items:
            match = re.fullmatch(splat, item)
            expanded += lists[match.group(1)] if match and match.group(1) in lists else [item]
        return expanded

    def applies(left, operator, right):
        # PowerShell: if ($Plugin -eq 'x') { ... }; bash: [[ $plugin != x ]] || ...
        values = [plugin if value.lower() == "$plugin" else unquote(value) for value in (left, right)]
        return (values[0] == values[1]) == (operator in ("-eq", "!="))

    for line in logical_lines(text, powershell):
        if powershell:
            assignment = re.fullmatch(r"(?:if \((\S+) (-eq|-ne) (\S+)\) \{ )?\$(\w+) (\+?=) (@\(.*?)(?: \})?", line)
            if assignment:
                left, operator, right, name, how, value = assignment.groups()
                if not left or applies(left, operator, right):
                    items = []
                    for term in words(value, r"[+]"):
                        term = term.strip()
                        items += (words(term[2:-1], r"[\s,]") if term.startswith("@(")
                                  else lists.get(term[1:], [term]))
                    lists[name] = (lists.get(name, []) if how == "+=" else []) + items
                continue
            loop = re.fullmatch(r"foreach \(\$(\w+) in (.+)\) \{", line)
            if loop:
                items = loop.group(2)
                loops[loop.group(1)] = len(words(items[2:-1], r"[\s,]") if items.startswith("@(")
                                           else lists.get(items[1:], [items]))
                continue
        else:
            for assignment in re.finditer(r"(?<![\w$])([A-Za-z_]\w*)=\(([^()]*)\)", line):
                guard = re.search(r"\[\[ (\$plugin) (!=|==) ([\w-]+) \]\] \|\|\s*$", line[:assignment.start()])
                if not guard or applies(*guard.groups()):
                    lists[assignment.group(1)] = words(assignment.group(2))
            loop = re.fullmatch(r"for (\w+) in (.+); do", line)
            if loop:
                loops[loop.group(1)] = len(expand(words(loop.group(2))))
                continue
        command = re.fullmatch(r"(?:& ?)?cargo (.+)", line)
        if not command:
            continue
        arguments = expand(words(command.group(1)))
        values = [unquote(argument) for argument in arguments]
        position = 0
        while position < len(values) and values[position].startswith("-"):
            position += 2 if values[position] == "--config" else 1
        if position >= len(values) or values[position] != "test":
            continue
        unresolved = [argument for argument in arguments if re.fullmatch(splat, argument)]
        if unresolved:
            raise AssertionError(f"unknown argument list {unresolved} in: {line}")
        normalized = tuple(sorted("<value>" if "$" in value or value.startswith("(") else value
                                  for index, value in enumerate(values) if index != position))
        runs = [count for variable, count in loops.items()
                if any(re.search(rf"\${{?{variable}\b", argument) for argument in arguments)]
        commands += [normalized] * (runs[-1] if runs else 1)
    return commands


class ValidationPlanTests(unittest.TestCase):
    def setUp(self):
        self.bash = (ROOT / "scripts/validate.sh").read_text(encoding="utf-8")
        self.powershell = (ROOT / "scripts/validate.ps1").read_text(encoding="utf-8")

    def test_both_platforms_list_the_same_steps_in_ci_job_order(self):
        linux, windows = bash_steps(self.bash), powershell_steps(self.powershell)
        self.assertGreaterEqual(len(linux), 40)
        self.assertEqual(linux, windows)
        identifiers = [step[0] for step in linux]
        self.assertEqual(len(identifiers), len(set(identifiers)))
        jobs = [step[1] for step in linux if step[1] != "not in CI"]
        order = ["public-sdk", "public-worker-sdk", "viewers", "viewer-conformance", "image-viewer",
                 "thumbnail-image", "archive-filesystem", "filesystem-libraries", "check", "release"]
        self.assertEqual(sorted(set(jobs), key=order.index), order)
        self.assertEqual(jobs, sorted(jobs, key=order.index), "steps follow CI's job order")

    def test_every_post_gate_test_executable_of_ci_runs_where_ci_runs_it(self):
        selections = ci_selections()
        self.assertGreater(len(selections), 8)
        for script, text in (("validate.sh", self.bash), ("validate.ps1", self.powershell)):
            listed = set(re.findall(SELECTION, text))
            listed |= set(re.findall(r"selection_words=(\S+) (\S+)", text))
            listed |= set(re.findall(r"Selection = @\('([^']+)', '([^']+)'", text))
            for selection in selections:
                if script == "validate.ps1" and selection[0] == "midnight-platform-linux":
                    continue  # Linux-only CI steps; validate.ps1 lists them as skipped.
                with self.subTest(script=script, selection=selection):
                    self.assertIn(selection, listed)

    def test_plugin_steps_run_the_test_commands_of_the_plugin_ci_scripts(self):
        # Validation copies the test commands of plugins/*/scripts/ci.* (which also run the
        # formatting and lint checks); a changed CI test command must be copied too.
        for directory, plugin, powershell_body, bash_body in PLUGIN_TESTS:
            for script, powershell, body in (
                    ("ci.ps1", True, re.search(rf"^( *)\${powershell_body} = \{{\n(.*?)^\1\}}",
                                               self.powershell, re.MULTILINE | re.DOTALL).group(2)),
                    ("ci.sh", False, re.search(rf"^{bash_body}\(\) \{{\n(.*?)^\}}",
                                               self.bash, re.MULTILINE | re.DOTALL).group(1))):
                with self.subTest(plugin=directory, script=script):
                    ci = (ROOT / "plugins" / directory / "scripts" / script).read_text(encoding="utf-8")
                    expected = plugin_test_commands(ci, plugin, powershell)
                    self.assertTrue(expected, f"no cargo test command found in {directory}/scripts/{script}")
                    self.assertEqual(plugin_test_commands(body, plugin, powershell), expected)

    def test_validation_runs_tests_only(self):
        for script, text in (("validate.sh", self.bash), ("validate.ps1", self.powershell)):
            with self.subTest(script=script):
                self.assertIn("scripts/ci-workspace-tests.py --keep-test-artifacts --no-fail-fast", text)
                self.assertIn("-m unittest discover -s scripts/tests -v", text)
                self.assertIn(FILESYSTEM_API_TESTS, text)
                self.assertNotIn("--retire-successful-test-artifacts", text)
                for check in ("cargo fmt", "clippy", "check-source-size"):
                    self.assertNotIn(check, text.lower())
                self.assertEqual(re.findall(PLUGIN_CI_SCRIPT, text), [])


class ValidationRunFixture:
    """A checkout with the validation script and stub tools."""

    def make_fixture(self, script):
        temporary = ROOT / "_build/tmp"
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix="validate ", dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name) / "checkout with spaces"
        (self.root / "scripts").mkdir(parents=True)
        shutil.copyfile(ROOT / "scripts" / script, self.root / "scripts" / script)
        self.bin = Path(self.owner.name) / "bin"
        self.bin.mkdir()
        self.log = Path(self.owner.name) / "calls.log"
        self.cargo_output = Path(self.owner.name) / "cargo-output.txt"
        self.cargo_output.write_text("", encoding="utf-8")
        self.environment = dict(os.environ, VALIDATE_TEST_LOG=str(self.log),
                                CARGO_STUB_OUTPUT=str(self.cargo_output), CARGO_STUB_EXIT="0",
                                PYTHONUTF8="1")
        for name in ("CARGO_TARGET_DIR", "CARGO_NET_OFFLINE", "MIDNIGHT_FFMPEG_PREFIX",
                     "MIDNIGHT_FFMPEG_SOURCE_ARCHIVE", "LIBCLANG_PATH", "HOST_TEST_INSTALL_ROOT"):
            self.environment.pop(name, None)
        # The workspace runner prints a libtest summary and checks the private installation.
        (self.root / "scripts/ci-workspace-tests.py").write_text(
            "import os, sys\n"
            "root = os.environ['HOST_TEST_INSTALL_ROOT']\n"
            "assert os.path.isdir(os.path.join(root, 'viewers')), root\n"
            "assert os.path.isdir(os.path.join(root, 'filesystems')), root\n"
            "if os.name != 'nt':\n"
            "    assert os.stat(root).st_mode & 0o777 == 0o700, oct(os.stat(root).st_mode)\n"
            "print('runner arguments: ' + ' '.join(sys.argv[1:]))\n"
            "print('installation: ' + root)\n"
            "print('test result: ok. 7 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out')\n",
            encoding="utf-8")
        tests = self.root / "plugins/image-decoder/native/tests"
        tests.mkdir(parents=True)
        (tests / "test_fixture.py").write_text(
            "import unittest\n"
            "class Fixture(unittest.TestCase):\n"
            "    def test_one(self): pass\n"
            "    def test_two(self): pass\n"
            "    @unittest.skip('fixture')\n"
            "    def test_skipped(self): pass\n"
            "    def test_failed(self): self.fail('fixture')\n", encoding="utf-8")
        self.worker_sdk = Path(self.owner.name) / "registry.toml"
        self.worker_sdk.write_text("# supplied worker SDK\n", encoding="utf-8")

    def calls(self):
        return self.log.read_text(encoding="utf-8") if self.log.exists() else ""

    def summary_row(self, output, identifier):
        rows = [line.split() for line in output.splitlines()
                if re.match(rf"^\s*\d+ {re.escape(identifier)} ", line)]
        self.assertEqual(len(rows), 1, output)
        return rows[0][2:]

    def assert_plan_lists_every_step(self, output, skipped_platform_step, platform_reason):
        listed = re.findall(r"^\s*\d+\. (\S+) - ", output, re.MULTILINE)
        script = (ROOT / "scripts/validate.sh").read_text(encoding="utf-8")
        self.assertEqual(listed, [step[0] for step in bash_steps(script)])
        line = next(line for line in output.splitlines() if f". {skipped_platform_step} - " in line)
        self.assertIn(f"(skip: {platform_reason}", line)
        self.assertIn("ci-workspace-tests.py --keep-test-artifacts --no-fail-fast", output)
        self.assertIn(FILESYSTEM_API_TESTS, output)
        self.assertNotIn("cargo fmt", output)
        self.assertNotIn("clippy", output.lower())


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux validation script")
class LinuxValidationTests(ValidationRunFixture, unittest.TestCase):
    def setUp(self):
        self.make_fixture("validate.sh")
        self.command("cargo", 'printf "cargo %s\\n" "$*" >> "$VALIDATE_TEST_LOG"\n'
                     'cat -- "$CARGO_STUB_OUTPUT"\nexit "$CARGO_STUB_EXIT"\n')
        self.command("rustc", 'printf "host: x86_64-unknown-linux-gnu\\n"\n')
        # The system tools and this Python; Cargo homes are not on PATH.
        (self.bin / "python3").symlink_to(shutil.which("python3"))
        self.environment.update(PATH=str(self.bin) + os.pathsep + "/usr/bin:/bin",
                                HOME=str(Path(self.owner.name) / "home"))

    def command(self, name, body):
        path = self.bin / name
        path.write_text("#!/bin/sh\n" + body, encoding="utf-8")
        path.chmod(0o755)

    def run_validation(self, *arguments):
        return subprocess.run(["bash", str(self.root / "scripts/validate.sh"), *arguments],
                              cwd=self.owner.name, env=self.environment, text=True,
                              capture_output=True, timeout=120)

    def test_invalid_arguments_stop_before_any_build_tool(self):
        for arguments in INVALID_ARGUMENTS + (("clean",),):
            with self.subTest(arguments=arguments):
                result = self.run_validation(*arguments)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Usage: scripts/validate.sh", result.stderr)
                self.assertEqual(self.calls(), "")
                self.assertFalse((self.root / "_build/validate").exists())

    def test_dry_run_prints_every_step_with_its_commands_and_runs_nothing(self):
        result = self.run_validation("--dry-run", "--offline", "release")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_plan_lists_every_step(result.stdout, "powershell-tests-7", "Windows only")
        self.assertIn("CARGO_NET_OFFLINE=true", result.stdout)
        self.assertIn("offline_flag=(--offline)", result.stdout)
        self.assertIn("host-build - Host build (release)", result.stdout)
        self.assertIn("(run if these pass: sdk-package)", result.stdout)
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build/validate").exists())

    def test_only_selects_the_named_steps_and_the_steps_they_need(self):
        result = self.run_validation("--dry-run", "--only", "viewer-basic,smoke-test")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(re.findall(r"^\s*\d+\. (\S+) - ", result.stdout, re.MULTILINE),
                         ["sdk-package", "viewer-basic", "test-install", "host-build", "smoke-test"])

    def test_results_set_the_exit_status_summary_and_test_counts(self):
        self.cargo_output.write_text(
            "test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n"
            "test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n",
            encoding="utf-8")
        result = self.run_validation("--only", "filesystem-api,decoder-native-tests,workspace-tests")
        output = result.stdout
        self.assertEqual(result.returncode, 1, output + result.stderr)
        self.assertEqual(self.summary_row(output, "filesystem-api")[:1]
                         + self.summary_row(output, "filesystem-api")[2:5], ["PASS", "5", "1", "1"])
        self.assertEqual(self.summary_row(output, "decoder-native-tests")[:1]
                         + self.summary_row(output, "decoder-native-tests")[2:5], ["FAIL", "2", "1", "1"])
        self.assertEqual(self.summary_row(output, "workspace-tests")[:1]
                         + self.summary_row(output, "workspace-tests")[2:5], ["PASS", "7", "0", "2"])
        self.assertIn("runner arguments: --keep-test-artifacts --no-fail-fast", output)
        self.assertIn("Tests: 14 passed, 2 failed, 4 ignored.", output)
        self.assertIn("VALIDATION FAILED", output)
        logs = sorted(path.name for path in (self.root / "_build/validate/logs").iterdir())
        self.assertEqual(len(logs), 1)
        run = self.root / "_build/validate/logs" / logs[0]
        self.assertTrue((run / "summary.txt").read_text(encoding="utf-8").strip().endswith(str(run)))
        self.assertIn(FILESYSTEM_API_TESTS, self.calls())
        # The owner-only private installation reached the workspace tests and is gone.
        installation = re.search(r"^installation: (.+)$", output, re.MULTILINE).group(1)
        self.assertTrue(installation.startswith(str(self.root / "_build/tmp/viewer-tests-")))
        self.assertFalse(Path(installation).exists())
        self.assertIn(f"Removed the private test installation {installation}", output)

    def test_failed_step_skips_what_needs_it_and_passing_runs_exit_zero(self):
        self.environment["CARGO_STUB_EXIT"] = "3"
        result = self.run_validation("--only", "viewer-basic")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.summary_row(result.stdout, "sdk-package")[:1], ["FAIL"])
        self.assertIn("needs sdk-package (FAIL)", result.stdout)
        self.environment["CARGO_STUB_EXIT"] = "0"
        result = self.run_validation("--only", "filesystem-api")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("VALIDATION PASSED", result.stdout)

    def test_strict_fails_for_missing_prerequisites_but_not_other_platforms(self):
        arguments = ("--only", "thumbnail-video,powershell-tests-5", "--worker-sdk", str(self.worker_sdk))
        result = self.run_validation(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("no prepared FFmpeg 9.0.2", result.stdout)
        self.assertIn("uses the bundle given with --worker-sdk", result.stdout)
        # Neither a step that CI does not run nor another platform's step is a CI gap.
        self.assertNotIn("WARNING:", result.stdout)
        result = self.run_validation(*arguments, "--strict")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("VALIDATION FAILED: --strict and 1 step(s) skipped", result.stdout)
        result = self.run_validation("--only", "powershell-tests-5", "--strict")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_ci_steps_skipped_for_a_prerequisite_are_named_before_the_result(self):
        # Offline, the image viewer's pinned dav1d downloads are not cached in the fixture.
        arguments = ("--only", "viewer-image", "--worker-sdk", str(self.worker_sdk), "--offline")
        result = self.run_validation(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        warning = lines.index("WARNING: 1 CI step(s) SKIPPED for a missing prerequisite or needed step: "
                              "viewer-image (use --strict to fail)")
        self.assertEqual(lines[warning + 1], "VALIDATION PASSED")
        result = self.run_validation(*arguments, "--strict")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("VALIDATION FAILED: --strict and 1 step(s) skipped", result.stdout)

    def test_missing_rust_or_python_stops_with_127(self):
        (self.bin / "rustc").unlink()
        result = self.run_validation("--only", "filesystem-api")
        self.assertEqual(result.returncode, 127, result.stdout + result.stderr)
        self.assertIn("rustc on PATH", result.stderr)
        self.assertEqual(self.calls(), "")


@unittest.skipUnless(os.name == "nt", "Windows validation script")
class WindowsValidationTests(ValidationRunFixture, unittest.TestCase):
    def setUp(self):
        self.make_fixture("validate.ps1")
        (self.bin / "cargo.cmd").write_text(
            '@echo off\r\n>>"%VALIDATE_TEST_LOG%" echo cargo %*\r\n'
            'type "%CARGO_STUB_OUTPUT%"\r\nexit /b %CARGO_STUB_EXIT%\r\n', encoding="utf-8")
        (self.bin / "rustc.cmd").write_text(
            "@echo off\r\necho host: x86_64-pc-windows-msvc\r\n", encoding="utf-8")
        self.environment["PATH"] = str(self.bin) + os.pathsep + os.environ["PATH"]
        self.shell = shutil.which("pwsh") or "powershell.exe"

    def run_validation(self, *arguments):
        return subprocess.run([self.shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                               str(self.root / "scripts/validate.ps1"), *arguments],
                              cwd=self.owner.name, env=self.environment, capture_output=True,
                              text=True, encoding="utf-8", errors="replace", timeout=180)

    def test_invalid_arguments_stop_before_any_build_tool(self):
        for arguments in INVALID_ARGUMENTS:
            with self.subTest(arguments=arguments):
                result = self.run_validation(*arguments)
                self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
                self.assertIn("Usage: scripts\\validate.ps1", result.stderr)
                self.assertEqual(self.calls(), "")
                self.assertFalse((self.root / "_build/validate").exists())

    def test_dry_run_prints_every_step_with_its_commands_and_runs_nothing(self):
        result = self.run_validation("--dry-run", "--offline")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_plan_lists_every_step(result.stdout, "trash-qualification", "Linux only")
        self.assertIn("CARGO_NET_OFFLINE=true", result.stdout)
        self.assertIn("$OfflineFlag = @('--offline')", result.stdout)
        # CI sets TEMP, TMP and TMPDIR; Python's tempfile prefers TMPDIR.
        for name in ("TEMP", "TMP", "TMPDIR"):
            self.assertRegex(result.stdout, rf"[ ;]{name}=[^;]*\\_build\\tmp;")
        self.assertEqual(self.calls(), "")
        self.assertFalse((self.root / "_build/validate").exists())

    def test_only_takes_ids_in_any_case_and_lists_from_a_powershell_session(self):
        result = self.run_validation("--dry-run", "--only", "Workspace-Tests")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(re.findall(r"^\s*\d+\. (\S+) - ", result.stdout, re.MULTILINE),
                         ["test-install", "workspace-tests"])
        # In a session, PowerShell passes "a,b" to the script as an array.
        script = str(self.root / "scripts/validate.ps1").replace("'", "''")
        result = subprocess.run(
            [self.shell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
             f"& '{script}' --dry-run --only filesystem-api,decoder-native-tests; exit $LASTEXITCODE"],
            cwd=self.owner.name, env=self.environment, capture_output=True, text=True,
            encoding="utf-8", errors="replace", timeout=180)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(re.findall(r"^\s*\d+\. (\S+) - ", result.stdout, re.MULTILINE),
                         ["filesystem-api", "decoder-native-tests"])

    def test_ci_steps_skipped_for_a_prerequisite_are_named_before_the_result(self):
        # Offline, the image viewer's pinned dav1d downloads are not cached in the fixture.
        arguments = ("--only", "viewer-image", "--worker-sdk", str(self.worker_sdk), "--offline")
        result = self.run_validation(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        warning = lines.index("WARNING: 1 CI step(s) SKIPPED for a missing prerequisite or needed step: "
                              "viewer-image (use --strict to fail)")
        self.assertEqual(lines[warning + 1], "VALIDATION PASSED")
        result = self.run_validation(*arguments, "--strict")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("VALIDATION FAILED: --strict and 1 step(s) skipped", result.stdout)

    def test_results_skips_strictness_and_the_private_installation(self):
        self.cargo_output.write_text(
            "test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out\n",
            encoding="utf-8")
        arguments = ("--only", "filesystem-api,decoder-native-tests,workspace-tests,thumbnail-video,"
                     "trash-qualification", "--worker-sdk", str(self.worker_sdk))
        result = self.run_validation(*arguments)
        output = result.stdout
        self.assertEqual(result.returncode, 1, output + result.stderr)
        self.assertEqual(self.summary_row(output, "filesystem-api")[:1]
                         + self.summary_row(output, "filesystem-api")[2:5], ["PASS", "3", "0", "1"])
        self.assertEqual(self.summary_row(output, "decoder-native-tests")[:1]
                         + self.summary_row(output, "decoder-native-tests")[2:5], ["FAIL", "2", "1", "1"])
        self.assertEqual(self.summary_row(output, "workspace-tests")[:1]
                         + self.summary_row(output, "workspace-tests")[2:5], ["PASS", "7", "0", "2"])
        self.assertEqual(self.summary_row(output, "trash-qualification")[:1], ["SKIPPED"])
        self.assertIn("no FFmpeg 9.0.2 MSVC prefix", output)
        self.assertIn("runner arguments: --keep-test-artifacts --no-fail-fast", output)
        installation = re.search(r"^installation: (.+)$", output, re.MULTILINE).group(1).strip()
        self.assertTrue(installation.startswith(os.path.join(os.environ["USERPROFILE"], "MidnightViewerTests-")))
        self.assertFalse(Path(installation).exists())
        self.assertIn("VALIDATION FAILED", output)
        # Without the failing fixture: skips for other platforms or supplied SDKs never fail,
        # a missing prerequisite fails only with --strict.
        (self.root / "plugins/image-decoder/native/tests/test_fixture.py").unlink()
        arguments = ("--only", "filesystem-api,thumbnail-video,trash-qualification", "--worker-sdk",
                     str(self.worker_sdk))
        result = self.run_validation(*arguments)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("VALIDATION PASSED", result.stdout)
        # Neither a step that CI does not run nor another platform's step is a CI gap.
        self.assertNotIn("WARNING:", result.stdout)
        result = self.run_validation(*arguments, "--strict")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("VALIDATION FAILED: --strict and 1 step(s) skipped", result.stdout)

    def test_failed_step_skips_what_needs_it(self):
        self.environment["CARGO_STUB_EXIT"] = "3"
        result = self.run_validation("--only", "viewer-basic")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.summary_row(result.stdout, "sdk-package")[:1], ["FAIL"])
        self.assertIn("needs sdk-package (FAIL)", result.stdout)


if __name__ == "__main__":
    unittest.main()
