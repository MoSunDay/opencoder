import json
from dataclasses import replace
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import load
from rolling.units import prepare_server


class AuthenticationConfigTest(unittest.TestCase):
    def test_explicit_boolean_controls_the_server_without_changing_credentials(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            token = root / 'server.token'
            token.write_text('test-credential')
            path = root / 'opencoder.json'
            raw = {'deployment': {'state_dir': str(root), 'token_file': str(token)}}
            path.write_text(json.dumps(raw))
            self.assertTrue(load(path).auth_enabled)
            raw['deployment']['auth_enabled'] = False
            path.write_text(json.dumps(raw))
            settings = replace(load(path), systemd_dir=root / 'units', server_user='root')
            record = {'id': 'test', 'server_port': 3001, 'server_unit': 'test.service'}
            (root / 'releases/test').mkdir(parents=True)
            prepare_server(settings, record)
            unit = (root / 'units/test.service').read_text()
            self.assertIn('"--auth-enabled" "false"', unit)
            self.assertIn('--token-file', unit)
            self.assertEqual(token.read_text(), 'test-credential')
            prepare_server(replace(settings, auth_enabled=True), record)
            self.assertNotIn('--auth-enabled', (root / 'units/test.service').read_text())
            for invalid in ['false', 0, None]:
                raw['deployment']['auth_enabled'] = invalid
                path.write_text(json.dumps(raw))
                with self.assertRaisesRegex(ValueError, 'boolean'):
                    load(path)


if __name__ == '__main__':
    unittest.main()
