#!/usr/bin/python3
import sys, json, os, pathlib, hashlib, subprocess
if '--version' in sys.argv:
    print('codex-cli fixture'); sys.exit(0)
v = json.loads(sys.stdin.readline())
root = pathlib.Path.cwd()
with open(root/'starts', 'a') as f: f.write('1\n')
(root/'invocation.json').write_text(json.dumps(v))
def emit(value): print(json.dumps(value), flush=True)
def event(value): emit({'type':'codex','phase':'review','event':value})
mode = os.environ['MODE'].split(':', 1)[1]
emit({'type':'stage','stage':'review','detail':v['input']})
if mode == 'invalid': print('not-json', flush=True); sys.exit(0)
if mode == 'failure': emit({'type':'error','error':'fixture failure'}); sys.exit(1)
event({'type':'thread.started','thread_id':'fixture-thread'})
event({'type':'turn.started'})
event({'type':'item.completed','item':{'id':'r1','type':'reasoning','text':'inspect first'}})
event({'type':'item.started','item':{'id':'c1','type':'command_execution','command':'inspect','aggregated_output':'','status':'in_progress','exit_code':None}})
if mode == 'hang':
    child = subprocess.Popen(['sleep','120'])
    (root/'child.pid').write_text(str(child.pid))
    sys.stdin.readline()
    child.terminate(); child.wait(); sys.exit(1)
event({'type':'item.completed','item':{'id':'c1','type':'command_execution','command':'inspect','aggregated_output':'tool result','status':'completed','exit_code':0}})
event({'type':'item.completed','item':{'id':'a1','type':'agent_message','text':'answer'}})
if mode != 'unfinished': event({'type':'turn.completed','usage':{'input_tokens':12,'output_tokens':5,'cached_input_tokens':3}})
if mode == 'missing': sys.exit(0)
result = pathlib.Path(v['output_dir'])/'result.json'
result.write_text(json.dumps({'verdict':'block','summary':'Business blocking result, successful execution'}))
hash = hashlib.sha256(result.read_bytes()).hexdigest()
if mode == 'tamper': result.write_text('{}')
emit({'type':'result','result_file':'result.json','artifacts':[{'file':'result.json','sha256':hash}]})
