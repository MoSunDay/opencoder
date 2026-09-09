"""Regression checks for the isolation/preparation bugs found in real acceptance."""
from pathlib import Path
import base64
import json
import sys
import subprocess
import tempfile
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import git, run
from environment import private_environment
from snapshots import clone
from verify import messages, read_only_at


class Boundaries(unittest.TestCase):
    def test_private_runner_preserves_existing_sso_without_forwarding_unrelated_secrets(self):
        source = {'HOME': '/original', 'JWT_TOKEN': 'existing-test-sso', 'UNRELATED_SECRET': 'excluded'}
        environment = private_environment(Path('/private/runtime'), source)
        self.assertEqual(environment['JWT_TOKEN'], source['JWT_TOKEN'])
        self.assertEqual(environment['HOME'], '/private/runtime/home')
        self.assertEqual(environment['XDG_CONFIG_HOME'], '/private/runtime/home/.config')
        self.assertNotIn('UNRELATED_SECRET', environment)
        self.assertEqual(source['HOME'], '/original')
        self.assertNotIn('JWT_TOKEN', private_environment(Path('/private/runtime'), {}))

    def test_batch_command_does_not_consume_its_callers_input(self):
        helper = str(Path(__file__).resolve().parents[1])
        script = ('import sys; sys.path.insert(0, sys.argv[1]); from common import run; '
                  'print(run([sys.executable, "-c", "import sys; print(sys.stdin.read())"]), end="")')
        result = subprocess.run([sys.executable, '-c', script, helper],
                                input='unrelated caller input', text=True, capture_output=True, check=True)
        self.assertEqual(result.stdout, '\n')

    def test_nested_writable_home_overrides_strict_root(self):
        mounts = ['1 0 0:1 / / ro - ext4 disk rw',
                  '2 1 0:1 /root /root rw - ext4 disk rw']
        self.assertFalse(read_only_at(mounts, '/root/workspace'))
        mounts += ['3 2 0:1 /root/workspace /root/workspace ro - ext4 disk rw']
        self.assertTrue(read_only_at(mounts, '/root/workspace'))

    def test_empty_submodule_directory_is_not_its_parent_repository(self):
        with tempfile.TemporaryDirectory(prefix='opencoder-snapshot-test-') as temporary:
            root = Path(temporary)
            source, parent, target = root / 'source', root / 'parent', root / 'parent/repos/nested'
            source.mkdir(); parent.mkdir(); target.mkdir(parents=True)
            for repository in [source, parent]:
                git(repository, 'init', '-q')
                git(repository, 'config', 'user.name', 'Acceptance')
                git(repository, 'config', 'user.email', 'acceptance@example.invalid')
                (repository / 'file.txt').write_text(repository.name)
                git(repository, 'add', 'file.txt')
                git(repository, 'commit', '-qm', 'fixture')
            revision = git(source, 'rev-parse', 'HEAD')
            parent_revision = git(parent, 'rev-parse', 'HEAD')
            clone(source, target, revision)
            self.assertEqual(git(target, 'rev-parse', '--show-toplevel'), str(target))
            self.assertEqual(git(parent, 'rev-parse', 'HEAD'), parent_revision)
            self.assertEqual(git(source, 'rev-parse', 'HEAD'), revision)
            self.assertFalse((target / '.git/objects/info/alternates').exists())
            obj = next(p for p in (source / '.git/objects').glob('*/*') if p.is_file())
            copied = target / '.git/objects' / obj.relative_to(source / '.git/objects')
            self.assertNotEqual(obj.stat().st_ino, copied.stat().st_ino)

    def test_message_byte_cursor_preserves_split_utf8(self):
        value = [{'kind': 'text', 'text': '真实执行'}]
        data = json.dumps(value, ensure_ascii=False).encode()
        cut = data.index('真'.encode()) + 1
        pages = []
        for offset, end in [(0, cut), (cut, len(data))]:
            pages.append({'chunks': [{'seq': 1, 'offset': offset,
                'bytes_b64': base64.b64encode(data[offset:end]).decode()}],
                'more': end < len(data), 'next_cursor': {'seq': 1, 'offset': end}})
        class API:
            def api(self, path):
                if not pages:
                    raise AssertionError('Unexpected extra page')
                return pages.pop(0)
        self.assertEqual(messages(API(), 'test'), [value])


if __name__ == '__main__':
    unittest.main()
