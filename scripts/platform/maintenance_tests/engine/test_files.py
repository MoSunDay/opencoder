"""Stopped engine preparation preserves service access without editing data."""
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.maintenance.engine import files


class EngineFileTests(unittest.TestCase):
    @unittest.skipUnless(os.geteuid() == 0, 'different file owners require root')
    def test_checkpoint_sidecars_follow_database_owner_without_changing_content(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / 'test.db'
            database.write_bytes(b'preserved database')
            os.chown(database, 65534, 65534)
            sidecars = [Path(str(database) + suffix) for suffix in ('-wal', '-shm', '-tshm')]
            for path in sidecars:
                path.write_bytes(b'preserved journal')
            files.mark([database])
            self.assertEqual(database.read_bytes(), b'preserved database')
            for path in sidecars:
                self.assertEqual((path.stat().st_uid, path.stat().st_gid), (65534, 65534))
                self.assertEqual(path.read_bytes(), b'preserved journal')

    def test_sidecar_symlink_is_rejected_before_marking(self):
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / 'test.db'
            database.touch()
            Path(str(database) + '-tshm').symlink_to(database)
            with self.assertRaisesRegex(ValueError, 'sidecar cannot be a symlink'):
                files.mark([database])
            self.assertFalse(Path(str(database) + '.engine.json').exists())
