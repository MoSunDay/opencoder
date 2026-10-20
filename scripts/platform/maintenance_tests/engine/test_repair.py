import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.config import Settings
from rolling.maintenance.engine import repair
from rolling.state import Journal
from rolling.units import service


class SleepingRuntimeRepairTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.settings = Settings(root / 'state', root / 'token', systemd_dir=root / 'units')
        self.settings.systemd_dir.mkdir()
        host = self.settings.state_dir / 'host'
        (host / 'host.locks').mkdir(parents=True)
        (host / 'node-id').write_text('node-test')
        self.data = self.settings.state_dir / 'runtimes/retained'
        self.image = self.data / 'dag/rootfs'
        (self.image / 'usr/bin').mkdir(parents=True)
        self.bundle = self.settings.state_dir / 'releases/current/bundle'
        (self.bundle / 'bin').mkdir(parents=True)
        for name in ('dag-runner', 'agent-step-runner', 'agent-session-runner'):
            (self.image / 'usr/bin' / name).write_text('original')
            (self.bundle / 'bin' / name).write_text('replacement')
        workdir = self.data / 'workdir'
        workdir.mkdir()
        self.config = workdir / 'opencoder.json'
        self.config.write_text(json.dumps({'dag': {'rootfs_dir': str(self.image)}}))
        self.record = {'id': 'retained', 'runtime_data': str(self.data),
                       'runtime_unit': 'retained.service', 'runtime_port': 4001,
                       'manifest': {'release_id': 'retained'}}
        self.unit = self.settings.systemd_dir / 'retained.service'
        self.unit.write_text(service(['/releases/old/bin/opencoder-agent', '--workdir', str(workdir),
                                     '--data-dir', str(self.data), 'runtime', '--port', '4001'],
                                    'retained', runtime=True))
        self.original_unit = self.unit.read_bytes()
        self.original_config = self.config.read_bytes()
        self.manifest = {'release_id': 'current'}
        journal = Journal(self.settings.state_dir)
        journal.data.update(current='current', phase='complete', candidate=None,
                            releases={'retained': self.record, 'current': {'manifest': self.manifest}})
        journal.save()
        self.database = host / 'host.db'
        inventory = {'runtime_id': 'retained', 'registration': {'id': 'node-test'},
                     'can_hibernate': True, 'owned_processes': 0, 'indexes': [],
                     'snapshot': {'active_runs': 0, 'pending_runs': 0, 'active_agent_loops': 0}}
        config = {'unit': 'retained.service', 'data_dir': str(self.data),
                  'endpoint': 'http://127.0.0.1:4001'}
        with sqlite3.connect(self.database) as conn:
            conn.executescript(
                'CREATE TABLE host_runtimes(id,mode,config);'
                'CREATE TABLE fleet_definitions(kind,id,body);'
                'CREATE TABLE capacity_queue(ticket,runtime_id,phase);')
            conn.execute('INSERT INTO host_runtimes VALUES (?,?,?)',
                         ('retained', 'retired', json.dumps(config)))
            conn.execute('INSERT INTO fleet_definitions VALUES (?,?,?)',
                         ('runtime_sleep', 'retained', json.dumps(inventory)))
        self.ops = Mock()
        self.ops.output.return_value = 'ActiveState=inactive\nMainPID=0\n'
        for target, kwargs in [
            ('rolling.maintenance.engine.repair.manifest.verify', {'return_value': self.manifest}),
            ('rolling.maintenance.engine.repair.manifest.compatible', {}),
            ('rolling.maintenance.engine.repair.manifest._installer.build_info', {'return_value': {}}),
            ('rolling.maintenance.engine.repair.programs.verify_image', {}),
            ('rolling.maintenance.recovery.capacity.kernel_owners', {'return_value': []}),
        ]:
            context = patch(target, **kwargs)
            context.start()
            self.addCleanup(context.stop)

    def run_repair(self):
        return repair.repair(self.settings, self.ops, 'retained', 'current')

    def test_installs_only_sleeping_program_and_replays_without_restarting(self):
        result = self.run_repair()
        self.assertEqual(result['phase'], 'installed')
        self.assertFalse(result['runtime_started'])
        self.assertEqual(self.ops.run.call_args.args, ('systemctl', 'daemon-reload'))
        self.assertEqual(self.config.read_bytes(), self.original_config)
        self.assertEqual((self.image / 'usr/bin/dag-runner').read_text(), 'original')
        record = Journal(self.settings.state_dir).record('retained')
        self.assertEqual({k: record[k] for k in self.record}, self.record)
        self.assertEqual(record['runtime_program']['release_id'], 'current')
        self.assertTrue(self.run_repair()['replayed'])
        self.ops.run.assert_called_once()

    def test_live_reservation_and_missing_idle_proof_are_rejected(self):
        with sqlite3.connect(self.database) as conn:
            conn.execute("INSERT INTO capacity_queue VALUES ('ticket','retained','running')")
        with self.assertRaisesRegex(ValueError, 'reservation'):
            self.run_repair()
        with sqlite3.connect(self.database) as conn:
            conn.execute('DELETE FROM capacity_queue')
            conn.execute("UPDATE fleet_definitions SET body='null'")
        with self.assertRaises(ValueError):
            self.run_repair()
        self.assertEqual(self.unit.read_bytes(), self.original_unit)
        self.ops.run.assert_not_called()

    def test_host_wake_lock_prevents_unit_replacement(self):
        path = repair.host_lock(self.settings, 'runtime-use', 'retained')
        with repair.exclusive(path):
            with self.assertRaises(BlockingIOError):
                self.run_repair()
        self.assertEqual(self.unit.read_bytes(), self.original_unit)
        self.assertEqual(self.run_repair()['phase'], 'installed')

    def test_registration_change_after_staging_is_rejected(self):
        original = repair.programs.stage_one

        def race(*args):
            staged = original(*args)
            with sqlite3.connect(self.database) as conn:
                conn.execute("UPDATE host_runtimes SET mode='active'")
            return staged

        with patch.object(repair.programs, 'stage_one', side_effect=race):
            with self.assertRaises(ValueError):
                self.run_repair()
        self.assertEqual(self.unit.read_bytes(), self.original_unit)
        self.ops.run.assert_not_called()

    def test_reload_failure_restores_prior_program_then_same_receipt_can_retry(self):
        self.ops.run.side_effect = [RuntimeError('reload failed'), None]
        with self.assertRaisesRegex(RuntimeError, 'reload failed'):
            self.run_repair()
        self.assertEqual(self.unit.read_bytes(), self.original_unit)
        self.assertEqual(Journal(self.settings.state_dir).record('retained'), self.record)
        self.ops.run.side_effect = None
        self.assertEqual(self.run_repair()['phase'], 'installed')

    def test_changed_backup_or_unit_is_not_overwritten_on_retry(self):
        self.ops.run.side_effect = [RuntimeError('reload failed'), None]
        with self.assertRaises(RuntimeError):
            self.run_repair()
        self.ops.run.side_effect = None
        self.unit.write_text('concurrent replacement')
        with self.assertRaisesRegex(ValueError, 'unit changed'):
            self.run_repair()
        self.assertEqual(self.unit.read_text(), 'concurrent replacement')


if __name__ == '__main__':
    unittest.main()
