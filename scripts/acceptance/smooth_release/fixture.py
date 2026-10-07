"""Loopback model and releasable native workload for process handoff tests."""
import json
from pathlib import Path
import subprocess
import tempfile
import shlex
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def field(prompt, name):
    line = next((s for s in prompt.splitlines() if s.startswith(name + '=')), None)
    return json.loads(line[len(name) + 1:]) if line else None


class Model:
    def __init__(self, root):
        self.release = threading.Event()
        self.entered = threading.Event()
        self.calls = []
        script = '\n'.join(['import os, pathlib, time',
            f'pathlib.Path({str(root / "shell.pid")!r}).write_text(str(os.getpid()))',
            f'while not pathlib.Path({str(root / "shell.release")!r}).exists(): time.sleep(.1)',
            'print("shell kept running")'])
        self.shell_command = 'python3 -c ' + shlex.quote(script)
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_POST(self):
                request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                messages = request.get('messages', [])
                prompt = next((m['content'] for m in reversed(messages) if m['role'] == 'user'), '')
                if isinstance(prompt, list):
                    prompt = '\n'.join(p.get('text', '') for p in prompt)
                if 'smooth-release-shell' in prompt and not any(m.get('role') == 'tool' for m in messages):
                    frames = [({'role':'assistant','tool_calls':[{'index':0,'id':'release-shell-call',
                        'type':'function','function':{'name':'bash','arguments':json.dumps({'command':owner.shell_command})}}]},None),
                        ({},'tool_calls')]
                else:
                    answer = owner.answer(prompt)
                    text = answer if isinstance(answer, str) else json.dumps(answer)
                    frames = [({'role':'assistant','content':text},None),({},'stop')]
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.end_headers()
                for delta, reason in frames:
                    self.wfile.write(('data: ' + json.dumps({'choices':[{'index':0,'delta':delta,'finish_reason':reason}]}) + '\n\n').encode())
                self.wfile.write(b'data: [DONE]\n\n')

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def answer(self, prompt):
        if 'Decide the next workflow operation' in prompt:
            ready = field(prompt, 'RUNNABLE')
            return {'operation':'dispatch','todos':[{'todo_id':ready[0],'context_mode':'new'}],'reason':'dependency ready'} if ready else {'operation':'complete','reason':'all accepted'}
        if 'Accept or reject one TODO candidate' in prompt:
            return {'operation':'accept','reason':'verified result','mark_milestone':False}
        if 'Complete exactly one focused TODO' in prompt:
            todo = field(prompt, 'TODO')
            self.calls.append(todo['id'])
            if todo['id'] == 'first' and len(self.calls) == 1:
                self.entered.set()
                if not self.release.wait(900):
                    raise TimeoutError('handoff did not release the model fixture')
            return {'status':'candidate','summary':todo['id']+' complete','result':todo['id']+' evidence',
                'verification':'fixture verified','evidence_refs':['result.txt'],'recovery_context':{'summary':'complete','refs':[]}}
        return 'smooth release model result'

    def config(self):
        return {'model':'fixture/model','cache_salt':False,'providers':{'fixture':{
            'base_url':f'http://127.0.0.1:{self.server.server_port}/v1','api_key':'local-fixture'}}}


def todo_spec():
    return {'schema_version':1,'id':'release-chain','name':'release chain','objective':'preserve dependencies across releases',
        'todos':[{'id':name,'title':name,'depends_on':depends,'agent':'act',
            'requirement_background':'release acceptance','instructions':'return verifiable evidence','max_attempts':2,'acceptance':{'criteria':'result complete'}}
            for name, depends in [('first',[]),('second',['first'])]]}


def release_native_gate(runtime_root, identifier, seconds=120):
    journal = runtime_root / 'dag' / identifier / 'execution.json'
    deadline = time.monotonic() + seconds
    while True:
        if journal.is_file():
            record = json.loads(journal.read_text())
            run = Path(record['annotations']['dag_parent']) / identifier
            container = run / 'container.json'
            if container.is_file():
                identity = json.loads(container.read_text())['id']
                # Runtime services own a private mount namespace. Release the
                # fixture through its container, where the overlay is mounted.
                result = subprocess.run(['runc', '--root', str(run / 'runc-state'),
                    'exec', identity, '/bin/sh', '-c',
                    'test -d /workspace/hold && : > /workspace/hold/release'],
                    capture_output=True, timeout=10)
                if result.returncode == 0:
                    return
        if time.monotonic() >= deadline:
            raise TimeoutError('cannot release unconfirmed native admission: ' + identifier)
        time.sleep(.1)


HOLD_SOURCE = r'''#include <stdio.h>
#include <unistd.h>
int main(void) {
    puts("kept-running");
    fflush(stdout);
    while (access("release", F_OK) != 0) usleep(10000);
    puts("kept-running");
    return 0;
}
'''


def publish_hold(environment):
    from rolling.native import publish_binary
    with tempfile.TemporaryDirectory() as directory:
        source = Path(directory) / 'hold.c'
        binary = Path(directory) / 'hold'
        source.write_text(HOLD_SOURCE)
        subprocess.run(['cc','-O2','-static','-s','-Wl,--build-id=none',str(source),'-o',str(binary)], check=True)
        return publish_binary(environment.settings.resource_url, 'release-hold', binary.read_bytes(), environment)
