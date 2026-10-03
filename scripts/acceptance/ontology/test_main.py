import tempfile
import unittest
from pathlib import Path

from main import evidence_root


class EvidenceRootTests(unittest.TestCase):
    def test_new_external_directory_is_accepted(self):
        with tempfile.TemporaryDirectory() as parent:
            requested = Path(parent) / 'ontology'
            self.assertEqual(evidence_root(requested), requested.resolve())
            self.assertFalse(requested.exists())

    def test_existing_and_protected_directories_are_rejected(self):
        with tempfile.TemporaryDirectory() as parent:
            for requested in [Path(parent), Path('/etc/ontology-acceptance'),
                              Path('/run/ontology-acceptance'),
                              Path('/var/lib/opencoder-platform/ontology-acceptance'),
                              Path(__file__).parent / 'new-evidence']:
                with self.subTest(requested=requested):
                    with self.assertRaises(ValueError):
                        evidence_root(requested)


if __name__ == '__main__':
    unittest.main()
