"""The release signing tool: RFC 8032 vectors, key files, signing and refusals."""
import hashlib
import importlib.util
import io
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location(
    "release_signing", Path(__file__).resolve().parents[1] / "release-signing.py")
signing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(signing)


class Ed25519(unittest.TestCase):
    def test_rfc8032_vectors(self):
        # RFC 8032, section 7.1, tests 1 and 2.
        for seed, public, message, signature in (
            ("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
             "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "",
             "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
             "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"),
            ("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb",
             "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c", "72",
             "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da"
             "085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00"),
        ):
            seed, message = bytes.fromhex(seed), bytes.fromhex(message)
            self.assertEqual(signing.ed25519_public(seed).hex(), public)
            self.assertEqual(signing.ed25519_sign(seed, message).hex(), signature)
            self.assertTrue(signing.ed25519_verify(bytes.fromhex(public), message,
                                                   bytes.fromhex(signature)))
            self.assertFalse(signing.ed25519_verify(bytes.fromhex(public), message + b"x",
                                                    bytes.fromhex(signature)))

    def test_libsodium_scrypt_parameters(self):
        # minisign's default limits select N = 2^20, r = 8, p = 1 (1 GiB).
        self.assertEqual(signing.scrypt_parameters(signing.OPSLIMIT, signing.MEMLIMIT), (20, 8, 1))
        self.assertEqual(signing.scrypt_parameters(524288, 16777216), (14, 8, 1))


class Keys(unittest.TestCase):
    def setUp(self):
        self.folder = Path(tempfile.mkdtemp())

    def generate(self, name="release.key", password=True):
        public = self.folder / "update-signing.pub"
        public.write_text("untrusted comment: PLACEHOLDER\n# none\n")
        args = SimpleNamespace(secret_key=str(self.folder / name), public_key=str(public),
                               no_password=not password)
        signing.command_generate(args, out=io.StringIO())
        return self.folder / name, public

    def test_sign_and_verify_round_trip(self):
        secret, public = self.generate(password=False)
        sums = self.folder / "SHA256SUMS"
        sums.write_text("0" * 64 + "  midnight-explorer-0.5.1-x86_64-pc-windows-msvc.zip\n")
        signing.command_sign(SimpleNamespace(secret_key=str(secret), version="0.5.1",
                                             public_key=str(public), force=False,
                                             sums=str(sums)), out=io.StringIO())
        signing.command_verify(SimpleNamespace(version="0.5.1", public_key=str(public),
                                               sums=str(sums)), out=io.StringIO())
        with self.assertRaises(SystemExit):
            signing.command_verify(SimpleNamespace(version="0.5.2", public_key=str(public),
                                                   sums=str(sums)), out=io.StringIO())
        sums.write_text(sums.read_text() + "\n")
        with self.assertRaises(SystemExit):
            signing.command_verify(SimpleNamespace(version="0.5.1", public_key=str(public),
                                                   sums=str(sums)), out=io.StringIO())

    def test_encrypted_key_needs_its_password(self):
        seed, key_id = b"\x01" * 32, b"\x02" * 8
        # Small limits keep the test fast; the format is the same as with the defaults.
        text = signing.secret_key_text(seed, key_id, b"correct horse", 524288, 16777216)
        self.assertEqual(signing.read_secret_key(text, lambda: b"correct horse"), (seed, key_id))
        with self.assertRaises(ValueError):
            signing.read_secret_key(text, lambda: b"wrong")

    def test_never_overwrites_a_key(self):
        secret, _ = self.generate(password=False)
        before = secret.read_bytes()
        with self.assertRaises(SystemExit):
            self.generate(password=False)
        self.assertEqual(secret.read_bytes(), before)

    def test_refuses_a_secret_key_inside_the_repository(self):
        inside = signing.ROOT / "_build" / "never-written.key"
        with self.assertRaises(SystemExit):
            signing.command_generate(SimpleNamespace(
                secret_key=str(inside), public_key=str(self.folder / "x.pub"), no_password=True),
                out=io.StringIO())
        self.assertFalse(inside.exists())

    def test_refuses_to_replace_a_real_public_key(self):
        _, public = self.generate(password=False)
        with self.assertRaises(SystemExit):
            signing.command_generate(SimpleNamespace(
                secret_key=str(self.folder / "second.key"), public_key=str(public),
                no_password=True), out=io.StringIO())

    def test_refuses_a_key_that_does_not_match_the_public_key(self):
        secret, public = self.generate(password=False)
        other_public = self.folder / "other.pub"
        other_public.write_text(signing.public_key_text(b"\x05" * 32, b"\x06" * 8))
        sums = self.folder / "SHA256SUMS"
        sums.write_text("0" * 64 + "  a.zip\n")
        with self.assertRaises(SystemExit):
            signing.command_sign(SimpleNamespace(secret_key=str(secret), version="1.0.0",
                                                 public_key=str(other_public), force=False,
                                                 sums=str(sums)), out=io.StringIO())

    def test_checksums_lists_the_archives(self):
        archive = self.folder / "midnight-explorer-1.0.0-x86_64-pc-windows-msvc.zip"
        archive.write_bytes(b"zip")
        signing.command_checksums(SimpleNamespace(archives=[str(archive)], force=False),
                                  out=io.StringIO())
        self.assertEqual((self.folder / "SHA256SUMS").read_text(),
                         hashlib.sha256(b"zip").hexdigest()
                         + "  midnight-explorer-1.0.0-x86_64-pc-windows-msvc.zip\n")


if __name__ == "__main__":
    unittest.main()
