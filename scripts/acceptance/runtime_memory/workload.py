"""Loopback-only model and completed executions for allocator retention checks."""
import concurrent.futures
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import time


class Model:
    def __init__(self):
        owner = self
        self.calls = 0
        self.lock = threading.Lock()

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                self.rfile.read(int(self.headers['Content-Length']))
                with owner.lock:
                    owner.calls += 1
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                for delta, reason in [({'role': 'assistant', 'content': 'fixture completed'}, None), ({}, 'stop')]:
                    body = {'choices': [{'index': 0, 'delta': delta, 'finish_reason': reason}]}
                    self.wfile.write(('data: ' + json.dumps(body) + '\n\n').encode())
                self.wfile.write(b'data: [DONE]\n\n')

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def config(self):
        return {'model': 'fixture/model', 'cache_salt': False, 'local_memory': False,
                'agent': {'agents_dir': None},
                'providers': {'fixture': {'base_url': f'http://127.0.0.1:{self.server.server_port}/v1',
                                           'api_key': 'local-fixture', 'protocol': 'chat_completions'}}}

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def complete(port, turn, tasks, request):
    def execution(index):
        identifier = f'agent-completion-{turn}-{index}'
        reference = {'id': identifier, 'kind': 'agent'}
        created = request(port, '/rpc', {'operation': 'create', 'assignment': {
            'index': dict(reference, node_id='node-memory-fixture', created_at=int(time.time() * 1000), status='pending'),
            'request': dict(reference, input={'prompt': 'Return fixture completed.'}),
            'runtime': {'fixture_payload': 'x' * (1024 * 1024)},
        }})
        if created['status'] != 200:
            raise AssertionError(f'create failed: {created}')
        deadline = time.monotonic() + 120
        while True:
            reply = request(port, '/rpc', {'operation': 'inspect', 'execution': reference})
            status = reply['body'].get('execution', {}).get('status')
            if status == 'idle':
                break
            if reply['status'] != 200 or status in ('error', 'cancelled', 'interrupted'):
                raise AssertionError(f'execution failed: {reply}')
            if time.monotonic() > deadline:
                raise TimeoutError('local fixture execution did not complete')
            time.sleep(0.05)
        stopped = request(port, '/rpc', {'operation': 'command', 'execution': reference,
            'command': {'action': 'cancel', 'input': {}}})
        if stopped['status'] != 200 or stopped['body']['status'] != 'cancelled':
            raise AssertionError(f'completed execution did not close: {stopped}')

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as executor:
        list(executor.map(execution, range(tasks)))

    deadline = time.monotonic() + 120
    while request(port, '/inventory')['snapshot']['active_runs']:
        if time.monotonic() > deadline:
            raise TimeoutError('completed executions did not release their running slots')
        time.sleep(0.05)


def verify_completed(data, expected):
    records = list((data / 'agent').glob('agent-completion-*/execution.json'))
    if len(records) != expected:
        raise AssertionError('completed execution history is incomplete')
    for path in records:
        record = json.loads(path.read_text())
        payload = record['assignment']['runtime']['fixture_payload']
        if payload != 'x' * (1024 * 1024):
            raise AssertionError('accepted runtime snapshot changed')
        if record['queue']['config']['agent']['runtime']['fixture_payload'] != payload:
            raise AssertionError('queued configuration snapshot changed')
        if record['assignment']['index']['status'] != 'cancelled' or record['lifecycle']['stop_intent'] != 'cancel':
            raise AssertionError('completed stop intent was not persisted')
    return len(records)
