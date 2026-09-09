"""Run real Go tests with an explicitly selected, loopback-only metrics fixture."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


PREFIXES = ('webcast.metricw.sdk/metricw_sampler/', 'webcast.metricw.sdk/metricw_clip/')


def response(body):
    """Answer only metricw subscriptions using BCC's absent-configuration contract."""
    if not isinstance(body, dict) or body.get('paths'):
        raise ValueError('Unexpected configuration path subscription')
    keys = [item['key'] for item in body.get('keys', [])]
    if not keys or any(not isinstance(key, str) or not key.startswith(PREFIXES) for key in keys):
        raise ValueError('Unexpected configuration key')
    return {'key_update': [{'key': key, 'status': 1, 'update_id': 1, 'version': 1} for key in keys],
            'path_update': [], 'query_interval': 1}, keys


def fixture_environment(environment, port):
    return {**environment, 'BCC_WITH_PULL_CHANNEL_REMOTE_ADDR': '127.0.0.1:' + str(port),
            'BCC_CLIENT_WITH_REMOTE_ADDR': '127.0.0.1:1', 'BCC_DISABLE_GRPC': 'true',
            'DISABLE_BCC_SIDECAR': 'true'}


def execute(command):
    interfaces = [line.split(':', 1)[0].strip()
                  for line in Path('/proc/net/dev').read_text().splitlines()[2:]]
    if interfaces != ['lo']:
        raise RuntimeError('Metrics fixture requires a network namespace with only loopback')
    if command[:2] != ['go', 'test']:
        raise ValueError('Metrics fixture only wraps real Go tests')
    records = []

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_POST(self):
            try:
                if self.path != '/conf/get':
                    raise ValueError('Unexpected fixture route')
                size = int(self.headers.get('Content-Length', '0'))
                if not 0 < size <= 1024 * 1024:
                    raise ValueError('Invalid request size')
                result, keys = response(json.loads(self.rfile.read(size)))
                records.append({'keys': keys, 'fixture': 'absent-metrics-config'})
                data = json.dumps(result).encode()
                self.send_response(200)
                self.end_headers()
                self.wfile.write(data)
            except Exception as error:
                # Never log request bodies: SDK environment metadata can contain credentials.
                records.append({'error': type(error).__name__})
                self.send_response(400)
                self.end_headers()

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    process = None
    old_handlers = {}

    def forward(sig, _frame):
        if process and process.poll() is None:
            process.send_signal(sig)

    try:
        process = subprocess.Popen(command, env=fixture_environment(os.environ, server.server_port))
        for sig in (signal.SIGTERM, signal.SIGINT):
            old_handlers[sig] = signal.signal(sig, forward)
        code = process.wait()
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)
        receipt = {'interfaces': interfaces, 'requests': records, 'command': command}
        Path('/cache/metricw-fixture.json').write_text(json.dumps(receipt, indent=2))
        print('opencoder_metrics_fixture=' + json.dumps(receipt), file=sys.stderr, flush=True)
    if any('error' in item for item in records):
        raise RuntimeError('Metrics fixture received an unexpected request')
    return code if code >= 0 else 128 - code


if __name__ == '__main__':
    sys.exit(execute(sys.argv[1:]))
