"""Acceptance failures and recovery guards must fail visibly and keep their scope."""
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
from pathlib import Path
import sys
import tempfile
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from environment import Environment
from faults.model import chat_stream
from faults.runtime import blockers, FrozenWindow
import main


class RunnerTests(unittest.TestCase):
    def test_ambiguous_admission_retries_the_exact_same_root_request(self):
        env = Environment.__new__(Environment)
        env.created, env.tag, env.node_id = [], 'brain-e2e-test', 'node-test'
        env.save = lambda *_: None
        plan = {'schema_version': 7}
        replies = [(504, {'error': 'retry same id'}), (429, {}), (202, {})]
        with patch.object(env, 'request', side_effect=replies) as request, patch('environment.time.sleep'):
            identifier = env.create('case', plan)
        self.assertEqual(env.created, [identifier])
        self.assertEqual(len(request.call_args_list), 3)
        self.assertTrue(all(call == request.call_args_list[0] for call in request.call_args_list))
        with patch.object(env, 'request', return_value=(422, {'error': 'bad input'})) as request:
            with self.assertRaisesRegex(AssertionError, 'HTTP 422'):
                env.create('invalid', plan)
            self.assertEqual(request.call_count, 1)

    def test_startup_failure_records_every_unrun_case_and_nonzero_exit(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'evidence'
            args = ['main.py', '--config', 'unavailable', '--expected-commit', 'abc', '--out', str(output)]
            with patch.object(sys, 'argv', args), patch.object(main, 'Environment', side_effect=TimeoutError('offline')):
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(main.main(), 1)
            report = json.loads((output / 'result.json').read_text())
            self.assertEqual(report['result'], 'FAIL')
            self.assertEqual({row['case'] for row in report['cases']}, set(main.CASES))
            self.assertTrue(all(row['result'] == 'NOT_RUN' for row in report['cases']))

    def test_existing_evidence_is_never_overwritten_by_a_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / 'result.json'
            result.write_text('original failed attempt')
            args = ['main.py', '--config', 'unused', '--expected-commit', 'abc', '--out', directory]
            with patch.object(sys, 'argv', args), self.assertRaisesRegex(AssertionError, 'immutable'):
                main.main()
            self.assertEqual(result.read_text(), 'original failed attempt')

    def test_interrupt_cannot_be_reported_as_acceptance_success(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'evidence'
            args = ['main.py', '--config', 'unused', '--expected-commit', 'abc', '--out', str(output)]
            with patch.object(sys, 'argv', args), patch.object(main, 'Environment', side_effect=KeyboardInterrupt):
                with contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(main.main(), 1)
            report = json.loads((output / 'result.json').read_text())
            self.assertEqual(report['result'], 'FAIL')
            self.assertEqual(report['error'], 'acceptance interrupted')
            self.assertTrue(all(row['result'] == 'NOT_RUN' for row in report['cases']))

    def test_non_json_gateway_failure_preserves_status_and_body_without_credentials(self):
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                self.send_response(502)
                self.end_headers()
                self.wfile.write(b'<html>Bad Gateway</html>')

        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            env = Environment.__new__(Environment)
            env.counter, env.token = 0, 'credential-must-not-enter-evidence'
            env.settings = SimpleNamespace(public_url=f'http://127.0.0.1:{server.server_port}')
            env.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
            receipts = []
            env.save = lambda name, value: receipts.append((name, value))
            status, value = env.request('GET', '/api/ready')
            self.assertEqual(status, 502)
            self.assertIn('Bad Gateway', value['non_json_body'])
            self.assertEqual(receipts[0][1]['status'], 502)
            self.assertNotIn(env.token, json.dumps(receipts))
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def test_fault_scope_requires_exact_owned_ids_and_idle_other_nodes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'brain' / 'owned'
            root.mkdir(parents=True)
            (root / 'execution.json').write_text('{}')
            env = SimpleNamespace(created=['owned'], node_id='local', runtime_url='local',
                record={'runtime_data': directory},
                brain_rpc=lambda *_: {'operations': [{'execution_id': 'owned-child'}]},
                http=lambda *_: {'indexes': [
                    {'id': 'owned-child', 'status': 'running'},
                    {'id': 'owned-but-other-task', 'status': 'running'},
                    {'id': 'finished', 'status': 'done'}], 'snapshot': {'resource_error': None}},
                api=lambda *_: {'nodes': [{'id': 'remote', 'online': True, 'snapshot': {'pending_runs': 1}}]})
            result = blockers(env)
            self.assertEqual([r['id'] for r in result['local_executions']], ['owned-but-other-task'])
            self.assertEqual(result['other_busy_nodes'], ['remote'])
            with self.assertRaisesRegex(AssertionError, 'unrelated work'):
                FrozenWindow(env).__enter__()

    def test_fault_stream_carries_a_single_structured_decision(self):
        decision = {'decision': 'block', 'reason': 'injected fault'}
        raw = chat_stream(json.dumps(decision), 'model').decode()
        frames = [json.loads(line[6:]) for line in raw.splitlines()
                  if line.startswith('data: ') and line != 'data: [DONE]']
        self.assertEqual(json.loads(frames[0]['choices'][0]['delta']['content']), decision)
        self.assertEqual(frames[-1]['choices'][0]['finish_reason'], 'stop')
        self.assertTrue(raw.endswith('data: [DONE]\n\n'))


if __name__ == '__main__':
    unittest.main()
