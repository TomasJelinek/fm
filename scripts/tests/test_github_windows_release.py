"""Exercise draft mutations through a fake GitHub CLI, without network or credentials."""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
PWSH = shutil.which("pwsh")
STEM = "midnight-explorer-0.3.0-x86_64-pc-windows-msvc"
ARCHIVE = STEM + ".zip"
INSTALLER = STEM + ".msi"
REVISION = "a" * 40

HARNESS = r"""
$ErrorActionPreference = 'Stop'
$global:LASTEXITCODE = 0
$fixture = Get-Content -LiteralPath $env:FIXTURE -Raw | ConvertFrom-Json -AsHashtable
$global:fixtureAssets = @($fixture.assets)
$global:fixtureMutations = @()
function gh {
    $arguments = @($args)
    $global:LASTEXITCODE = 0
    if ($arguments[0] -eq 'api') {
        $endpoint = $arguments[-1]
        if ($arguments -contains 'DELETE') {
            $global:fixtureMutations += $endpoint
            $id = [int]($endpoint -split '/')[-1]
            $global:fixtureAssets = @($global:fixtureAssets | Where-Object { $_.id -ne $id })
            if ($fixture.publish_after_delete) { $fixture.draft = $false }
            return
        }
        if ($endpoint -match '/git/ref/tags/') {
            return @{object = @{type = 'commit'; sha = $fixture.tag_revision}} | ConvertTo-Json -Depth 8
        }
        if ($endpoint -match '/compare/') { return '{"status":"ahead"}' }
        if ($endpoint -match '/releases/tags/') {
            return @{id = 42; draft = $fixture.draft; tag_name = 'v0.3.0'; assets = @()} | ConvertTo-Json
        }
        if ($endpoint -match '/releases/42/assets\?') {
            return ConvertTo-Json -InputObject @($global:fixtureAssets) -Depth 8
        }
        throw "Unexpected endpoint $endpoint"
    }
    if ($arguments[0] -eq 'release' -and $arguments[1] -eq 'upload') {
        $path = $arguments[3]
        $name = [IO.Path]::GetFileName($path)
        if (@($global:fixtureAssets | Where-Object {$_.name -ceq $name}).Count -ne 0) {
            throw 'Upload attempted to clobber an asset.'
        }
        $global:fixtureMutations += $name
        $global:fixtureAssets += @{id = 100 + $global:fixtureMutations.Count; name = $name
            size = (Get-Item -LiteralPath $path).Length; updated_at = 'new'
            digest = 'sha256:' + (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()}
        if ($fixture.bad_digest) { $global:fixtureAssets[-1].digest = 'sha256:wrong' }
        return
    }
    throw 'Unexpected gh command'
}
try {
    & $env:RELEASE_SCRIPT -Operation Upload
    $result = @{success = $true; mutations = $global:fixtureMutations; assets = $global:fixtureAssets}
} catch {
    $result = @{success = $false; error = $_.Exception.Message; mutations = $global:fixtureMutations}
}
'RESULT:' + ($result | ConvertTo-Json -Depth 12 -Compress)
"""


@unittest.skipUnless(PWSH, "PowerShell 7 is required")
class DraftReleaseTests(unittest.TestCase):
    def run_upload(self, *, draft=True, changed=False, tag_changed=False, checksums=None,
                   publish_after_delete=False, bad_digest=False, tag="v0.3.0"):
        temporary_root = ROOT / "_build" / "tests"
        temporary_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=temporary_root) as temporary:
            root = Path(temporary)
            scripts = root / "scripts"
            scripts.mkdir()
            script = scripts / "github-windows-release.ps1"
            shutil.copyfile(ROOT / "scripts" / script.name, script)
            signing = root / "_build/windows-signing"
            output = signing / "work/output"
            output.mkdir(parents=True)
            inputs = signing / "input"
            inputs.mkdir()
            for name in (ARCHIVE, ARCHIVE + ".sha256", ARCHIVE + ".signing.json",
                         INSTALLER, INSTALLER + ".sha256", INSTALLER + ".signing.json"):
                (output / name).write_bytes(("signed:" + name).encode())
            original = ("1" * 64 + "  linux.tar.gz\r\n" + "2" * 64 + "  " + ARCHIVE + "\r\n")
            (inputs / "SHA256SUMS").write_bytes((checksums if checksums is not None else original).encode())
            assets = [dict(id=1, name=ARCHIVE, size=3, updated_at="old"),
                      dict(id=2, name="SHA256SUMS", size=4, updated_at="old")]
            identities = {name: None for name in (ARCHIVE, ARCHIVE + ".sha256", ARCHIVE + ".signing.json",
                                                  INSTALLER, INSTALLER + ".sha256", INSTALLER + ".signing.json",
                                                  "SHA256SUMS")}
            identities.update({ARCHIVE: "1:3:old", "SHA256SUMS": "2:4:old"})
            state = dict(repository="example/repo", tag="v0.3.0", revision=REVISION,
                         release_id=42, archive_name=ARCHIVE, assets=identities)
            (signing / "release-state.json").write_text(json.dumps(state))
            if changed:
                assets[0]["updated_at"] = "changed"
            fixture = root / "fixture.json"
            fixture.write_text(json.dumps(dict(assets=assets, draft=draft,
                                               tag_revision="b" * 40 if tag_changed else REVISION,
                                               publish_after_delete=publish_after_delete, bad_digest=bad_digest)))
            harness = root / "harness.ps1"
            harness.write_text(HARNESS)
            environment = dict(os.environ, FIXTURE=str(fixture), RELEASE_SCRIPT=str(script),
                               GH_TOKEN="fixture", GITHUB_REPOSITORY="example/repo",
                               GITHUB_REF="refs/heads/main", DEFAULT_BRANCH="main", RELEASE_TAG=tag)
            completed = subprocess.run([PWSH, "-NoProfile", "-File", str(harness)],
                                       env=environment, capture_output=True, text=True, timeout=30)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            result = json.loads(next(line[7:] for line in completed.stdout.splitlines() if line.startswith("RESULT:")))
            result["checksums"] = (output / "SHA256SUMS").read_bytes() if (output / "SHA256SUMS").exists() else b""
            return result

    def test_updates_draft_zip_installer_and_preserves_linux_checksum(self):
        result = self.run_upload()
        self.assertTrue(result["success"], result)
        self.assertEqual(len(result["assets"]), 7)
        self.assertTrue(result["checksums"].startswith(b"1" * 64 + b"  linux.tar.gz\r\n"))
        for name in (ARCHIVE, INSTALLER):
            digest = hashlib.sha256(("signed:" + name).encode()).hexdigest()
            self.assertIn((digest + "  " + name).encode(), result["checksums"])

    def test_published_release_is_rejected_before_mutation(self):
        result = self.run_upload(draft=False)
        self.assertFalse(result["success"])
        self.assertEqual(result["mutations"], [])

    def test_changed_asset_is_rejected_before_mutation(self):
        result = self.run_upload(changed=True)
        self.assertFalse(result["success"])
        self.assertEqual(result["mutations"], [])

    def test_moved_tag_is_rejected_before_mutation(self):
        result = self.run_upload(tag_changed=True)
        self.assertFalse(result["success"])
        self.assertEqual(result["mutations"], [])

    def test_malformed_or_duplicate_global_checksums_are_rejected(self):
        line = "1" * 64 + "  " + ARCHIVE + "\n"
        for content in ("malformed\n", line + line, line + "\n"):
            with self.subTest(content=content):
                result = self.run_upload(checksums=content)
                self.assertFalse(result["success"])
                self.assertEqual(result["mutations"], [])

    def test_publication_between_delete_and_upload_stops_further_mutation(self):
        result = self.run_upload(publish_after_delete=True)
        self.assertFalse(result["success"])
        self.assertEqual(result["mutations"], ["repos/example/repo/releases/assets/1"])

    def test_uploaded_content_digest_mismatch_stops_further_mutation(self):
        result = self.run_upload(bad_digest=True)
        self.assertFalse(result["success"])
        self.assertIn("digest mismatch", result["error"])
        self.assertEqual(len(result["mutations"]), 2)

    def test_non_msi_versions_are_rejected_before_mutation(self):
        for tag in ("v0.3.0-beta.1", "v256.0.0", "v0.0.65536"):
            with self.subTest(tag=tag):
                result = self.run_upload(tag=tag)
                self.assertFalse(result["success"])
                self.assertEqual(result["mutations"], [])


if __name__ == "__main__":
    unittest.main()
