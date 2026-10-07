from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from fixtures import Fixture
from rolling.maintenance import flow, gates, restore
from rolling.state import Journal


class IngressSwitchTests(unittest.TestCase):
    def test_close_waits_for_old_acceptors_before_stopping_any_service(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            active = set(fixture.active)
            fixture.ingress_switched = lambda workers: False
            with fixture.patches():
                with self.assertRaises(TimeoutError):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
                self.assertEqual(fixture.active, active)
                self.assertEqual(Journal(fixture.settings.state_dir).data['maintenance']['stage'], 'closing')
                self.assertFalse(any('/api/admin/drain' in call for call in fixture.calls))
                fixture.ingress_switched = lambda workers: True
                self.assertEqual(flow.deploy(fixture.settings, fixture.bundle, fixture)['phase'], 'complete')

    def test_open_and_restore_do_not_report_success_while_maintenance_workers_accept(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = Fixture(root)
            saved = root / 'saved'
            saved.mkdir()
            (saved / 'ingress').write_bytes(fixture.settings.nginx_include.read_bytes())
            metadata = {'control': [{'path': str(fixture.settings.nginx_include), 'copy': 'ingress', 'exists': True}]}
            actions = [lambda: gates.reopen(fixture.settings, fixture.old, fixture, 1),
                       lambda: restore.ingress(fixture.settings, saved, metadata, fixture, 1)]
            for action in actions:
                fixture.ingress_switched = lambda workers: False
                with self.assertRaises(TimeoutError):
                    action()
                fixture.ingress_switched = lambda workers: True
                action()
            self.assertEqual(fixture.settings.nginx_include.read_bytes(), b'old ingress\n')


if __name__ == '__main__':
    unittest.main()
