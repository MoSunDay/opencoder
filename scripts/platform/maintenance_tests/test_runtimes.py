import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from fixtures import Fixture
from rolling.maintenance import runtimes


class RuntimeTests(unittest.TestCase):
    def test_idle_inventory_is_cached_without_changing_the_live_host(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            saved = runtimes.capture(fixture.settings, fixture)
            path = fixture.settings.state_dir / 'host/host.db'
            with sqlite3.connect(path) as conn:
                self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('active',))
                self.assertEqual(conn.execute('SELECT count(*) FROM fleet_definitions').fetchone(), (0,))
            runtimes.hibernate_stopped(fixture.settings, saved)
            runtimes.hibernate_stopped(fixture.settings, saved)
            with sqlite3.connect(path) as conn:
                self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('retired',))
                self.assertEqual(json.loads(conn.execute('SELECT body FROM fleet_definitions').fetchone()[0]), saved['old'])
            fixture.http = lambda *args: self.fail('a sleeping Runtime must not be contacted')
            self.assertEqual(runtimes.capture(fixture.settings, fixture), saved)

    def test_live_runtime_and_reserved_capacity_prevent_hibernation(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            fixture.http = lambda *args: {'runtime_id': 'old', 'can_hibernate': False, 'owned_processes': 1}
            with self.assertRaisesRegex(ValueError, 'not idle'):
                runtimes.capture(fixture.settings, fixture)
            path = fixture.settings.state_dir / 'host/host.db'
            with sqlite3.connect(path) as conn:
                conn.execute("INSERT INTO capacity_queue VALUES ('running')")
            with self.assertRaisesRegex(ValueError, 'reservations'):
                runtimes.hibernate_stopped(fixture.settings, {'old': {}})
            with sqlite3.connect(path) as conn:
                self.assertEqual(conn.execute('SELECT mode FROM host_runtimes').fetchone(), ('active',))
