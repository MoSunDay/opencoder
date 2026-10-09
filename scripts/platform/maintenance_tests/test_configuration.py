from pathlib import Path
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import configuration


class ConfigurationTests(unittest.TestCase):
    def test_overlay_preserves_provider_credentials_and_replaces_obsolete_dag(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'candidate.json'
            path.write_text(json.dumps({'dag': {'rootfs_dir': '/images/new'},
                                        'llm': {'model': 'new-model'}}))
            original = {'dag': {'binary_dir': '/old'},
                        'llm': {'api_key': 'fixture-private', 'model': 'old-model'}}
            result = configuration.overlay(original, path)
            self.assertEqual(result['dag'], {'rootfs_dir': '/images/new'})
            self.assertEqual(result['llm'], {'api_key': 'fixture-private', 'model': 'new-model'})
            self.assertEqual(original['dag'], {'binary_dir': '/old'})

    def test_freeze_leaves_working_configuration_unchanged_and_rejects_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            old = (fixture.settings.agent_workdir / 'opencoder.json').read_bytes()
            expected = configuration.hashes(fixture.desired)
            with patch.object(configuration, 'desired_configs', return_value=fixture.desired):
                snapshots = configuration.freeze(fixture.settings, 'new', expected)
                self.assertEqual(configuration.freeze(fixture.settings, 'new', expected), snapshots)
            self.assertEqual((fixture.settings.agent_workdir / 'opencoder.json').read_bytes(), old)
            with patch.object(configuration, 'desired_configs', return_value=({}, {})):
                with self.assertRaisesRegex(ValueError, 'changed after preflight'):
                    configuration.freeze(fixture.settings, 'new', expected)

    def test_install_verifies_every_snapshot_before_any_shared_write(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            expected = configuration.hashes(fixture.desired)
            with patch.object(configuration, 'desired_configs', return_value=fixture.desired):
                snapshots = configuration.freeze(fixture.settings, 'new', expected)
            paths = [fixture.settings.agent_workdir / 'opencoder.json',
                     fixture.settings.server_workdir / 'opencoder.json']
            before = [path.read_bytes() for path in paths]
            server_snapshot = Path(snapshots['server']['path'])
            original = server_snapshot.read_bytes()
            server_snapshot.write_bytes(b'corrupted')
            with self.assertRaisesRegex(ValueError, 'checksum differs'):
                configuration.install(fixture.settings, snapshots)
            self.assertEqual([path.read_bytes() for path in paths], before)
            server_snapshot.write_bytes(original)
            configuration.install(fixture.settings, snapshots)
            self.assertEqual([json.loads(path.read_text()) for path in paths], list(fixture.desired))
