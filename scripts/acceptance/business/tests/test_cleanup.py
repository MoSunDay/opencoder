"""Runtime finalization preserves reports and every database in and outside the run."""
from pathlib import Path
from contextlib import closing
import sqlite3
import json
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from lifecycle.runtime import initialize_runtime, finalize_runtime
from lifecycle.process import record_fixture, stop_fixture


class RuntimeCleanup(unittest.TestCase):
    def test_explicit_disposal_removes_only_owned_runtime(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / 'run'
            (root / 'evidence').mkdir(parents=True)
            initialize_runtime(root)
            existing = base / 'existing.db'
            existing.write_text('untouched')
            (root / 'runtime/task.db').write_text('temporary')
            (root / 'evidence/report.json').write_text('retained')
            self.assertTrue(finalize_runtime(root, destroy=True)['runtimeRemoved'])
            self.assertFalse((root / 'runtime').exists())
            self.assertEqual(existing.read_text(), 'untouched')
            self.assertEqual((root / 'evidence/report.json').read_text(), 'retained')

    def test_fixture_reattachment_checks_start_time_before_stopping(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
            try:
                record_fixture(root, child)
                marker = root / 'evidence/fixture-process.json'
                owner = json.loads(marker.read_text())
                marker.write_text(json.dumps({**owner, 'startTime': 'different'}))
                stop_fixture(root)
                self.assertIsNone(child.poll())
                marker.write_text(json.dumps(owner))
                stop_fixture(root)
                self.assertIsNotNone(child.poll())
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait()

    def test_all_databases_and_reports_survive_finalization(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            root = base / 'run'
            (root / 'evidence').mkdir(parents=True)
            initialize_runtime(root)
            existing = base / 'existing.db'
            for file in [existing, root / 'runtime/task.db']:
                with closing(sqlite3.connect(file)) as db:
                    db.execute('create table receipt (value text)')
                    db.execute('insert into receipt values (?)', ['keep'])
                    db.commit()
            report = root / 'evidence/report.json'
            report.write_text('{"passed":true}')
            result = finalize_runtime(root)
            self.assertFalse(result['runtimeRemoved'])
            self.assertTrue((root / 'runtime/task.db').exists())
            self.assertEqual(report.read_text(), '{"passed":true}')
            with closing(sqlite3.connect(existing)) as db:
                self.assertEqual(db.execute('select value from receipt').fetchall(), [('keep',)])
            self.assertTrue(finalize_runtime(root)['runtimeRetained'])

    def test_unowned_runtime_and_symlink_replacement_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'run'
            (root / 'evidence').mkdir(parents=True)
            (root / 'runtime').mkdir()
            with self.assertRaisesRegex(RuntimeError, 'ownership record missing'):
                finalize_runtime(root)
            (root / 'runtime').rmdir()
            initialize_runtime(root)
            saved = root / 'original-runtime'
            (root / 'runtime').rename(saved)
            (saved / 'existing.db').write_text('must remain')
            (root / 'runtime').symlink_to(saved, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, 'ownership mismatch'):
                finalize_runtime(root)
            self.assertEqual((saved / 'existing.db').read_text(), 'must remain')

    def test_replaced_directory_and_completed_run_reuse_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'evidence').mkdir()
            initialize_runtime(root)
            (root / 'runtime').rename(root / 'saved')
            (root / 'runtime').mkdir()
            with self.assertRaisesRegex(RuntimeError, 'identity changed'):
                finalize_runtime(root)
            with self.assertRaisesRegex(RuntimeError, 'cannot be overwritten'):
                initialize_runtime(root)

    def test_open_file_prevents_removal_even_with_an_outside_working_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'evidence').mkdir()
            initialize_runtime(root)
            file = root / 'runtime/held.db'
            file.write_text('held')
            with file.open():
                with self.assertRaisesRegex(RuntimeError, 'still referenced'):
                    finalize_runtime(root)
                self.assertTrue(file.exists())
            self.assertTrue(finalize_runtime(root)['runtimeRetained'])


if __name__ == '__main__':
    unittest.main()
