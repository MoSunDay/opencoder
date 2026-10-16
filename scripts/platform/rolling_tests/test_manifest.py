"""Storage preflight accepts duplicate local mounts and rejects unknown layers."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from copy import deepcopy
from rolling.manifest import resources, brain_preflight, compatible, verify, execution_io, writer_preflight, _installer
import json


class ResourceTests(unittest.TestCase):
    def test_initiative_format_requires_maintenance_before_any_candidate_can_overlap(self):
        candidate = {'release_id': 'initiative-without-status', 'protocol_version': 10,
                     'brain_schema_version': 7, 'compatibility': {
                         'protocol': {'min': 1, 'max': 1},
                         'data_format': {'min': 9, 'max': 9}}}
        from rolling.manifest import _installer
        with patch.object(_installer, 'verify_bundle', return_value=candidate), \
                patch.object(_installer, 'bundle_names', return_value=_installer.NAMES), \
                patch.object(_installer, 'build_info', return_value={
                    'brain_schema_version': 7, 'release_compatibility': candidate['compatibility']}):
            self.assertEqual(verify(Path('bundle')), candidate)
        compatible(candidate, [candidate])
        for version in range(1, 9):
            previous = {**candidate, 'release_id': 'previous', 'compatibility': {
                **candidate['compatibility'], 'data_format': {'min': version, 'max': version}}}
            for new, old in [(candidate, previous), (previous, candidate)]:
                with self.subTest(version=version), self.assertRaisesRegex(ValueError, '--maintenance'):
                    compatible(new, [old])

    def test_identity_format_verifies_but_cannot_overlap_legacy_identity_servers(self):
        candidate = {'release_id': 'identity-index', 'protocol_version': 10,
                     'brain_schema_version': 7, 'compatibility': {
                         'protocol': {'min': 1, 'max': 1},
                         'data_format': {'min': 6, 'max': 6}}}
        info = {'brain_schema_version': 7,
                'release_compatibility': candidate['compatibility']}
        from rolling.manifest import _installer
        with patch.object(_installer, 'verify_bundle', return_value=candidate), \
                patch.object(_installer, 'bundle_names', return_value=_installer.NAMES), \
                patch.object(_installer, 'build_info', return_value=info):
            self.assertEqual(verify(Path('bundle')), candidate)
        compatible(candidate, [candidate])
        previous = {**candidate, 'release_id': 'legacy-identity', 'compatibility': {
            **candidate['compatibility'], 'data_format': {'min': 5, 'max': 5}}}
        for new, old in [(candidate, previous), (previous, candidate)]:
            with self.assertRaisesRegex(ValueError, 'different data_format; use --maintenance'):
                compatible(new, [old])

    def test_container_cutover_rejects_unpinned_active_dags_without_mutating_them(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'dag/active/execution.json'
            path.parent.mkdir(parents=True)
            record = {'assignment': {'request': {'id': 'active', 'kind': 'dag'}, 'index': {'status': 'running'}}}
            path.write_text(json.dumps(record))
            settings = Settings(root, root / 'token')
            original = path.read_bytes()
            with self.assertRaisesRegex(ValueError, 'DAG migration blocked.*active'):
                brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(root)}])
            self.assertEqual(path.read_bytes(), original)
            record['annotations'] = {'dag_parent': str(root / 'runs/1970-01-01/active')}
            path.write_text(json.dumps(record))
            brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(root)}])
            record['annotations'] = None
            path.write_text(json.dumps(record))
            with self.assertRaisesRegex(ValueError, 'DAG migration blocked.*active'):
                brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(root)}])
            record.pop('annotations')
            record['assignment']['index']['status'] = 'done'
            path.write_text(json.dumps(record))
            brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(root)}])

    def test_bootstrap_history_is_preserved_but_only_registered_runtime_roots_block_cutover(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            legacy = root / 'legacy'
            runtime = root / 'runtime'
            path = legacy / 'dag/stale/execution.json'
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({'annotations': None, 'assignment': {
                'request': {'id': 'stale', 'kind': 'dag'}, 'index': {'status': 'interrupted'}}}))
            original = path.read_bytes()
            settings = Settings(root, root / 'token', legacy_agent_data=legacy)
            brain_preflight(settings, {'protocol_version': 10}, [{'runtime_data': str(runtime)}])
            self.assertEqual(path.read_bytes(), original)
            for releases in ([], [{'runtime_data': str(legacy)}]):
                with self.subTest(releases=releases), self.assertRaisesRegex(ValueError, 'DAG migration blocked.*stale'):
                    brain_preflight(settings, {'protocol_version': 10}, releases)
            self.assertEqual(path.read_bytes(), original)

    def test_active_supported_brain_runs_allow_release_without_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            runtime = root / 'runtime'
            settings = Settings(root, root / 'token')
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
            settings = Settings(root, root / 'token')
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
            settings = Settings(root, root / 'token')
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
            settings = Settings(root, root / 'token')
            for candidate, active in ((5, 4), (4, 5), (6, 5), (5, 6)):
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
            record['assignment']['request']['input']['schema_version'] = 6
            path.write_text(json.dumps(record))
            brain_preflight(settings, {'protocol_version': 10, 'brain_schema_version': 6}, [{'runtime_data': str(root)}])

    def test_every_reported_mount_must_be_verified_local_storage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root, root / 'token', min_memory_mb=1)
            with patch('rolling.manifest.shutil.disk_usage', return_value=SimpleNamespace(free=2**40)):
                for output in ['ext4\n', 'ext4\next4\n', 'ext4\nxfs\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        resources(settings, {'release_id': 'candidate', 'files': {}})
                for output in ['', 'nfs4\n', 'ext4\nnfs4\n', 'ext4\nunknown\n']:
                    with self.subTest(output=output), patch('subprocess.run', return_value=SimpleNamespace(stdout=output)):
                        with self.assertRaisesRegex(ValueError, 'verified local storage'):
                            resources(settings, {'release_id': 'candidate', 'files': {}})


def release(identifier, io=None):
    value = {
        'release_id': identifier, 'protocol_version': 10, 'brain_schema_version': 7,
        'compatibility': {'protocol': {'min': 1, 'max': 1}, 'data_format': {'min': 9, 'max': 9}},
    }
    if io is not None:
        value['execution_io'] = io
        if io.get('writer') == 1:
            value['compatibility']['data_format'] = {'min': 10, 'max': 10}
    return value


class ExecutionIoTests(unittest.TestCase):
    def setUp(self):
        self.legacy = release('legacy')
        self.bridge = release('bridge', {'reader': 1, 'writer': 0})
        self.writer = release('writer', {'reader': 1, 'writer': 1})

    def test_forward_requires_a_reader_baseline_before_writer(self):
        writer_preflight(self.bridge, self.legacy)
        with self.assertRaisesRegex(ValueError, 'reader bridge'):
            writer_preflight(self.writer, self.legacy)
        writer_preflight(self.writer, self.bridge)
        compatible(self.writer, [self.legacy, self.bridge])

    def test_rollback_bridge_reads_new_intents_but_legacy_target_is_rejected(self):
        retained = [self.legacy, self.bridge, self.writer]
        compatible(self.bridge, retained)
        with self.assertRaisesRegex(ValueError, 'cannot read execution IO'):
            compatible(self.legacy, retained)
        compatible(self.legacy, [self.legacy, self.bridge])

    def test_legacy_resources_remain_compatible(self):
        compatible(self.writer, [release('resource-service')])

    def test_new_writer_cannot_mislabel_its_durable_format_for_legacy_tools(self):
        forged = deepcopy(self.writer)
        forged['compatibility']['data_format'] = {'min': 9, 'max': 9}
        with self.assertRaisesRegex(ValueError, 'handoff format 10'):
            compatible(forged, [self.bridge])
        forged = deepcopy(self.writer)
        forged.pop('execution_io')
        with self.assertRaisesRegex(ValueError, 'format 10 requires'):
            compatible(self.bridge, [forged])

    def test_bad_or_invented_protocol_metadata_fails_closed(self):
        self.assertEqual(execution_io(self.legacy), {'reader': 0, 'writer': 0})
        for value in [None, {}, {'reader': True, 'writer': 0}, {'reader': 0, 'writer': 1},
                      {'reader': 2, 'writer': 1}, {'reader': 1, 'writer': '0'}]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                execution_io(release('bad', value) if value is not None else {'execution_io': None})

    def test_manifest_must_match_the_compiled_protocol_metadata(self):
        info = {'release_compatibility': self.bridge['compatibility'],
                'brain_schema_version': 7, 'execution_io': self.bridge['execution_io']}
        with patch.object(_installer, 'verify_bundle', return_value=self.bridge), \
                patch.object(_installer, 'bundle_names', return_value=_installer.NAMES), \
                patch.object(_installer, 'build_info', return_value=info):
            self.assertEqual(verify(Path('bridge')), self.bridge)
            forged = deepcopy(self.bridge)
            forged['execution_io']['writer'] = 1
            with patch.object(_installer, 'verify_bundle', return_value=forged), \
                    self.assertRaisesRegex(ValueError, 'compiled binary'):
                verify(Path('bridge'))


if __name__ == '__main__':
    unittest.main()
