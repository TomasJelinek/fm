"""Deployment payload tooling fixtures; synthetic EXEs are never executed or qualified as MAX."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

from scripts.tests.test_runtime_packages import complete_runtime


ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / 'scripts/deployment-validation-payload.ps1'
PWSH = shutil.which('pwsh')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def quoted(value):
    return "'" + str(value).replace("'", "''") + "'"


@unittest.skipUnless(os.name == 'nt' and PWSH, 'Windows PowerShell 7 tooling')
class ValidationPayloadTests(unittest.TestCase):
    def setUp(self):
        temporary = ROOT / '_build/tmp'
        temporary.mkdir(parents=True, exist_ok=True)
        self.owner = tempfile.TemporaryDirectory(prefix='validation-payload-', dir=temporary)
        self.addCleanup(self.owner.cleanup)
        self.root = Path(self.owner.name)
        self.source = self.root / 'debug'
        self.artifact = self.source / 'deps/midnight_explorer-0123456789abcdef.exe'
        self.artifact.parent.mkdir(parents=True)
        self.artifact.write_bytes(b'owned synthetic tooling payload\x00\xff')
        (self.source / 'midnight-explorer.exe').write_bytes(b'owned normal app tooling fixture')
        self.destination = self.root / 'new-product'
        self.snapshot = self.root / 'source-snapshot.json'
        self.source_rows = [
            {'path': str(ROOT / path), 'sha256': digest(ROOT / path)}
            for path in ('Cargo.toml', 'Cargo.lock', 'crates/midnight-explorer/Cargo.toml',
                         'crates/midnight-explorer/src/main.rs')
        ]
        self.snapshot.write_text(json.dumps(self.source_rows), encoding='utf-8')
        self.cargo = self.root / 'compile.jsonl'
        self.row = {
            'reason': 'compiler-artifact',
            'package_id': 'path+' + (ROOT / 'crates/midnight-explorer').as_uri() + '#0.2.0',
            'manifest_path': str(ROOT / 'crates/midnight-explorer/Cargo.toml'),
            'target': {'name': 'midnight-explorer', 'kind': ['bin'], 'crate_types': ['bin'],
                       'src_path': str(ROOT / 'crates/midnight-explorer/src/main.rs')},
            'profile': {'test': True, 'opt_level': '0', 'debuginfo': 0, 'debug_assertions': True},
            'features': [], 'filenames': [str(self.artifact)],
            'executable': str(self.artifact), 'fresh': False,
        }
        self.events = [self.row, {'reason': 'build-finished', 'success': True}]
        self.receipt = self.root / 'payload-receipt.json'
        self.evidence = {
            'schema_version': 1, 'cargo_json': str(self.cargo), 'cargo_json_sha256': '',
            'cargo_exit_code': 0, 'artifact_sha256': digest(self.artifact),
            'application_sha256': digest(self.source / 'midnight-explorer.exe'),
            'source_snapshot': str(self.snapshot), 'source_snapshot_sha256': digest(self.snapshot),
            'target': 'x86_64-pc-windows-msvc', 'profile': 'debug',
        }
        self.receipt_array = False

    def save(self):
        self.cargo.write_text(''.join(json.dumps(row) + '\n' for row in self.events), encoding='utf-8')
        self.evidence['cargo_json_sha256'] = digest(self.cargo)
        self.receipt.write_text(json.dumps([self.evidence] if self.receipt_array else self.evidence), encoding='utf-8')

    def invoke(self, action='qualify'):
        self.save()
        driver = self.root / 'driver.ps1'
        operation = ''
        if action in ('copy', 'late-snapshot', 'private-copy'):
            mutation = f"[IO.File]::AppendAllText({quoted(self.snapshot)},' ')\n" if action == 'late-snapshot' else ''
            if action == 'private-copy':
                mutation = (
                    "$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value\n"
                    "$acl=[Security.AccessControl.DirectorySecurity]::new()\n"
                    "$acl.SetSecurityDescriptorSddlForm(\"D:P(A;OICI;FA;;;$sid)(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)\")\n"
                    f"Set-Acl -LiteralPath {quoted(self.destination)} -AclObject $acl\n")
            operation = mutation + (
                f"$record = Copy-DeploymentValidationPayload $lease {quoted(self.destination)}; "
                f"Write-DeploymentValidationInventory $record {quoted(self.destination)}; "
                "$record | ConvertTo-Json -Depth 12")
        elif action == 'locked':
            operation = "$stream = [IO.File]::Open($lease.Source,[IO.FileMode]::Open,[IO.FileAccess]::Write,[IO.FileShare]::ReadWrite); $stream.Dispose()"
        elif action == 'runtime':
            operation = f"Assert-DeploymentValidationRuntime {quoted(ROOT)} {quoted(self.source)} {quoted(sys.executable)}"
        else:
            operation = '$lease.Metadata | ConvertTo-Json -Depth 12'
        driver.write_text(
            "$ErrorActionPreference='Stop'\n"
            f". {quoted(HELPER)}\n"
            "$lease=$null\ntry {\n"
            f"$lease=Open-DeploymentValidationPayload {quoted(self.receipt)} {quoted(ROOT)} {quoted(self.source)}\n"
            + operation + "\n} finally { if($lease){$lease.Stream.Dispose();$lease.ApplicationStream.Dispose()} }\n",
            encoding='utf-8',
        )
        return subprocess.run([PWSH, '-NoProfile', '-File', str(driver)], cwd=ROOT,
                              capture_output=True, text=True, timeout=40)

    def assert_refused(self, expected, action='qualify'):
        result = self.invoke(action)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(expected, result.stderr)
        if action not in ('copy', 'late-snapshot', 'private-copy'):
            self.assertFalse(self.destination.exists(), 'refusal created a product directory')
        return result

    def test_exact_receipt_selects_real_cargo_path_and_hashes_copy_bytes(self):
        self.destination.mkdir()
        (self.destination / 'midnight-explorer.exe').write_bytes(b'owned normal app tooling fixture')
        result = self.invoke('copy')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        record = json.loads(result.stdout)
        copied = self.destination / self.artifact.name
        self.assertEqual(copied.read_bytes(), b'owned synthetic tooling payload\x00\xff')
        self.assertEqual(record['sha256'], digest(self.artifact))
        self.assertEqual(record['path'], self.artifact.name)
        inventory = json.loads((self.destination / 'validation-payload.json').read_text(encoding='utf-8'))
        self.assertEqual(inventory['application_sha256'], digest(self.source / 'midnight-explorer.exe'))
        self.assertEqual(inventory['source_snapshot_sha256'], digest(self.snapshot))
        self.assertEqual(inventory['live_source_files_checked'], 4)
        self.assertEqual((self.destination / 'midnight-explorer.exe').read_bytes(), b'owned normal app tooling fixture')

    def test_wrong_package_manifest_or_target_cannot_authorize_payload(self):
        for key, value in [('package_id', 'registry+elsewhere#midnight-explorer@0.2.0'),
                           ('manifest_path', str(ROOT / 'crates/midnight-engine/Cargo.toml'))]:
            with self.subTest(key=key):
                old = self.row[key]
                self.row[key] = value
                self.assert_refused('Explorer')
                self.row[key] = old
        self.row['target']['name'] = 'another'
        self.assert_refused('Explorer')

    def test_production_exe_profile_and_string_boolean_are_rejected(self):
        for value in (False, 'true'):
            with self.subTest(value=value):
                self.row['profile']['test'] = value
                self.assert_refused('Explorer')

    def test_failed_incomplete_or_nonterminal_build_is_rejected(self):
        for events in ([self.row], [self.row, {'reason': 'build-finished', 'success': False}],
                       [self.row, {'reason': 'build-finished', 'success': True}, self.row]):
            with self.subTest(events=len(events)):
                self.events = events
                self.assert_refused('build-finished')

    def test_nonzero_cargo_exit_and_ambiguous_artifact_are_rejected(self):
        self.evidence['cargo_exit_code'] = 1
        self.assert_refused('Cargo exit')
        self.evidence['cargo_exit_code'] = 0
        self.events.insert(1, dict(self.row))
        self.assert_refused('exactly one')

    def test_artifact_hash_and_snapshot_hash_mismatch_refuse_before_copy(self):
        self.evidence['artifact_sha256'] = '0' * 64
        self.assert_refused('payload SHA256')
        self.evidence['artifact_sha256'] = digest(self.artifact)
        self.evidence['source_snapshot_sha256'] = '0' * 64
        self.assert_refused('snapshot SHA256')

    def test_stale_malformed_or_incomplete_live_snapshot_is_rejected(self):
        variants = [[], {'path': 'not an array'}, self.source_rows[1:],
                    [dict(row, sha256='0' * 64) for row in self.source_rows],
                    self.source_rows + [self.source_rows[0]]]
        for rows in variants:
            with self.subTest(rows=rows):
                self.snapshot.write_text(json.dumps(rows), encoding='utf-8')
                self.evidence['source_snapshot_sha256'] = digest(self.snapshot)
                self.assert_refused('snapshot')

    def test_array_wrapped_objects_cannot_bypass_receipt_or_snapshot_structure(self):
        self.receipt_array = True
        self.assert_refused('receipt')
        self.receipt_array = False
        for field in ('target', 'profile', 'cargo_json', 'source_snapshot'):
            original = self.evidence[field]
            try:
                self.evidence[field] = [original]
                with self.subTest(receipt_field=field):
                    self.assert_refused('receipt')
            finally:
                self.evidence[field] = original
        self.snapshot.write_text(json.dumps([[row] for row in self.source_rows]), encoding='utf-8')
        self.evidence['source_snapshot_sha256'] = digest(self.snapshot)
        self.assert_refused('snapshot')
        self.snapshot.write_text(json.dumps(self.source_rows), encoding='utf-8')
        self.evidence['source_snapshot_sha256'] = digest(self.snapshot)
        self.events = [[self.row], {'reason': 'build-finished', 'success': True}]
        self.assert_refused('Cargo JSON event')
        self.events = [self.row, {'reason': 'build-finished', 'success': True}]
        for field in ('target', 'profile'):
            original = self.row[field]
            try:
                self.row[field] = [original]
                with self.subTest(field=field):
                    self.assert_refused('Cargo')
            finally:
                self.row[field] = original
        self.row['target']['kind'] = [['bin']]
        self.assert_refused('Cargo')
        self.row['target']['kind'] = ['bin']
        self.row['reason'] = ['compiler-artifact']
        self.assert_refused('Cargo')

    def test_application_hash_binds_payload_to_same_assembly_epoch(self):
        self.evidence['application_sha256'] = '0' * 64
        self.assert_refused('application SHA256')

    def test_snapshot_changed_after_qualification_refuses_before_payload_creation(self):
        self.destination.mkdir()
        (self.destination / 'midnight-explorer.exe').write_bytes(b'owned normal app tooling fixture')
        self.assert_refused('snapshot', 'late-snapshot')
        self.assertEqual(sorted(p.name for p in self.destination.iterdir()), ['midnight-explorer.exe'])

    def test_receipt_cargo_hash_cannot_be_substituted(self):
        self.save()
        self.evidence['cargo_json_sha256'] = '0' * 64
        self.receipt.write_text(json.dumps(self.evidence), encoding='utf-8')
        driver = self.root / 'hash-driver.ps1'
        driver.write_text(f"$ErrorActionPreference='Stop'; . {quoted(HELPER)}; "
                          f"Open-DeploymentValidationPayload {quoted(self.receipt)} {quoted(ROOT)} {quoted(self.source)}",
                          encoding='utf-8')
        result = subprocess.run([PWSH, '-NoProfile', '-File', str(driver)], cwd=ROOT,
                                capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Cargo JSON SHA256', result.stderr)
        self.assertFalse(self.destination.exists())

    def test_artifact_outside_reported_deps_and_missing_filename_are_rejected(self):
        self.row['filenames'] = []
        self.assert_refused('filenames')
        self.row['filenames'] = [str(self.artifact)]
        self.row['executable'] = str(self.root / self.artifact.name)
        self.row['filenames'] = [self.row['executable']]
        self.assert_refused('deps')

    def test_relative_cargo_executable_is_not_reinterpreted_against_cwd(self):
        self.row['executable'] = os.path.relpath(self.artifact, ROOT)
        self.row['filenames'] = [self.row['executable']]
        self.assert_refused('absolute')

    def test_receipt_target_and_profile_cannot_relabel_another_build(self):
        for field, value in [('target', 'aarch64-pc-windows-msvc'), ('profile', 'release')]:
            with self.subTest(field=field):
                previous = self.evidence[field]
                self.evidence[field] = value
                self.assert_refused('Windows x64 debug')
                self.evidence[field] = previous

    def test_release_profile_artifact_cannot_be_relabelled_debug(self):
        self.row['profile']['debug_assertions'] = False
        self.assert_refused('Explorer')

    def test_destination_app_epoch_mismatch_refuses_without_adding_payload(self):
        self.destination.mkdir()
        (self.destination / 'midnight-explorer.exe').write_bytes(b'different app')
        self.assert_refused('application SHA256', 'copy')
        self.assertEqual(sorted(p.name for p in self.destination.iterdir()), ['midnight-explorer.exe'])

    def test_lease_denies_modifying_source_before_copy(self):
        self.assert_refused('being used', 'locked')
        self.assertEqual(self.artifact.read_bytes(), b'owned synthetic tooling payload\x00\xff')

    def test_copy_collision_keeps_existing_app_payload_and_receipt(self):
        self.destination.mkdir()
        collision = self.destination / self.artifact.name
        collision.write_bytes(b'existing payload')
        self.assert_refused('already exists', 'copy')
        self.assertEqual(collision.read_bytes(), b'existing payload')
        collision.unlink()
        inventory = self.destination / 'validation-payload.json'
        inventory.write_bytes(b'existing inventory')
        self.assert_refused('already exists', 'copy')
        self.assertFalse(collision.exists())
        self.assertEqual(inventory.read_bytes(), b'existing inventory')

    def test_new_payload_inherits_new_owned_private_directory_acl(self):
        # Only this new D fixture receives an ACL. Its ancestors remain untrusted:
        # this is a copy/inheritance test, never a qualified product installation.
        self.destination.mkdir()
        (self.destination / 'midnight-explorer.exe').write_bytes(b'owned normal app tooling fixture')
        result = self.invoke('private-copy')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        driver = self.root / 'acl-readback.ps1'
        driver.write_text(
            f"$a=Get-Acl -LiteralPath {quoted(self.destination / self.artifact.name)}; "
            "[ordered]@{protected=$a.AreAccessRulesProtected;explicit=@($a.Access|Where-Object{-not $_.IsInherited}).Count} | ConvertTo-Json",
            encoding='utf-8')
        result = subprocess.run([PWSH, '-NoProfile', '-File', str(driver)], cwd=ROOT,
                                capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout), {'protected': False, 'explicit': 0})

    def test_full_runtime_validation_accepts_extra_root_file_but_rejects_missing_archive(self):
        complete_runtime(self.source, 'x86_64-pc-windows-msvc')
        self.evidence['application_sha256'] = digest(self.source / 'midnight-explorer.exe')
        result = self.invoke('runtime')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        shutil.rmtree(self.source / 'filesystems/archive')
        self.assert_refused('Full runtime validation', 'runtime')

    def test_option_is_off_by_default_and_existing_destination_is_never_modified(self):
        self.destination.mkdir()
        sentinel = self.destination / 'sentinel'
        sentinel.write_bytes(b'keep')
        result = subprocess.run([PWSH, '-NoProfile', '-File', str(ROOT / 'scripts/deploy-private.ps1'),
                                 '-SourceDirectory', str(self.source), '-Destination', str(self.destination)],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Destination already exists', result.stderr)
        self.assertEqual(sorted(p.name for p in self.destination.iterdir()), ['sentinel'])
        self.assertEqual(sentinel.read_bytes(), b'keep')

    def test_opt_in_requires_complete_modules_before_destination_creation(self):
        self.save()
        result = subprocess.run([PWSH, '-NoProfile', '-File', str(ROOT / 'scripts/deploy-private.ps1'),
                                 '-SourceDirectory', str(self.source), '-Destination', str(self.destination),
                                 '-ValidationPayloadReceipt', str(self.receipt)],
                                cwd=ROOT, capture_output=True, text=True, timeout=30)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('IncludeFilesystems and IncludeThumbnailers', result.stderr)
        self.assertFalse(self.destination.exists())


if __name__ == '__main__':
    unittest.main()
