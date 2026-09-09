"""Offline metrics setup must preserve real assertions and reject unrelated configuration."""
from pathlib import Path
import copy
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from dependencies.metricw import configure
from dependencies.metricw_fixture import fixture_environment, response


class OfflineMetrics(unittest.TestCase):
    def test_only_known_metrics_configuration_can_be_absent(self):
        key = 'webcast.metricw.sdk/metricw_sampler/-'
        result, keys = response({'keys': [{'key': key}], 'env_param': {'auth_token': 'private'}})
        self.assertEqual(keys, [key])
        self.assertEqual(result['key_update'], [{'key': key, 'status': 1, 'update_id': 1, 'version': 1}])
        self.assertNotIn('private', repr(result))
        for request in [{'paths': [{'path': '/business'}]}, {'keys': []},
                        {'keys': [{'key': 'business.config/default/credential'}]},
                        {'keys': [{'key': 1}]}, []]:
            with self.assertRaises(ValueError):
                response(request)

    def test_fixture_routes_are_local_and_do_not_modify_parent_environment(self):
        original = {'PATH': '/runtime/bin', 'BCC_WITH_PULL_CHANNEL_REMOTE_ADDR': 'old:123'}
        updated = fixture_environment(original, 3456)
        self.assertEqual(original['BCC_WITH_PULL_CHANNEL_REMOTE_ADDR'], 'old:123')
        self.assertEqual(updated['BCC_WITH_PULL_CHANNEL_REMOTE_ADDR'], '127.0.0.1:3456')
        self.assertEqual(updated['BCC_CLIENT_WITH_REMOTE_ADDR'], '127.0.0.1:1')
        self.assertEqual(updated['BCC_DISABLE_GRPC'], 'true')
        self.assertEqual(updated['PATH'], original['PATH'])

    def test_configuration_keeps_real_test_command_and_dependency_preparation(self):
        command = ['go', 'test', '-mod=readonly', '-count=1', '-run', '^TestActual$', './biz/handler']
        spec = {'repository': 'repos/jianying-openagent-api', 'command': command,
                'preparation': ['go', 'mod', 'download'], 'network': True,
                'files': [{'path': 'new_test.go', 'content': 'real assertion'}],
                'environment': {'TMPDIR': '/tmp'}}
        before = copy.deepcopy(spec)
        fixture = {'goroot': '/private/toolchain', 'wrapper': '/private/bin/fixture.py',
                   'files': [{'path': 'conf/filter.yaml', 'content': 'local_first: true'}]}
        result = configure(spec, fixture)
        self.assertEqual(spec, before)
        self.assertEqual(result['command'], ['python3', fixture['wrapper'], *command])
        self.assertEqual(result['preparation'], spec['preparation'])
        self.assertTrue(result['network'])  # The existing runner disconnects after preparation.
        self.assertEqual(result['files'][0], spec['files'][0])
        self.assertEqual(result['environment']['GOTOOLCHAIN'], 'local')
        self.assertEqual(result['environment']['TMPDIR'], '/cache/tmp')
        self.assertIn(fixture['goroot'], result['readOnly'])

    def test_unrelated_commands_are_unchanged_and_conflicts_fail(self):
        fixture = {'goroot': '/private/go', 'wrapper': '/private/bin/fixture.py',
                   'files': [{'path': 'conf/filter.yaml', 'content': 'local_first: true'}]}
        self.assertEqual(configure({'command': ['node', 'test.js']}, fixture), {'command': ['node', 'test.js']})
        for extra in [{'repository': 'repos/another'}, {'files': fixture['files']},
                      {'environment': {'GOFLAGS': '-race'}}]:
            with self.assertRaises(ValueError):
                configure({'repository': 'repos/jianying-openagent-api',
                           'command': ['go', 'test', './...'], **extra}, fixture)


if __name__ == '__main__':
    unittest.main()
