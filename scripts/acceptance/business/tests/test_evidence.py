"""Evidence retries merge receipts without reading sandbox credentials."""
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lifecycle.evidence import copy_evidence


class EvidenceCopy(unittest.TestCase):
    def test_repeated_collection_merges_nested_receipts_and_excludes_private_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / 'source', root / 'target'
            (source / 'tests').mkdir(parents=True)
            (source / 'tests/result.log').write_text('first')
            (source / 'tests/codex-home').mkdir()
            (source / 'tests/codex-home/auth.json').write_text('private fixture')
            copy_evidence(source, target)
            (source / 'tests/result.log').write_text('completed')
            copy_evidence(source, target)
            self.assertEqual((target / 'tests/result.log').read_text(), 'completed')
            self.assertFalse((target / 'tests/codex-home').exists())

    def test_symlinks_cannot_copy_private_files_or_overwrite_outside_evidence(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source, target = root / 'source', root / 'target'
            source.mkdir()
            target.mkdir()
            private = root / 'private'
            private.write_text('unchanged')
            (source / 'receipt').symlink_to(private)
            with self.assertRaisesRegex(RuntimeError, 'symlinks'):
                copy_evidence(source, target)
            (source / 'receipt').unlink()
            (source / 'receipt').write_text('new')
            (target / 'receipt').symlink_to(private)
            with self.assertRaisesRegex(RuntimeError, 'symlinks'):
                copy_evidence(source, target)
            self.assertEqual(private.read_text(), 'unchanged')


if __name__ == '__main__':
    unittest.main()
