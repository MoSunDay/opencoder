"""Unpinned children use their actual owner; foreign or ambiguous journals fail."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from workspaces import owned_directory, runtime_directories


class WorkspaceTests(unittest.TestCase):
    def journal(self, root, owner='node-child', parent='owned', operation='op-owned'):
        root.mkdir(exist_ok=True)
        (root / 'node-id').write_text(owner)
        path = root / 'dag/child/execution.json'
        path.parent.mkdir(parents=True)
        path.write_text(json.dumps({'assignment': {
            'index': {'id': 'child', 'kind': 'dag', 'node_id': owner},
            'request': {'id': 'child', 'input': {'brain_layered': {
                'run_id': parent, 'operation_id': operation}}}}}))

    def operation(self):
        return {'run_id': 'owned', 'operation_id': 'op-owned',
                'execution_id': 'child', 'execution_kind': 'dag'}

    def test_unpinned_child_uses_exact_owner_instead_of_root_runtime(self):
        with tempfile.TemporaryDirectory() as directory:
            root, child = Path(directory) / 'root', Path(directory) / 'child'
            self.journal(root, owner='node-root')
            self.journal(child)
            self.assertEqual(owned_directory(self.operation(), 'node-child', [root, child]), child)

    def test_foreign_parent_operation_and_ambiguous_owners_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'first'
            self.journal(root, parent='foreign')
            with self.assertRaisesRegex(AssertionError, 'does not belong'):
                owned_directory(self.operation(), 'node-child', [root])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'first'
            self.journal(root, operation='foreign-op')
            with self.assertRaisesRegex(AssertionError, 'does not belong'):
                owned_directory(self.operation(), 'node-child', [root])
        with tempfile.TemporaryDirectory() as directory:
            roots = [Path(directory) / name for name in ('first', 'second')]
            for root in roots:
                self.journal(root)
            with self.assertRaisesRegex(AssertionError, 'uniquely accessible'):
                owned_directory(self.operation(), 'node-child', roots)

    def test_process_discovery_does_not_read_config_or_accept_other_binaries(self):
        with tempfile.TemporaryDirectory() as directory:
            proc = Path(directory)
            for pid, binary in [('1', 'opencoder-agent'), ('2', 'other')]:
                path = proc / pid
                path.mkdir()
                (path / 'exe').symlink_to(proc / binary)
                (path / 'cmdline').write_bytes(b'exe\0--data-dir\0/owned-data\0')
            self.assertEqual(runtime_directories(proc), {Path('/owned-data')})


if __name__ == '__main__':
    unittest.main()
