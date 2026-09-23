"""Storage preflight accepts duplicate local mounts and rejects unknown layers."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling.manifest import resources, brain_preflight
import json


class ResourceTests(unittest.TestCase):
    def test_active_supported_brain_runs_allow_release_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            settings = Settings(root, root, root, root, root / 'token')
            originals = {}
            for version in (4,):
                for status in ('pending', 'running', 'idle'):
                    identifier = f'brain-v{version}-{status}'
                    path = runtime / 'brain' / identifier / 'execution.json'
                    path.parent.mkdir(parents=True)
                    record = {'assignment': {'request': {'id': identifier,
                        'kind': 'brain', 'input': {'schema_version': version}},
                        'index': {'status': status}}}
                    path.write_text(json.dumps(record))
                    originals[path] = path.read_bytes()
            brain_preflight(settings, {'protocol_version': 10},
                [{'runtime_data': str(runtime)}])
            self.assertEqual({path: path.read_bytes() for path in originals}, originals)

    def test_supported_brain_still_rejects_legacy_embedded_receipts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'brain' / 'brain-v4' / 'execution.json'
            path.parent.mkdir(parents=True)
            settings = Settings(root, root, root, root, root / 'token')
            for legacy in ({'_brain': {'schema_version': 1}}, {'_brain': {'schema_version': 2}},
                           {'brain_scheduler': {}},
                           {'brain_receipt': {}}, {'playbook_receipt': {}}):
                record = {'assignment': {'request': {'id': 'brain-v4',
                    'kind': 'brain', 'input': {'schema_version': 4, **legacy}},
                    'index': {'status': 'running'}}}
                path.write_text(json.dumps(record))
                original = path.read_bytes()
                with self.subTest(legacy=legacy), self.assertRaisesRegex(
                        ValueError, 'migration blocked.*brain-v4'):
                    brain_preflight(settings, {'protocol_version': 10},
                        [{'runtime_data': str(root)}])
                self.assertEqual(path.read_bytes(), original)

    def test_brain_upgrade_does_not_mutate_old_runs_and_waits_for_idle_roots(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            path = runtime / 'brain' / 'brain-old' / 'execution.json'
            path.parent.mkdir(parents=True)
            settings = Settings(root, root, root, root, root / 'token')
            for version in (1, 2, 3, 5, None):
                record = {'assignment': {'request': {'id': 'brain-old', 'kind': 'brain', 'input': {'schema_version': version}}, 'index': {'status': 'idle'}}}
                path.write_text(json.dumps(record))
                original = path.read_bytes()
                with self.subTest(version=version), self.assertRaisesRegex(ValueError, 'migration blocked.*brain-old'):
                    brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(runtime)}])
                self.assertEqual(path.read_bytes(), original)
                record['assignment']['index']['status'] = 'done'
                path.write_text(json.dumps(record))
                brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(runtime)}])

    def test_milestone_upgrade_and_rollback_wait_for_incompatible_active_roots(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'brain' / 'brain-retained' / 'execution.json'
            path.parent.mkdir(parents=True)
            settings = Settings(root, root, root, root, root / 'token')
            for candidate, active in ((5, 4), (4, 5)):
                record = {'assignment': {'request': {'id': 'brain-retained', 'kind': 'brain',
                    'input': {'schema_version': active}}, 'index': {'status': 'idle'}}}
                path.write_text(json.dumps(record))
                original = path.read_bytes()
                with self.assertRaisesRegex(ValueError, 'migration blocked.*brain-retained'):
                    brain_preflight(settings, {'protocol_version': 10, 'brain_schema_version': candidate}, [{'runtime_data': str(root)}])
                self.assertEqual(original, path.read_bytes())
                record['assignment']['index']['status'] = 'done'
                path.write_text(json.dumps(record))
                brain_preflight(settings, {'protocol_version': 10, 'brain_schema_version': candidate}, [{'runtime_data': str(root)}])
            record['assignment']['index']['status'] = 'running'
            path.write_text(json.dumps(record))
            brain_preflight(settings, {'protocol_version': 10, 'brain_schema_version': 5}, [{'runtime_data': str(root)}])

    def test_every_reported_mount_must_be_verified_local_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root, root, root, root, root / 'token', min_memory_mb=1)
            with patch('rolling.manifest.shutil.disk_usage', return_value=SimpleNamespace(free=2**40)):
                for output in ['ext4\n', 'ext4\next4\n', 'ext4\nxfs\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        resources(settings, {'files': {}})
                for output in ['', 'nfs4\n', 'ext4\nnfs4\n', 'ext4\nunknown\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        with self.assertRaisesRegex(ValueError, 'verified local storage'):
                            resources(settings, {'files': {}})


if __name__ == '__main__':
    unittest.main()
