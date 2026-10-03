import copy
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import flow
from rolling.maintenance.planning import capacity
from rolling.state import Journal


class CapacityTests(unittest.TestCase):
    def test_new_image_backup_controls_and_packages_are_budgeted_together(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            image = fixture.rootfs / 'image.bin'
            with image.open('wb') as stream:
                stream.truncate(512 * 1024 * 1024)
            candidate = copy.deepcopy(fixture.candidate)
            candidate['files'] = {'bin/opencoder-server': {'bytes': 100}, 'bin/dag-runner': {'bytes': 20}}
            budget = capacity.plan(fixture.settings, candidate, fixture.rootfs,
                                   Journal(fixture.settings.state_dir).data, {})
            self.assertGreaterEqual(budget['components']['frozen_image'], 512 * 1024 * 1024 + 20)
            self.assertGreaterEqual(budget['components']['stopped_backup'], budget['components']['frozen_image'])
            self.assertEqual(budget['components']['packages'], 240)
            self.assertGreater(budget['components']['controls'], 0)
            required = budget['filesystems'][0]['required']
            for available in (768 * 1024 * 1024, required - 1):
                with patch.object(capacity.shutil, 'disk_usage', return_value=SimpleNamespace(free=available)):
                    with self.assertRaisesRegex(ValueError, 'insufficient space before maintenance'):
                        capacity.check(budget)
            with patch.object(capacity.shutil, 'disk_usage', return_value=SimpleNamespace(free=required)):
                self.assertEqual(capacity.check(budget), budget)

    def test_existing_frozen_image_only_needs_backup_and_incomplete_copy_is_counted(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            runtime = fixture.settings.state_dir / 'runtimes/new/dag'
            frozen = runtime / 'rootfs'
            frozen.mkdir(parents=True)
            (frozen / 'image').write_bytes(b'x' * 1000)
            stage = runtime / '.rootfs-stage-interrupted'
            stage.mkdir(); (stage / 'image').write_bytes(b'x' * 500)
            budget = capacity.plan(fixture.settings, fixture.candidate, fixture.rootfs,
                                   Journal(fixture.settings.state_dir).data, {})
            self.assertEqual(budget['components']['frozen_image'], 0)
            self.assertGreaterEqual(budget['components']['stopped_backup'], 1500)

    def test_low_space_recheck_leaves_old_service_and_admission_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            with fixture.patches(), patch.object(capacity.shutil, 'disk_usage', return_value=SimpleNamespace(free=0)):
                with self.assertRaisesRegex(ValueError, 'insufficient space'):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
            self.assertIn(fixture.old['server_unit'], fixture.active)
            self.assertFalse(any('/api/admin/drain' in call for call in fixture.calls))
            self.assertEqual(Journal(fixture.settings.state_dir).data['maintenance']['stage'], 'closing')

    def test_tree_bytes_ignores_runtime_mounts_and_symlink_targets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'usr').mkdir(); (root / 'usr/app').write_bytes(b'abc')
            (root / 'proc').mkdir(); (root / 'proc/ephemeral').write_bytes(b'x' * 100)
            (root / 'link').symlink_to(root / 'usr')
            self.assertEqual(capacity.tree_bytes(root, {'proc'}), 3)
