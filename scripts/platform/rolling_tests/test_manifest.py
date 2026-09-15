"""Storage preflight accepts duplicate local mounts and rejects unknown layers."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.manifest import resources


class ResourceTests(unittest.TestCase):
    def test_every_reported_mount_must_be_verified_local_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root, root, root, root, root / 'token', min_memory_mb=1)
            with patch('rolling.manifest.shutil.disk_usage', return_value=SimpleNamespace(free=2**40)):
                for output in ['ext4\n', 'ext4\next4\n', 'ext4\nxfs\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        resources(settings, {'files': {}})
                for output in ['', 'nfs4\n', 'ext4\nnfs4\n', 'ext4\nunknown\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        with self.assertRaisesRegex(ValueError, 'verified local storage'):
                            resources(settings, {'files': {}})


if __name__ == '__main__':
    unittest.main()
