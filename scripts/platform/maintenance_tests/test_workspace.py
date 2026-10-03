from dataclasses import replace
from pathlib import Path
import pwd
import copy
import stat
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture
from rolling.maintenance import preflight, services


def inventory(root):
    return {str(path.relative_to(root)): (
        path.stat().st_uid, path.stat().st_gid, stat.S_IMODE(path.stat().st_mode),
        path.read_bytes() if path.is_file() else None)
        for path in [root, *sorted(root.rglob('*'))]}


class WorkspaceTests(unittest.TestCase):
    def test_preflight_protects_source_from_launcher_service_and_ingress_installation(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            ingress_source = fixture.settings.nginx_include.parent / 'source-nginx'
            for source in (fixture.settings.bin_dir, fixture.settings.systemd_dir, ingress_source):
                with self.subTest(source=source):
                    source.mkdir(exist_ok=True)
                    settings = (replace(fixture.settings, nginx_include=source / 'ingress.conf')
                                if source == ingress_source else fixture.settings)
                    agent, server = copy.deepcopy(fixture.desired)
                    server['dag']['workspace_dir'] = str(source)
                    before = inventory(source)
                    paths = {'rootfs_dir': fixture.rootfs, 'binary_dir': Path('/mnt/binaries'),
                             'workspace_dir': Path('/mnt/workspace'), 'agents_dir': Path('/mnt/agents')}
                    with patch.object(preflight, 'configs', return_value=(agent, server)), \
                         patch.object(preflight, 'configuration', return_value=paths), \
                         patch.object(preflight.shutil, 'which', return_value='/usr/bin/runc'):
                        with self.assertRaisesRegex(ValueError, 'must be outside Server source workspace'):
                            preflight.check(settings, fixture.candidate, fixture)
                    self.assertEqual(inventory(source), before)

    def test_source_inside_service_state_is_rejected_before_any_upgrade_writes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            source = fixture.settings.state_dir / 'external-source'
            source.mkdir()
            (source / 'project.txt').write_bytes(b'outside application ownership')
            before = inventory(source)
            with patch.object(services, 'configs', return_value=({}, {'dag': {
                    'workspace_dir': str(source), 'binary_dir': str(Path(directory) / 'binaries')}})):
                with self.assertRaisesRegex(ValueError, 'must be outside Server source workspace'):
                    services.resource_upgrade(fixture.settings, fixture.bundle, fixture)
            self.assertEqual(inventory(source), before)
            self.assertEqual(fixture.calls, [])
            self.assertFalse((Path(directory) / 'binaries').exists())

    def test_mount_and_configuration_paths_cannot_alias_the_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.mkdir()
            alias = root / 'alias'
            alias.symlink_to(source, target_is_directory=True)
            before = inventory(source)
            operations = type('Commands', (), {'run': staticmethod(
                lambda *args: self.fail('permission commands must not run for an overlapping path'))})()
            for destination in (source, source / 'mount', alias / 'configuration'):
                with self.subTest(destination=destination):
                    with self.assertRaisesRegex(ValueError, 'must be outside Server source workspace'):
                        preflight.workspace_source(source, 'root', operations, [destination])
                    self.assertEqual(inventory(source), before)

    def test_managed_binary_pool_cannot_alias_or_be_nested_under_source(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            source = Path(directory) / 'source'
            source.mkdir()
            before = inventory(source)
            for binary in (source, source / 'binaries'):
                with self.subTest(binary=binary), patch.object(services, 'configs', return_value=({}, {
                        'dag': {'workspace_dir': str(source), 'binary_dir': str(binary)}})):
                    with self.assertRaisesRegex(ValueError, 'must be outside Server source workspace'):
                        services.resource_upgrade(fixture.settings, fixture.bundle, fixture)
                    self.assertEqual(inventory(source), before)
                    self.assertEqual(fixture.calls, [])

    def test_missing_workspace_is_rejected_without_creation_or_service_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            source = Path(directory) / 'missing/source'
            with patch.object(services, 'configs', return_value=({}, {'dag': {
                    'workspace_dir': str(source), 'binary_dir': str(Path(directory) / 'binaries')}})):
                with self.assertRaisesRegex(ValueError, 'must already exist'):
                    services.resource_upgrade(fixture.settings, fixture.bundle, fixture)
            self.assertFalse(source.parent.exists())
            self.assertFalse((Path(directory) / 'binaries').exists())
            self.assertEqual(fixture.calls, [])
            self.assertEqual((fixture.settings.state_dir / 'services/opencoder-resources').read_bytes(),
                             b'old resources')

    def test_upgrade_failure_and_retry_preserve_source_with_a_different_owner(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = Fixture(root)
            source = root / 'source'
            source.mkdir()
            (source / 'step').mkdir()
            (source / 'step/input.txt').write_bytes(b'immutable project input')
            root.chmod(0o755)
            user = 'nobody'
            self.assertNotEqual(source.stat().st_uid, pwd.getpwnam(user).pw_uid)
            settings = replace(fixture.settings, server_user=user)
            config = {'dag': {'workspace_dir': str(source), 'binary_dir': str(root / 'binaries')}}
            before = inventory(source)
            original = fixture.run

            def run(*args):
                if args[0] == 'runuser':
                    subprocess.run(args, check=True, capture_output=True)
                else:
                    original(*args)

            fixture.run = run
            with patch.object(services, 'configs', return_value=({}, config)):
                services.resource_upgrade(settings, fixture.bundle, fixture)
                self.assertEqual(inventory(source), before)
                with patch.object(services, 'atomic_bytes', side_effect=OSError('interrupted upgrade')):
                    with self.assertRaisesRegex(OSError, 'interrupted upgrade'):
                        services.resource_upgrade(settings, fixture.bundle, fixture)
                self.assertEqual(inventory(source), before)
                services.resource_upgrade(settings, fixture.bundle, fixture)
                self.assertEqual(inventory(source), before)

    def test_actual_service_user_must_be_able_to_read_and_traverse_workspace(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            root.chmod(0o755)
            source = root / 'source'
            source.mkdir()
            operations = type('Commands', (), {'run': staticmethod(
                lambda *args: subprocess.run(args, check=True, capture_output=True))})()
            for permissions in (0o333, 0o444, 0o700):
                with self.subTest(permissions=permissions):
                    source.chmod(permissions)
                    before = inventory(source)
                    with self.assertRaisesRegex(ValueError, 'must be readable by nobody'):
                        preflight.workspace_source(source, 'nobody', operations)
                    self.assertEqual(inventory(source), before)
            source.chmod(0o555)
            before = inventory(source)
            preflight.workspace_source(source, 'nobody', operations)
            self.assertEqual(inventory(source), before)
