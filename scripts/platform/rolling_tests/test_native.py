import hashlib
import json
import os
from pathlib import Path
import sys
import unittest
import tempfile
from types import SimpleNamespace
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.io import HttpFailure
from rolling.native import publish_binary, runtime_config
from rolling.units import service, freeze_rootfs


class PublisherTests(unittest.TestCase):
    def test_frozen_image_preserves_internal_hardlinks_without_linking_the_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, binaries = root / 'source', root / 'bin'
            programs = source / 'usr/bin'
            programs.mkdir(parents=True)
            binaries.mkdir()
            (programs / 'git').write_bytes(b'original git')
            os.link(programs / 'git', programs / 'git-status')
            (programs / 'dag-runner').write_bytes(b'old runner')
            os.link(programs / 'dag-runner', programs / 'agent-step-runner')
            for name in ['dag-runner', 'agent-step-runner']:
                (binaries / name).write_bytes(name.encode())
            image = freeze_rootfs({'runtime_data': str(root / 'runtime')}, source, binaries)
            copied = image / 'usr/bin'
            self.assertEqual((copied / 'git').stat().st_ino, (copied / 'git-status').stat().st_ino)
            self.assertNotEqual((copied / 'git').stat().st_ino, (programs / 'git').stat().st_ino)
            (programs / 'git').write_bytes(b'changed after snapshot')
            self.assertEqual((copied / 'git-status').read_bytes(), b'original git')
            for name in ['dag-runner', 'agent-step-runner']:
                self.assertEqual((copied / name).read_bytes(), name.encode())
                self.assertEqual((programs / name).read_bytes(), b'old runner')

    def test_only_runtime_units_create_private_mounts_for_native_dags(self):
        runtime = service(['/bin/true'], 'runtime', runtime=True)
        self.assertIn('ExecStart="/usr/bin/unshare" "--mount" "--propagation" "private" "--" "/bin/true"', runtime)
        self.assertIn('Delegate=yes', runtime)
        self.assertNotIn('/usr/bin/unshare', service(['/bin/true'], 'server'))

    def test_runtime_config_uses_its_explicit_home_and_preserves_the_accepted_copy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / 'private-home'
            work = root / 'work'
            work.mkdir()
            (home / '.opencoder').mkdir(parents=True)
            (home / '.opencoder/config.json').write_text(json.dumps({'model': 'private/model'}))
            (work / 'opencoder.json').write_text(json.dumps({'dag': {'workspace_dir': '/private/nfs'}}))
            settings = SimpleNamespace(agent_workdir=work)
            record = {'runtime_data': str(root / 'runtime')}
            image = root / 'image'
            accepted = runtime_config(settings, record, image, home=home)
            config = json.loads((accepted / 'opencoder.json').read_text())
            self.assertEqual(config['model'], 'private/model')
            self.assertEqual(config['dag']['workspace_dir'], '/private/nfs')
            self.assertEqual(config['dag']['rootfs_dir'], str(image))
            (work / 'opencoder.json').write_text(json.dumps({'model': 'changed/model'}))
            self.assertEqual(runtime_config(settings, record, image, home=home), accepted)
            self.assertEqual(json.loads((accepted / 'opencoder.json').read_text()), config)
            with self.assertRaisesRegex(ValueError, 'retained runtime rootfs changed'):
                runtime_config(settings, record, root / 'different-image', home=home)

    def operations(self, mode):
        owner = self
        class Operations:
            calls = []
            exists = mode == 'existing'
            def http(self, base, path, method='GET', body=None):
                self.calls.append((path, method, body))
                if method == 'POST':
                    self.exists = True
                    if mode == 'race':
                        raise HttpFailure(method, path, 409, 'already published')
                    return {}
                if not self.exists:
                    raise HttpFailure(method, path, 404, 'missing')
                digest = hashlib.sha256(b'binary').hexdigest()
                return {'history': [{'version': 1, 'sha256': digest if mode != 'corrupt' else '0'*64, 'size_bytes': 6}]}
        return Operations()

    def test_existing_version_is_reused_without_mutation(self):
        operations = self.operations('existing')
        result = publish_binary('http://private', 'probe', b'binary', operations)
        self.assertTrue(result.endswith('@v1'))
        self.assertEqual(len(operations.calls), 1)

    def test_creation_and_lost_race_verify_exact_published_bytes(self):
        for mode in ['create', 'race']:
            operations = self.operations(mode)
            result = publish_binary('http://private', 'probe', b'binary', operations)
            self.assertTrue(result.endswith('@v1'))
            self.assertEqual([row[1] for row in operations.calls], ['GET', 'POST', 'GET'])

    def test_corrupt_publication_fails_closed(self):
        with self.assertRaisesRegex(ValueError, 'differs'):
            publish_binary('http://private', 'probe', b'binary', self.operations('corrupt'))
