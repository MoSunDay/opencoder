"""Dedicated service recovery and rollback protection, with isolated state."""
import hashlib
import json
import os
from pathlib import Path
import pwd
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.state import Journal, write
from rolling.todo_resources import installation, policy


class Operations:
    def __init__(self):
        self.calls = []
        self.crash = False

    def run(self, *args):
        self.calls.append(args)
        if args == ('systemctl', 'start', installation.UNIT) and self.crash:
            self.crash = False
            raise RuntimeError('interrupted after start')

    def wait(self, check, seconds):
        return check()


class TodoServiceTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.settings = Settings(self.root, self.root/'credential', server_user=pwd.getpwuid(os.getuid()).pw_name,
                                 systemd_dir=self.root/'units')
        self.settings.server_workdir.mkdir(parents=True)
        self.config_path = self.settings.server_workdir/'opencoder.json'
        write(self.config_path, {'model': 'retained'})
        self.bundle = self.root/'bundle'
        (self.bundle/'bin').mkdir(parents=True)
        (self.bundle/'bin/opencoder-server').write_bytes(b'isolated candidate artifact')
        self.candidate = {'commit': 'candidate', 'release_id': 'r2', 'todo_files_contract': 1,
            'files': {'bin/opencoder-server': {'sha256': hashlib.sha256(b'isolated candidate artifact').hexdigest()}}}
        self.ops = Operations()
        mock = patch('rolling.todo_resources.installation.probe', return_value={'ok': True})
        self.probe = mock.start(); self.addCleanup(mock.stop)
        listener = patch('rolling.todo_resources.installation.socket.socket')
        listener.start(); self.addCleanup(listener.stop)

    def config(self):
        return json.loads(self.config_path.read_text())

    def bridge(self):
        journal = Journal(self.root)
        journal.data.update(current='full', previous='bridge', phase='complete',
            releases={name: {'manifest': {'todo_files_contract': 1}} for name in ['full', 'bridge']})
        journal.save()
        return journal

    def test_install_replays_interrupted_start_without_touching_existing_exports(self):
        self.ops.crash = True
        with self.assertRaisesRegex(RuntimeError, 'interrupted'):
            installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        record_path = installation.location(self.settings)/'installation.json'
        record = json.loads(record_path.read_text())
        self.assertIsNone(record['before'])
        config = self.config(); config['model'] = 'concurrent edit'; write(self.config_path, config)
        installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        self.assertEqual(json.loads(record_path.read_text())['phase'], 'ready')
        self.assertEqual(self.config()['model'], 'concurrent edit')
        self.assertFalse(self.config()['project']['todo_storage']['uploads_enabled'])
        calls = len(self.ops.calls)
        installation.prepare(self.settings, self.bundle, {**self.candidate, 'commit': 'next'}, self.ops)
        self.assertEqual(len(self.ops.calls), calls)
        self.assertFalse(any('opencoder-resources.service' in call for call in self.ops.calls))
        self.assertNotIn('definitions.db', {p.name for p in self.root.rglob('*')})

    def test_upload_enable_requires_bridge_and_floor_survives_interrupted_config_write(self):
        installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        journal = self.bridge()
        journal.data['releases']['bridge']['manifest'] = {}; journal.save()
        with self.assertRaisesRegex(ValueError, 'bridge'):
            policy.set_uploads(self.settings, True, self.ops)
        journal.data['releases']['bridge']['manifest']['todo_files_contract'] = 1; journal.save()
        with patch('rolling.todo_resources.policy.update_config', side_effect=RuntimeError('power loss')):
            with self.assertRaisesRegex(RuntimeError, 'power loss'):
                policy.set_uploads(self.settings, True, self.ops)
        with self.assertRaisesRegex(ValueError, 'unsafe rollback'):
            policy.require_candidate(self.settings, {})
        policy.set_uploads(self.settings, True, self.ops)
        self.assertTrue(self.config()['project']['todo_storage']['uploads_enabled'])
        policy.set_uploads(self.settings, False, self.ops)
        policy.require_candidate(self.settings, self.candidate)
        with self.assertRaisesRegex(ValueError, 'unsafe rollback'):
            policy.require_candidate(self.settings, {})
        with self.assertRaisesRegex(ValueError, 'retain the service'):
            installation.restore(self.settings, self.ops)

    def test_existing_files_reject_legacy_even_if_policy_was_lost(self):
        root = self.root/'exports/todos'; root.mkdir(parents=True)
        (root/'task.bin').write_bytes(b'accepted file')
        write(self.settings.server_data/'todo-storage.json', {'files_root': str(root)})
        with self.assertRaisesRegex(ValueError, 'unsafe rollback'):
            policy.require_candidate(self.settings, {})
        policy.require_candidate(self.settings, self.candidate)

    def test_restore_replays_and_preserves_other_configuration(self):
        installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        config = self.config(); config['model'] = 'new'; write(self.config_path, config)
        installation.restore(self.settings, self.ops)
        installation.restore(self.settings, self.ops)
        self.assertEqual(self.config(), {'model': 'new'})
        self.assertFalse((self.settings.systemd_dir/installation.UNIT).exists())

    def test_config_drift_and_tampered_pinned_artifact_are_rejected(self):
        self.ops.crash = True
        with self.assertRaises(RuntimeError):
            installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        config = self.config(); config['project']['todo_storage']['max_file_bytes'] = 2
        write(self.config_path, config)
        with self.assertRaisesRegex(ValueError, 'configuration changed'):
            installation.prepare(self.settings, self.bundle, self.candidate, self.ops)
        (installation.location(self.settings)/'bin/opencoder-server').write_bytes(b'tampered')
        with self.assertRaisesRegex(ValueError, 'artifact changed'):
            installation.prepare(self.settings, self.bundle, self.candidate, self.ops)


if __name__ == '__main__':
    unittest.main()
