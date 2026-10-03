from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from fixtures import Fixture
from rolling.maintenance import flow


class ScopeTests(unittest.TestCase):
    def test_offline_registration_survives_local_maintenance(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.http

            def http(base, path, method='GET', body=None):
                if path == '/api/nodes':
                    return {'nodes': [{'id': 'node', 'online': True},
                                      {'id': 'historical', 'online': False}]}
                return original(base, path, method, body)

            fixture.http = http
            with fixture.patches():
                self.assertEqual(flow.deploy(fixture.settings, fixture.bundle, fixture)['phase'], 'complete')
            self.assertFalse(any(call[1:3] == ('/api/nodes/historical', 'DELETE') for call in fixture.calls))

    def test_online_unmanaged_node_rejected_before_gate_closure(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            original = fixture.http

            def http(base, path, method='GET', body=None):
                if path == '/api/nodes':
                    return {'nodes': [{'id': 'node', 'online': True},
                                      {'id': 'external', 'online': True}]}
                return original(base, path, method, body)

            fixture.http = http
            with fixture.patches(), self.assertRaisesRegex(ValueError, 'remote writers'):
                flow.deploy(fixture.settings, fixture.bundle, fixture)
            self.assertEqual(fixture.settings.nginx_include.read_text(), 'old ingress\n')
            self.assertFalse(any(call[:2] == ('systemctl', 'stop') for call in fixture.calls))


if __name__ == '__main__':
    unittest.main()
