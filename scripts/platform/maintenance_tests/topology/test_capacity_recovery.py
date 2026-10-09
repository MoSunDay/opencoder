from pathlib import Path
import fcntl
import hashlib
import json
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from fixtures import Fixture
from rolling.maintenance.recovery import capacity


class RecoveryTests(unittest.TestCase):
    def fixture(self, root):
        f = Fixture(root)
        runtime = f.settings.state_dir / 'runtimes/old'
        runtime.mkdir(parents=True)
        (runtime / 'node-id').write_text('node')
        (runtime / 'node.lock').touch()
        (runtime / 'admission.json').write_text('{"version":1,"mode":"open"}')
        host = f.settings.state_dir / 'host'
        (host / 'host.locks').mkdir()
        (runtime / 'host-binding.json').write_text(json.dumps({'database': str(host / 'host.db'), 'runtime_id': 'old'}))
        with sqlite3.connect(host / 'host.db') as conn:
            conn.execute('UPDATE host_runtimes SET config=?,mode=? WHERE id=?',
                         (json.dumps({'data_dir': str(runtime), 'unit': 'opencoder-runtime-old.service'}), 'retired', 'old'))
            conn.execute('DROP TABLE capacity_queue')
            conn.executescript("CREATE TABLE capacity_queue(ticket TEXT PRIMARY KEY,runtime_id TEXT,execution_id TEXT,phase TEXT);"
                               "INSERT INTO capacity_queue VALUES ('ticket','old','agent-task','running');"
                               "CREATE TABLE platform_users(token_hash TEXT);INSERT INTO platform_users VALUES ('unchanged');")
        f.active.discard('opencoder-runtime-old.service')
        original = f.output
        f.output = lambda *args: 'ActiveState=inactive\nMainPID=0\n' if args[0] == 'systemctl' else original(*args)
        return f, runtime

    def phase(self, f):
        with sqlite3.connect(f.settings.state_dir / 'host/host.db') as conn:
            return conn.execute('SELECT phase FROM capacity_queue').fetchone()[0]

    def test_exact_retry_keeps_backup_and_authentication_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            f, _ = self.fixture(Path(directory))
            with patch.object(capacity, 'kernel_owners', return_value=[]):
                receipt = capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
                backup = Path(receipt['backup'])
                digest = hashlib.sha256(backup.read_bytes()).hexdigest()
                capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
            self.assertEqual(self.phase(f), 'done')
            self.assertEqual(hashlib.sha256(backup.read_bytes()).hexdigest(), digest)
            self.assertEqual(json.loads((f.settings.state_dir / 'runtimes/old/admission.json').read_text()),
                             {'version': 1, 'mode': 'frozen'})
            self.assertEqual(json.loads((backup.parent / 'runtime-admission-before.json').read_text()),
                             {'version': 1, 'mode': 'open'})
            with sqlite3.connect(backup) as conn:
                self.assertEqual(conn.execute('SELECT phase FROM capacity_queue').fetchone(), ('running',))
            with sqlite3.connect(f.settings.state_dir / 'host/host.db') as conn:
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), ('unchanged',))

    def test_live_kernel_owner_and_node_lock_prevent_release(self):
        with tempfile.TemporaryDirectory() as directory:
            f, runtime = self.fixture(Path(directory))
            with patch.object(capacity, 'kernel_owners', return_value=[123]), self.assertRaisesRegex(ValueError, 'kernel processes'):
                capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
            with (runtime / 'node.lock').open('r+') as owner:
                fcntl.flock(owner, fcntl.LOCK_EX | fcntl.LOCK_NB)
                with patch.object(capacity, 'kernel_owners', return_value=[]), self.assertRaises(BlockingIOError):
                    capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
            self.assertEqual(self.phase(f), 'running')

    def test_active_service_or_wrong_identity_cannot_release_another_reservation(self):
        with tempfile.TemporaryDirectory() as directory:
            f, _ = self.fixture(Path(directory))
            with patch.object(capacity, 'kernel_owners', return_value=[]):
                original = f.output
                f.output = lambda *args: 'ActiveState=active\nMainPID=100\n'
                with self.assertRaisesRegex(ValueError, 'must be stopped'):
                    capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
                f.output = original
                with self.assertRaisesRegex(ValueError, 'identity or phase'):
                    capacity.recover(f.settings, f, 'old', 'another-task', 'ticket')
            self.assertEqual(self.phase(f), 'running')

    def test_lost_completion_receipt_resumes_without_replacing_anchor(self):
        with tempfile.TemporaryDirectory() as directory:
            f, _ = self.fixture(Path(directory))
            original = capacity.write

            def crash(path, receipt):
                if receipt.get('stage') == 'released':
                    raise RuntimeError('completion receipt lost')
                original(path, receipt)

            with patch.object(capacity, 'kernel_owners', return_value=[]):
                with patch.object(capacity, 'write', side_effect=crash), self.assertRaisesRegex(RuntimeError, 'receipt lost'):
                    capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
                self.assertEqual(self.phase(f), 'done')
                receipt = capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
                self.assertEqual(receipt['stage'], 'released')

    def test_single_orphan_logical_run_recovers_but_another_reservation_rejects(self):
        with tempfile.TemporaryDirectory() as directory:
            f, runtime = self.fixture(Path(directory))
            original = f.http

            def status(*args, **kwargs):
                reply = original(*args, **kwargs)
                if args[1] == '/api/admin/drain':
                    reply['nodes'][0]['body']['active_runs'] = 1
                return reply

            f.http = status
            with sqlite3.connect(f.settings.state_dir / 'host/host.db') as conn:
                conn.execute("INSERT INTO capacity_queue VALUES ('another','other-runtime','other-task','queued')")
            with patch.object(capacity, 'kernel_owners', return_value=[]), self.assertRaisesRegex(ValueError, 'another reservation'):
                capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
            self.assertEqual(json.loads((runtime / 'admission.json').read_text())['mode'], 'open')
            with sqlite3.connect(f.settings.state_dir / 'host/host.db') as conn:
                conn.execute("UPDATE capacity_queue SET phase='done' WHERE ticket='another'")
            with patch.object(capacity, 'kernel_owners', return_value=[]):
                capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
            self.assertEqual(self.phase(f), 'done')

    def test_admission_backup_tampering_prevents_recovery_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            f, _ = self.fixture(Path(directory))
            with patch.object(capacity, 'kernel_owners', return_value=[]):
                receipt = capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')
                (Path(receipt['backup']).parent / 'runtime-admission-before.json').write_text('{}')
                with self.assertRaisesRegex(ValueError, 'admission backup checksum'):
                    capacity.recover(f.settings, f, 'old', 'agent-task', 'ticket')

    def test_boot_recovery_releases_multiple_runtime_reservations(self):
        with tempfile.TemporaryDirectory() as directory:
            f, old = self.fixture(Path(directory))
            root = f.settings.state_dir / 'runtimes/new'
            root.mkdir(parents=True)
            (root / 'node-id').write_text('node')
            (root / 'node.lock').touch()
            (root / 'admission.json').write_text('{"version":1,"mode":"open"}')
            host = f.settings.state_dir / 'host'
            (root / 'host-binding.json').write_text(json.dumps(
                {'database': str(host / 'host.db'), 'runtime_id': 'new'}))
            with sqlite3.connect(host / 'host.db') as conn:
                conn.execute('INSERT INTO host_runtimes VALUES (?,?,?,?)',
                             ('new', 'new', json.dumps({'data_dir': str(root),
                              'unit': 'opencoder-runtime-new.service'}), 'active'))
                conn.execute("INSERT INTO capacity_queue VALUES ('new-ticket','new','new-task','running')")
            rows = [('ticket', 'old', 'agent-task', 'running'),
                    ('new-ticket', 'new', 'new-task', 'running')]
            with patch.object(capacity, '_live_rows', return_value=rows), \
                    patch.object(capacity, 'kernel_owners', return_value=[]):
                receipt = capacity.recover_running(f.settings, f, wait_seconds=1)
            self.assertEqual(len(receipt['reservations']), 2)
            with sqlite3.connect(host / 'host.db') as conn:
                self.assertEqual(conn.execute(
                    'SELECT phase FROM capacity_queue ORDER BY ticket').fetchall(),
                    [('done',), ('done',)])


if __name__ == '__main__':
    unittest.main()
