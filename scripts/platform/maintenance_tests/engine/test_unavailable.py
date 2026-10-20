"""Missing retained data is never silently replaced or deleted during upgrades."""
import copy
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import runtimes
from rolling.maintenance.engine import gates, programs, unavailable


class UnavailableRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.fixture = Fixture(Path(self.directory.name))
        self.settings = self.fixture.settings
        with sqlite3.connect(self.settings.state_dir / 'host/host.db') as conn:
            row = conn.execute("SELECT config FROM host_runtimes WHERE id='old'").fetchone()
            config = json.loads(row[0])
            config['data_dir'] = self.fixture.old['runtime_data']
            conn.execute("UPDATE host_runtimes SET config=? WHERE id='old'", (json.dumps(config),))
        self.data = self.settings.state_dir / 'runtimes/missing'
        self.unit = self.settings.systemd_dir / 'opencoder-runtime-missing.service'
        self.unit.parent.mkdir(parents=True, exist_ok=True)
        self.unit.symlink_to('/dev/null')
        self.inventory = {'runtime_id': 'missing', 'registration': {'id': 'node'},
            'can_hibernate': True, 'owned_processes': 0,
            'snapshot': {'active_runs': 0, 'pending_runs': 0, 'active_agent_loops': 0},
            'indexes': [{'id': 'history', 'status': 'done', 'kind': 'dag'}]}
        self.item = {'mode': 'retired', 'config': json.dumps({'data_dir': str(self.data),
            'unit': self.unit.name, 'endpoint': 'http://127.0.0.1:4001'}),
            'sleep': json.dumps(self.inventory)}
        self.state = patch.object(unavailable.subprocess, 'check_output',
            return_value='ActiveState=inactive\nMainPID=0\n')
        self.state.start()
        self.addCleanup(self.state.stop)

    def test_only_masked_absent_terminal_runtime_is_separated_without_mutating_rows(self):
        path = self.settings.state_dir / 'host/host.db'
        with sqlite3.connect(path) as conn:
            conn.execute('INSERT INTO host_runtimes VALUES (?,?,?,?)',
                ('missing', 'missing', self.item['config'], self.item['mode']))
            conn.execute('INSERT INTO fleet_definitions VALUES (?,?,?)',
                ('runtime_sleep', 'missing', self.item['sleep']))
        before = path.read_bytes()
        self.assertEqual(set(programs.registrations(self.settings)), {'old'})
        self.assertEqual(programs.registrations(self.settings, include_unavailable=True)['missing'], self.item)
        self.assertEqual(path.read_bytes(), before)
        saved = runtimes.capture(self.settings, self.fixture)
        self.assertEqual(set(saved), {'old'})
        runtimes.install(self.settings, saved)
        with sqlite3.connect(path) as conn:
            self.assertEqual(conn.execute("SELECT config,mode FROM host_runtimes WHERE id='missing'").fetchone(),
                (self.item['config'], 'retired'))
            self.assertEqual(conn.execute("SELECT body FROM fleet_definitions WHERE id='missing'").fetchone(),
                (self.item['sleep'],))
        self.assertFalse(self.data.exists())
        self.assertEqual(self.unit.readlink(), Path('/dev/null'))
        with patch.object(gates.ingress, 'reopen'), patch.object(gates, 'runtime_call'):
            gates.reopen(self.settings, self.fixture.old, self.fixture, 1)
        self.assertFalse(self.data.exists())

    def test_complete_engine_cutover_preserves_unavailable_rows_and_mask(self):
        from test_engine import prepared
        from rolling.maintenance import flow
        with tempfile.TemporaryDirectory() as directory, prepared(Path(directory)) as (fixture, _):
            settings = fixture.settings
            unit = settings.systemd_dir / 'opencoder-runtime-missing.service'
            unit.symlink_to('/dev/null')
            data = settings.state_dir / 'runtimes/missing'
            item = {**self.item, 'config': json.dumps({'unit': unit.name,
                'data_dir': str(data), 'endpoint': 'http://127.0.0.1:4999'})}
            path = settings.state_dir / 'host/host.db'
            with sqlite3.connect(path) as conn:
                conn.execute('INSERT INTO host_runtimes VALUES (?,?,?,?)',
                    ('missing', 'missing', item['config'], item['mode']))
                conn.execute('INSERT INTO fleet_definitions VALUES (?,?,?)',
                    ('runtime_sleep', 'missing', item['sleep']))
            result = flow.deploy(settings, fixture.bundle, fixture, 1)
            self.assertEqual(result['maintenance']['stage'], 'complete')
            self.assertEqual(result['maintenance']['scope']['unavailable_registered'], {'missing': item})
            self.assertNotIn('missing', result['maintenance']['scope']['registered'])
            self.assertEqual(programs.registrations(settings, include_unavailable=True)['missing'], item)
            self.assertFalse(data.exists())
            self.assertEqual(unit.readlink(), Path('/dev/null'))

    def test_existing_data_or_unmasked_service_always_blocks_missing_release(self):
        self.data.mkdir(parents=True)
        with self.assertRaisesRegex(ValueError, 'not safely unavailable'):
            unavailable.verify(self.settings, 'missing', self.item)
        self.data.rmdir()
        self.data.symlink_to(self.settings.state_dir / 'absent')
        with self.assertRaises(ValueError):
            unavailable.verify(self.settings, 'missing', self.item)
        self.data.unlink()
        self.unit.unlink()
        self.unit.write_text('old service')
        with self.assertRaises(ValueError):
            unavailable.verify(self.settings, 'missing', self.item)

    def test_live_work_or_conversation_waiting_for_input_is_never_excluded(self):
        changes = [{'can_hibernate': False}, {'owned_processes': 1},
            {'runtime_id': 'different'}, {'registration': {'id': 'different'}},
            {'snapshot': {**self.inventory['snapshot'], 'pending_runs': 1}},
            {'indexes': [{'id': 'conversation', 'kind': 'agent', 'status': 'idle'}]},
            {'indexes': [{'id': 'pending', 'kind': 'dag', 'status': 'running'}]}]
        for change in changes:
            with self.subTest(change=change), self.assertRaises(ValueError):
                unavailable.verify(self.settings, 'missing',
                    {**self.item, 'sleep': json.dumps({**copy.deepcopy(self.inventory), **change})})
        for item in [{**self.item, 'mode': 'active'}, {**self.item, 'sleep': None}]:
            with self.assertRaises(ValueError):
                unavailable.verify(self.settings, 'missing', item)
        with patch.object(unavailable.subprocess, 'check_output', return_value='ActiveState=active\nMainPID=12\n'):
            with self.assertRaises(ValueError):
                unavailable.verify(self.settings, 'missing', self.item)


if __name__ == '__main__':
    unittest.main()
