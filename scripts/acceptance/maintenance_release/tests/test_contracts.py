"""CLI choices and preservation assertions without real services."""
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import main
from evidence import source_inventory, unchanged_source, authentication
from inputs import debug_manifest, package_debug, NAMES
from rolling import manifest
from rolling.maintenance import configuration
from fixture import configuration_scope, create_settings


class ContractsTests(unittest.TestCase):
    def test_candidate_choice_is_required_and_exclusive(self):
        required = ['--rootfs', '/image', '--old-bundle', '/old']
        with patch('sys.stderr'), self.assertRaises(SystemExit):
            main.arguments(required)
        with patch('sys.stderr'), self.assertRaises(SystemExit):
            main.arguments([*required, '--bin-dir', '/bin', '--platform-bundle', '/bundle'])
        args = main.arguments([*required, '--bin-dir', '/bin'])
        self.assertEqual(args.bin_dir, Path('/bin'))
        self.assertIsNone(args.platform_bundle)

    def test_source_inventory_rejects_changed_bytes_and_same_bytes_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            path = source / 'file'
            path.write_text('original')
            expected = source_inventory(source)
            self.assertTrue(unchanged_source(source, expected))
            path.write_text('changed')
            with self.assertRaisesRegex(AssertionError, 'source workspace'):
                unchanged_source(source, expected)
            path.write_text('original')
            replacement = root / 'replacement'
            replacement.write_text('original')
            replacement.replace(path)
            with self.assertRaisesRegex(AssertionError, 'identity'):
                unchanged_source(source, expected)

    def test_authentication_fingerprint_has_no_raw_rows_and_reads_only(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'definitions.db'
            with sqlite3.connect(path) as connection:
                connection.execute('CREATE TABLE platform_users (name TEXT, token_hash BLOB)')
                connection.execute('INSERT INTO platform_users VALUES (?, ?)', ('fixture', b'private-bytes'))
            before = path.read_bytes()
            result = authentication(path)
            self.assertEqual(result['platform_users']['rows'], 1)
            self.assertNotIn('private-bytes', json.dumps(result))
            self.assertNotIn('fixture', json.dumps(result))
            self.assertEqual(path.read_bytes(), before)

    def test_actual_and_desired_configs_use_private_home_and_real_overlay(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = create_settings(root)
            (settings.agent_workdir / 'opencoder.json').write_text('{"dag":{"old":1}}')
            (settings.server_workdir / 'opencoder.json').write_text('{"storage":{"backend":"libsql"}}')
            settings.agent_config.write_text('{"dag":{"workspace_dir":"/private"}}')
            settings.server_config.write_text('{}')
            with configuration_scope(settings, root):
                actual = configuration.actual_configs(settings)
                desired = configuration.desired_configs(settings)
            self.assertEqual(actual[0]['dag'], {'old': 1})
            self.assertEqual(desired[0]['dag'], {'workspace_dir': '/private'})
            self.assertEqual(desired[1], actual[1])

    def test_debug_manifest_preserves_dirty_and_unknown_provenance(self):
        info = {'git_commit': 'a' * 40, 'version': 'test', 'version_long': 'test-dirty',
                'protocol_version': 10, 'brain_schema_version': 7, 'spa_sha256': 'unknown',
                'release_compatibility': {'data_format': {'min': 2, 'max': 2}}}
        result = debug_manifest({}, info)
        self.assertEqual(result['spa_sha256'], 'unknown')
        self.assertEqual(result['version_long'], 'test-dirty')
        self.assertTrue(result['release_id'].startswith('debug-'))

    def test_debug_packaging_preserves_strict_unknown_and_dirty_rejections(self):
        for spa, expected in [('unknown', 'invalid spa_sha256'), ('b' * 64, 'metadata does not match')]:
            with self.subTest(spa=spa), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                binaries = root / 'bin'
                binaries.mkdir()
                for name in NAMES:
                    (binaries / name).write_bytes(b'fixture-binary')
                info = {'git_commit': 'a' * 40, 'git_dirty': True, 'version': 'test', 'version_long': 'dirty',
                        'protocol_version': 10, 'brain_schema_version': 7, 'spa_sha256': spa,
                        'release_compatibility': {'data_format': {'min': 2, 'max': 2},
                                                  'protocol': {'min': 1, 'max': 1}}}
                with patch.object(manifest._installer, 'build_info', return_value=info):
                    package_debug(binaries, root / 'candidate')
                    with self.assertRaisesRegex(manifest._installer.InstallError, expected):
                        manifest.verify(root / 'candidate')


if __name__ == '__main__':
    unittest.main()
