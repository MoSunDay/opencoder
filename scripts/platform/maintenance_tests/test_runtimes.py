import copy
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import Mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import runtimes


class RuntimeTests(unittest.TestCase):
    def test_interrupted_conversation_is_retained_only_after_all_work_has_stopped(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.http('', '/inventory')
            for kind in ('agent', 'operator'):
                paused = {**copy.deepcopy(original), 'indexes': [
                    {'id': 'awaiting-user', 'kind': kind, 'status': 'interrupted'}]}
                expected = copy.deepcopy(paused)
                self.assertEqual(runtimes.idle('old', 'node', paused), expected)
                for patch in ({'can_hibernate': False}, {'owned_processes': 1},
                              {'snapshot': {**paused['snapshot'], 'pending_runs': 1}},
                              {'snapshot': {**paused['snapshot'], 'active_runs': 1}},
                              {'snapshot': {**paused['snapshot'], 'active_agent_loops': 1}}):
                    with self.subTest(kind=kind, patch=patch), self.assertRaises(ValueError):
                        runtimes.idle('old', 'node', {**paused, **patch})
                self.assertEqual(paused, expected)
            for kind in ('brain', 'dag', 'todos', 'team', 'project'):
                with self.subTest(kind=kind), self.assertRaises(ValueError):
                    runtimes.idle('old', 'node', {**original, 'indexes': [
                        {'id': 'unfinished', 'kind': kind, 'status': 'interrupted'}]})

    def test_final_inventory_retires_old_runtime_and_survives_repeated_install(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            saved = runtimes.capture(fixture.settings, fixture)
            path = fixture.settings.state_dir / 'host/host.db'
            with sqlite3.connect(path) as conn:
                self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('active',))
            runtimes.install(fixture.settings, saved)
            runtimes.install(fixture.settings, saved)
            with sqlite3.connect(path) as conn:
                self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('retired',))
                self.assertEqual(conn.execute('SELECT count(*) FROM fleet_definitions').fetchone(), (1,))
            offline = Mock()
            offline.http.side_effect = AssertionError('already hibernated Runtime must stay stopped')
            self.assertEqual(runtimes.capture(fixture.settings, offline), saved)

    def test_busy_ledger_or_changed_registration_preserves_live_host_state(self):
        for conflict in ('queued', 'registration'):
            with self.subTest(conflict=conflict), tempfile.TemporaryDirectory() as directory:
                fixture = Fixture(Path(directory))
                saved = runtimes.capture(fixture.settings, fixture)
                path = fixture.settings.state_dir / 'host/host.db'
                with sqlite3.connect(path) as conn:
                    if conflict == 'queued':
                        conn.execute("INSERT INTO capacity_queue VALUES ('accepted','queued')")
                    else:
                        saved['old']['config'] = '{}'
                with self.assertRaises(ValueError):
                    runtimes.install(fixture.settings, saved)
                with sqlite3.connect(path) as conn:
                    self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('active',))
                    self.assertEqual(conn.execute('SELECT count(*) FROM fleet_definitions').fetchone(), (0,))

    def test_unfinished_work_processes_and_identity_are_rejected_before_stop(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            good = fixture.http('', '/inventory')
            self.assertEqual(runtimes.idle('old', 'node', {**good, 'indexes': [
                {'id': 'completed-turn', 'kind': 'operator', 'status': 'idle'}]})['runtime_id'], 'old')
            for patch in [{'can_hibernate': False}, {'owned_processes': 1}, {'runtime_id': 'other'},
                          {'indexes': [{'id': 'accepted', 'status': 'pending'}]},
                          {'snapshot': {**good['snapshot'], 'pending_runs': 1}}]:
                with self.subTest(patch=patch), self.assertRaises(ValueError):
                    runtimes.idle('old', 'node', {**copy.deepcopy(good), **patch})
