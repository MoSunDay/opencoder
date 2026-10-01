import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.maintenance import configuration
from fixtures import Fixture


class ConfigurationTests(unittest.TestCase):
    def test_pending_configuration_is_private_and_installed_only_from_fixed_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            settings = fixture.settings
            for workdir in (settings.agent_workdir, settings.server_workdir):
                workdir.mkdir()
                (workdir / 'opencoder.json').write_text('old configuration')
            old = {'dag': {'obsolete': True}, 'api_key': 'isolated-fixture'}
            pending = Path(directory) / 'pending.json'
            pending.write_text(json.dumps({'dag': {'binary_dir': '/new/binaries'}}))
            object.__setattr__(settings, 'agent_config', pending)
            object.__setattr__(settings, 'server_config', pending)
            with patch.object(configuration, 'actual_configs', return_value=(old, old)):
                configs = configuration.desired_configs(settings)
                snapshots = configuration.freeze(settings, 'new', configuration.hashes(configs))
            self.assertEqual((settings.server_workdir / 'opencoder.json').read_text(), 'old configuration')
            for item in snapshots.values():
                path = Path(item['path'])
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
                self.assertNotIn('obsolete', json.loads(path.read_text())['dag'])
            pending.write_text('changed after acceptance')
            configuration.install(settings, snapshots)
            for workdir in (settings.agent_workdir, settings.server_workdir):
                value = json.loads((workdir / 'opencoder.json').read_text())
                self.assertEqual(value['dag']['binary_dir'], '/new/binaries')
                self.assertEqual(value['api_key'], 'isolated-fixture')
            Path(snapshots['server']['path']).write_text('tampered')
            (settings.agent_workdir / 'opencoder.json').write_text('unchanged on checksum failure')
            with self.assertRaisesRegex(ValueError, 'checksum differs'):
                configuration.install(settings, snapshots)
            self.assertEqual((settings.agent_workdir / 'opencoder.json').read_text(), 'unchanged on checksum failure')
