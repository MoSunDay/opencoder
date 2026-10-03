import json
import os
from pathlib import Path
import socket
import stat
import sys
import tempfile
import unittest
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling import backup
from rolling.backup_files import tree
from rolling.maintenance import archive, restore
from rolling.maintenance.planning import capacity
from rolling.state import write


class BackupTreeTests(unittest.TestCase):
    @unittest.skipUnless(os.geteuid() == 0, 'device snapshots require root')
    def test_devices_and_pipes_copy_restore_and_verify_without_reading_streams(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            (source / 'answer').write_bytes(b'kept')
            (source / 'answer-link').symlink_to('answer')
            os.mknod(source / 'full', stat.S_IFCHR | 0o600, os.makedev(1, 7))
            os.mknod(source / 'block', stat.S_IFBLK | 0o600, os.makedev(7, 255))
            os.mkfifo(source / 'pipe', 0o600)
            saved = root / 'saved'
            tree.copy_tree(source, saved)
            self.assertEqual(tree.inventory(saved), tree.inventory(source))
            self.assertTrue(stat.S_ISCHR((saved / 'full').lstat().st_mode))
            self.assertEqual((saved / 'full').lstat().st_size, 0)
            self.assertEqual(capacity.tree_bytes(source), 4)
            self.assertEqual(capacity.tree_bytes(source, block_size=4096), 6 * 4096)
            restored = root / 'restored'
            restore.replace(saved, restored)
            restore.replace(saved, restored)
            self.assertEqual(tree.inventory(restored), tree.inventory(source))
            write(saved / 'manifest.json', {'files': archive.inventory(saved)})
            (saved / 'manifest.sha256').write_text(archive.digest(saved / 'manifest.json') + '\n')
            archive.verify(saved)
            (saved / 'full').unlink()
            os.mknod(saved / 'full', stat.S_IFCHR | 0o600, os.makedev(1, 3))
            with self.assertRaisesRegex(ValueError, 'checksum or file set'):
                archive.verify(saved)

    @unittest.skipUnless(os.geteuid() == 0, 'device snapshots require root')
    def test_stopped_snapshot_records_device_identity_and_retains_regular_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root / 'server'
            server.mkdir()
            (server / 'data').write_bytes(b'original')
            os.mknod(server / 'full', stat.S_IFCHR | 0o600, os.makedev(1, 7))
            settings = SimpleNamespace(server_data=server, state_dir=root / 'state', legacy_agent_data=None)
            output = root / 'backup'
            backup.snapshot(settings, output, stopped=True)
            manifest = json.loads((output / 'backup-manifest.json').read_text())
            self.assertEqual(manifest['files']['server/full'], {'kind': 'character', 'mode': 0o600, 'device': os.makedev(1, 7)})
            self.assertEqual((output / 'server/data').read_bytes(), b'original')
            self.assertEqual((output / 'server/full').lstat().st_size, 0)

    def test_socket_is_refused_by_preflight_and_inventory(self):
        with tempfile.TemporaryDirectory() as directory, socket.socket(socket.AF_UNIX) as listener:
            root = Path(directory)
            listener.bind(str(root / 'live.sock'))
            for call in (capacity.tree_bytes, tree.inventory):
                with self.assertRaisesRegex(ValueError, 'unsupported backup filesystem entry'):
                    call(root)


if __name__ == '__main__':
    unittest.main()
