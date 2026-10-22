"""Journal changes serialize with Server catalog writes, including crashes."""
from pathlib import Path
import fcntl
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.state import Journal


class CatalogWriterTests(unittest.TestCase):
    def test_crashed_reader_releases_lock_and_retry_preserves_lock_inode(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            journal = Journal(root)
            journal.data.update(current='old', phase='complete')
            journal.save()
            lock = root / 'catalog-writer.lock'
            inode = lock.stat().st_ino
            reader = subprocess.Popen([sys.executable, '-c',
                "import fcntl,sys,time; f=open(sys.argv[1]); "
                "fcntl.flock(f,fcntl.LOCK_SH); print('locked',flush=True); time.sleep(30)", str(lock)],
                stdout=subprocess.PIPE, text=True)
            try:
                self.assertEqual(reader.stdout.readline().strip(), 'locked')
                with lock.open('a+') as file:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(file, fcntl.LOCK_EX | fcntl.LOCK_NB)
                reader.kill()
                reader.wait(timeout=5)
                journal.data['current'] = 'new'
                journal.save()
                self.assertEqual(Journal(root).data['current'], 'new')
                journal.save()
                self.assertEqual(lock.stat().st_ino, inode)
                self.assertEqual(lock.stat().st_mode & 0o777, 0o644)
            finally:
                if reader.poll() is None:
                    reader.kill()
                    reader.wait(timeout=5)
                reader.stdout.close()

    def test_failed_save_releases_lock_and_leaves_last_committed_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            journal = Journal(root)
            journal.data.update(current='old', phase='complete')
            journal.save()
            journal.data['current'] = 'new'
            with patch('rolling.state.atomic_bytes', side_effect=OSError('interrupted')):
                with self.assertRaises(OSError):
                    journal.save()
            self.assertEqual(Journal(root).data['current'], 'old')
            journal.save()
            self.assertEqual(Journal(root).data['current'], 'new')


if __name__ == '__main__':
    unittest.main()
