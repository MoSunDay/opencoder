import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from contracts import build_request, device_request, identity
from evidence import freeze_source


class Contracts(unittest.TestCase):
    def test_device_request_is_pinned_and_reuses_identity(self):
        request = device_request('brain-demo', 'reproduce', 1, 'node-device', '/cases.json', ['case-original'])
        self.assertEqual(request['id'], 'dag-pc-demo-reproduce-1')
        self.assertEqual(request['node_id'], 'node-device')
        self.assertNotIn('prompt', request['input'])
        self.assertEqual(request['input']['device_count'], 1)
        self.assertEqual(request, device_request('brain-demo', 'reproduce', 1, 'node-device', '/cases.json', ['case-original']))

    def test_rounds_and_duplicate_cases_rejected(self):
        with self.assertRaises(ValueError):
            identity('brain-demo', 'verify', 3, 'team')
        with self.assertRaises(ValueError):
            device_request('brain-demo', 'reproduce', 1, 'node', '/cases', ['same', 'same'])

    def test_changed_input_cannot_replace_frozen_case(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source.json'
            frozen = Path(directory) / 'frozen.json'
            source.write_text(json.dumps({'case_id': 'original', 'prompt': 'actual problem'}))
            freeze_source(source, frozen)
            freeze_source(source, frozen)
            source.write_text('{}')
            with self.assertRaises(ValueError):
                freeze_source(source, frozen)

    def test_build_cannot_omit_diff_or_use_branch_as_commit(self):
        source = {'repo': 'VideoFusion-win', 'branch': 'release', 'commit': 'a'*40,
                  'worktree': '/isolated', 'purpose': 'fix', 'original_assertions': ['original'],
                  'changes': [{'diff_sha256': 'b'*64}]}
        request = build_request('brain-demo', 1, 'node-builder', source)
        self.assertEqual(request['target'], 'jy-builder')
        source['commit'] = 'HEAD'
        with self.assertRaises(ValueError):
            build_request('brain-demo', 1, 'node-builder', source)


if __name__ == '__main__':
    unittest.main()
