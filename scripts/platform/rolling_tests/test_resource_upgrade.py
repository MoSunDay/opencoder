import sys
import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from contextlib import nullcontext, redirect_stdout
from io import StringIO

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import resources
from rolling import cli


class Operations:
    def __init__(self, commit):
        self.commit = commit
        self.modes = {'consumer': 'open', 'already-frozen': 'frozen'}
        self.active = 0
        self.calls = []

    def http(self, base, path, method='GET', body=None):
        self.calls.append((method, path))
        if path == '/api/nodes':
            return {'nodes': [{'id': node, 'online': True, 'snapshot': {'pending_runs': 0}}
                              for node in self.modes]}
        if path.endswith('/admission'):
            node = path.split('/')[3]
            if method == 'POST':
                self.modes[node] = 'frozen'
            elif method == 'DELETE':
                self.modes[node] = 'open'
            return {'mode': self.modes[node], 'active_runs': self.active, 'owned_processes': 0}
        if path == '/api/health':
            return {'build': {'git_commit': self.commit}}
        if path == '/api/agents/nfs':
            return {'status': {'port': 2049}}
        raise AssertionError(path)

    def wait(self, check, seconds):
        if not check():
            raise TimeoutError('still active')

    def run(self, *args):
        self.calls.append(args)


class ResourceUpgradeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.settings = SimpleNamespace(state_dir=root / 'state', systemd_dir=root / 'units',
                                        public_url='http://public', resource_url='http://resources',
                                        token_file=root / 'token')
        self.binary = self.settings.state_dir / 'services/opencoder-resources'
        self.unit = self.settings.systemd_dir / 'opencoder-resources.service'
        self.binary.parent.mkdir(parents=True)
        self.unit.parent.mkdir()
        self.binary.write_bytes(b'old binary')
        self.unit.write_bytes(b'old unit')
        self.operations = Operations('c' * 40)
        self.stops = []
        for name, value in [('manifest.verify', {'release_id': 'candidate', 'commit': 'c' * 40}),
                            ('probes.resource_service', None), ('probes.resources', None),
                            ('check_retransmission', None)]:
            patcher = patch('rolling.resources.' + name, return_value=value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch('rolling.resources.services.stop_unit', side_effect=self.stop)
        patcher.start()
        self.addCleanup(patcher.stop)
        patcher = patch('rolling.resources.services.resource_upgrade', side_effect=self.install)
        patcher.start()
        self.addCleanup(patcher.stop)

    def stop(self, unit, operations, seconds):
        self.assertEqual(unit, 'opencoder-resources.service')
        self.stops.append(unit)

    def install(self, *args):
        self.binary.write_bytes(b'new binary')
        self.unit.write_bytes(b'new unit')

    def test_success_preserves_preexisting_freeze_and_never_stops_business_processes(self):
        result = resources.deploy(self.settings, Path('/bundle'), self.operations,
                                  ['consumer', 'already-frozen'])
        self.assertEqual(result['phase'], 'complete')
        self.assertLess(result['switch_seconds'], 30)
        self.assertEqual(self.operations.modes, {'consumer': 'open', 'already-frozen': 'frozen'})
        self.assertEqual(self.binary.read_bytes(), b'new binary')
        self.assertEqual(len(self.stops), 1)

    def test_active_work_aborts_before_service_stop_and_reopens_admission(self):
        self.operations.active = 1
        with self.assertRaises(TimeoutError):
            resources.deploy(self.settings, Path('/bundle'), self.operations, ['consumer'])
        self.assertEqual(self.stops, [])
        self.assertEqual(self.binary.read_bytes(), b'old binary')
        self.assertEqual(self.operations.modes['consumer'], 'open')

    def test_failed_rpc_verification_restores_only_binary_and_unit(self):
        with patch('rolling.resources.check_retransmission', side_effect=TimeoutError('duplicate blocked')):
            with self.assertRaises(TimeoutError):
                resources.deploy(self.settings, Path('/bundle'), self.operations, ['consumer'])
        self.assertEqual(self.binary.read_bytes(), b'old binary')
        self.assertEqual(self.unit.read_bytes(), b'old unit')
        self.assertEqual(self.operations.modes['consumer'], 'open')
        self.assertEqual(len(self.stops), 2)

    def test_cli_rejects_missing_consumers_and_conflicting_actions_before_configuration(self):
        for arguments in [
            ['--resources-only', '--bundle', '/bundle'],
            ['--resources-only', '--resource-nodes', 'consumer', '--signal'],
            ['--resources-only', '--resource-nodes', 'consumer', '--maintenance'],
            ['--resource-nodes', 'consumer'],
            ['--resources-only', '--rollback', '--resource-nodes', 'consumer'],
        ]:
            with self.subTest(arguments=arguments), patch('sys.argv', ['rolling', *arguments]), \
                    patch('rolling.cli.config.load') as load:
                with self.assertRaises(SystemExit) as caught:
                    cli.main()
                self.assertEqual(caught.exception.code, 2)
                load.assert_not_called()

    def test_cli_dispatches_only_resource_service_with_explicit_consumers(self):
        arguments = ['rolling', '--resources-only', '--resource-nodes', 'consumer', 'already-frozen',
                     '--bundle', '/bundle']
        with patch('sys.argv', arguments), patch('rolling.cli.config.load', return_value=self.settings), \
                patch('rolling.cli.Operations', return_value=self.operations), \
                patch('rolling.cli.locked', return_value=nullcontext()), \
                patch('rolling.cli.resources.deploy', return_value={'phase': 'complete'}) as deploy, \
                patch('rolling.cli.deployment.deploy') as business, redirect_stdout(StringIO()):
            cli.main()
        deploy.assert_called_once_with(self.settings, Path('/bundle'), self.operations,
                                       ['consumer', 'already-frozen'])
        business.assert_not_called()


if __name__ == '__main__':
    unittest.main()
