"""Loopback model and releasable WASI workload for process handoff tests."""
import json
import shlex
import threading
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


# A real WASI invocation remains inside its original Runtime until the test
# creates `release` in this DAG's context root. Each poll sleeps for 10ms.
HOLD_WASM = '''(module
 (import "wasi_snapshot_preview1" "path_filestat_get" (func $stat (param i32 i32 i32 i32 i32) (result i32)))
 (import "wasi_snapshot_preview1" "poll_oneoff" (func $poll (param i32 i32 i32 i32) (result i32)))
 (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
 (memory (export "memory") 1)
 (data (i32.const 512) "release") (data (i32.const 520) "kept-running\\0a")
 (func (export "_start")
  (i32.store (i32.const 256) (i32.const 520)) (i32.store (i32.const 260) (i32.const 13))
  (drop (call $write (i32.const 1) (i32.const 256) (i32.const 1) (i32.const 264)))
  (i32.store (i32.const 16) (i32.const 1)) (i64.store (i32.const 24) (i64.const 10000000))
  (block $done (loop $again
   (br_if $done (i32.eqz (call $stat (i32.const 3) (i32.const 0) (i32.const 512) (i32.const 7) (i32.const 128))))
   (drop (call $poll (i32.const 0) (i32.const 64) (i32.const 1) (i32.const 112))) (br $again)))
  (drop (call $write (i32.const 1) (i32.const 256) (i32.const 1) (i32.const 264)))))'''
