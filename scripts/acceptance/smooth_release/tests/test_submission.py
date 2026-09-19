"""Acceptance work stays on the Runtime whose continuity is measured."""
import copy
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from live import Live
from rolling.io import HttpFailure, Operations


class SubmissionTests(unittest.TestCase):
    def live(self):
        env = Live.__new__(Live)
        env.settings = SimpleNamespace(public_url='http://control')
        env.node_id = 'local-node'
        return env

    def test_measured_submission_is_pinned_and_failure_is_not_retried(self):
        env = self.live()
        request = {'id': 'dag-traffic', 'kind': 'dag'}
        with patch.object(Operations, 'http', side_effect=HttpFailure('POST', '/api/executions', 504, 'lost')) as http:
            with self.assertRaises(HttpFailure):
                env.api('/api/executions', 'POST', request)
        http.assert_called_once_with('http://control', '/api/executions', 'POST',
            {'node_id': 'local-node', **request}, timeout=90)
        self.assertNotIn('node_id', request)

    def test_initial_lost_reply_recovers_exact_request_and_durable_receipt(self):
        env = self.live()
        posts = []
        result = {'id': 'todos-chain', 'status': 'pending'}

        def http(base, path, method='GET', body=None):
            if method == 'GET':
                if len(posts) < 2:
                    raise HttpFailure(method, path, 404, 'no receipt')
                return {'phase': 'accepted', 'receipt': {'status': 202, 'body': result}}
            posts.append(copy.deepcopy(body))
            if len(posts) == 1:
                raise HttpFailure(method, path, 504, 'node request timed out')
            return result

        env.http = http
        request = {'id': 'todos-chain', 'kind': 'todos', 'input': {'prompt': 'fixed'}}
        self.assertEqual(env.submit_initial(request), result)
        self.assertEqual(posts, [{'node_id': 'local-node', **request}] * 2)
        self.assertNotIn('node_id', request)


if __name__ == '__main__':
    unittest.main()
