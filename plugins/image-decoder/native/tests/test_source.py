"""Native source preparation refuses wrong bytes and unowned generated paths."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'prepare-source.py'
spec = importlib.util.spec_from_file_location('dav1d_source', SCRIPT)
source = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source)


class SourcePreparationTests(unittest.TestCase):
    def setUp(self):
        generated = SCRIPT.parents[3] / '_build/tmp'
        generated.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=generated)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def test_corrupt_cached_archive_fails_before_extraction(self):
        (self.root / f'dav1d-{source.VERSION}.tar.xz').write_bytes(b'wrong archive')
        with self.assertRaisesRegex(ValueError, 'checksum mismatch'):
            source.prepare(self.root, offline=True)
        self.assertFalse((self.root / f'dav1d-{source.VERSION}').exists())
        self.assertFalse((self.root / 'source.json').exists())

    def test_offline_missing_archive_does_not_fetch_or_extract(self):
        with self.assertRaisesRegex(ValueError, 'Offline source archive is missing'):
            source.prepare(self.root, offline=True)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_output_outside_build_is_rejected_before_creation(self):
        outside = SCRIPT.parent / 'unexpected-native-output'
        self.assertFalse(outside.exists())
        with self.assertRaisesRegex(ValueError, 'under an ordinary _build'):
            source.build_directory(outside)
        self.assertFalse(outside.exists())


if __name__ == '__main__':
    unittest.main()
