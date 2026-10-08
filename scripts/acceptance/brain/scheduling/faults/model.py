"""Model-response faults scoped to exact test run IDs and their own journals."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import subprocess
import urllib.request

from environment import require
from faults.runtime import FrozenWindow, blockers, journal, require_restartable, NotRun


def chat_stream(text, model):
    chunks = [{'id': 'brain-fault-response', 'object': 'chat.completion.chunk', 'model': model,
               'choices': [{'index': 0, 'delta': {'role': 'assistant', 'content': text}, 'finish_reason': None}]},
              {'id': 'brain-fault-response', 'object': 'chat.completion.chunk', 'model': model,
               'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'stop'}]}]
    return (''.join('data: ' + json.dumps(chunk) + '\n\n' for chunk in chunks) + 'data: [DONE]\n\n').encode()


class ModelFault:
    def __init__(self, env, config):
        self.env = env
        self.config = config
        self.provider = config['model'].split('/')[0]
        self.upstream = config['providers'][self.provider]['base_url'].rstrip('/')
        self.scripts = {}
        self.calls = {}
        self.closed = threading.Event()

    def __enter__(self):
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                raw = self.rfile.read(int(self.headers['Content-Length']))
                body = json.loads(raw)
                matched = [identifier for identifier in owner.scripts if identifier in raw.decode()]
                try:
                    require(len(matched) <= 1, 'ambiguous model fault target')
                    if matched:
                        identifier = matched[0]
                        count = owner.calls.get(identifier, 0) + 1
                        owner.calls[identifier] = count
                        owner.env.save('faults/model-calls', owner.calls)
                        text = owner.scripts[identifier](count, body, owner.closed)
                        data = chat_stream(text, body.get('model', 'acceptance'))
                        self.send_response(200)
                        self.send_header('Content-Type', 'text/event-stream')
                        self.send_header('Content-Length', str(len(data)))
                        self.end_headers()
                        self.wfile.write(data)
                    else:
                        # Unmatched calls retain the original model transport.
                        headers = {k: v for k, v in self.headers.items()
                                   if k.lower() not in ('host', 'content-length', 'connection', 'accept-encoding')}
                        suffix = self.path.removeprefix('/v1')
                        request = urllib.request.Request(owner.upstream + suffix, data=raw, headers=headers)
                        with owner.env.opener.open(request, timeout=330) as response:
                            self.send_response(response.status)
                            self.send_header('Content-Type', response.headers.get('Content-Type', 'application/json'))
                            self.end_headers()
                            while chunk := response.read(4096):
                                self.wfile.write(chunk)
                                self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
                except Exception as error:
                    owner.env.save('faults/proxy-error', {'type': type(error).__name__})
                    self.send_error(502, 'acceptance proxy request failed')

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f'http://127.0.0.1:{self.server.server_port}/v1'
        return self

    def create(self, label, plan, responder):
        identifier = self.env.tag + '-' + label
        self.scripts[identifier] = responder
        env = self.env
        root = env.create(label, plan)
        env.api('POST', f'/api/brain/runs/{root}/commands', {'action': 'pause'})
        require(not env.view(root)['operations'], 'initial real model outran fault preparation')
        def released_slot():
            try:
                require_restartable(env)
                return True
            except NotRun:
                return False
        env.wait(released_slot, 330, 'paused root releases its previous model slot')
        with FrozenWindow(env):
            require(not any(blockers(env, allow_frozen=True).values()), 'unrelated execution prevents scoped model fault')
            require_restartable(env)
            unit = env.record['runtime_unit']
            # A normal, quiescent exit preserves the paused root. SIGKILL is a
            # separate failure case and must never prepare a resumable fixture.
            subprocess.run(['systemctl', 'stop', unit], check=True, timeout=90)
            require(subprocess.check_output(['systemctl', 'show', unit, '-p', 'MainPID', '--value']).strip() == b'0',
                    'Runtime must be stopped before modifying the owned fixture record')
            # Fault injection changes ONLY this test root's frozen model transport.
            # Production config, credentials, decisions, attempts and deadlines stay intact.
            record = journal(env, root)
            record['queue']['config']['providers'][self.provider]['base_url'] = self.url
            path = Path(env.record['runtime_data']) / 'brain' / root / 'execution.json'
            atomic_bytes(path, (json.dumps(record) + '\n').encode(), 0o600)
            subprocess.run(['systemctl', 'start', unit], check=True, timeout=90)
        env.api('POST', f'/api/brain/runs/{root}/commands', {'action': 'resume'})
        env.save('faults/scoped-model-target', {'run_id': root, 'transport_only': True})
        return root

    def __exit__(self, *_):
        self.closed.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


def atomic_bytes(path, data, mode):
    temporary = Path(str(path) + '.brain-acceptance.tmp')
    with temporary.open('xb') as target:
        temporary.chmod(mode)
        target.write(data)
    temporary.replace(path)
