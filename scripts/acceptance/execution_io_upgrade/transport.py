"""Fixture-only HTTP transport that loses one acknowledged Create reply."""
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import urllib.error
import urllib.request

from environment import Environment


class RuntimeProxy:
    def __init__(self, endpoint):
        self.endpoint = endpoint
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.fault = None
        self.lock = threading.Lock()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def handle_request(self):
                raw = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                request = urllib.request.Request(owner.endpoint + self.path,
                    data=raw if self.command != 'GET' else None, method=self.command,
                    headers={k: v for k, v in self.headers.items()
                             if k.lower() not in ('host', 'connection', 'content-length')})
                try:
                    with owner.opener.open(request, timeout=120) as response:
                        body, status = response.read(), response.status
                except urllib.error.HTTPError as error:
                    body, status = error.read(), error.code
                except OSError as error:
                    body, status = json.dumps({'error': str(error)}).encode(), 503
                # The real Runtime has already checked authentication and
                # accepted the exact request before any fault body is parsed.
                callback = None
                if self.path == '/rpc' and status == 200 and raw:
                    operation = json.loads(raw)
                    assignment = operation.get('assignment', {})
                    payload = assignment.get('request', {}).get('input', {})
                    with owner.lock:
                        if (owner.fault and operation.get('operation') == 'create'
                                and assignment['index']['kind'] == 'agent'
                                and payload.get('brain_layered', {}).get('run_id') == owner.fault[0]
                                and json.loads(body)['status'] < 300):
                            callback = owner.fault[1]
                            owner.fault = None
                    if callback:
                        callback(assignment)
                        self.close_connection = True
                        return
                self.send_response(status)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            do_GET = handle_request
            do_POST = handle_request
            do_PUT = handle_request

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def arm(self, run_id, callback):
        with self.lock:
            assert self.fault is None
            self.fault = run_id, callback


class ProxiedEnvironment(Environment):
    def __init__(self, *args, **kwargs):
        self.proxies = {}
        super().__init__(*args, **kwargs)

    def http(self, base, path, method='GET', body=None):
        if path == '/runtimes' and method == 'POST':
            body = copy.deepcopy(body)
            identifier = body['id']
            endpoint = body['config']['endpoint']
            assert endpoint.startswith('http://127.0.0.1:')
            if identifier not in self.proxies:
                self.proxies[identifier] = RuntimeProxy(endpoint)
            proxy = self.proxies[identifier]
            body['config']['endpoint'] = f'http://127.0.0.1:{proxy.server.server_port}'
        return super().http(base, path, method, body)

    def close(self):
        super().close()
        for proxy in self.proxies.values():
            proxy.server.shutdown()
            proxy.server.server_close()
