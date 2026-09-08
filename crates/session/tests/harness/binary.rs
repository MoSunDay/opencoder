use std::path::Path;

pub fn fake_binary(root: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let path = bin.join("codex");
    std::fs::write(&path, r##"#!/usr/bin/python3
import json, os, sys, time
if '--version' in sys.argv:
    print('codex-cli fixture'); sys.exit(0)
prompt = sys.stdin.read()
with open(os.environ['CAPTURE'], 'a') as f:
    f.write(json.dumps({'args':sys.argv[1:], 'prompt':prompt, 'env':os.environ.get('EXAMPLE'), 'cwd':os.getcwd()})+'\n')
def emit(v): print(json.dumps(v), flush=True)
thread = 'fork-thread' if 'fork' in sys.argv else 'fixture-thread'
emit({'type':'thread.started','thread_id':thread})
emit({'type':'turn.started'})
if os.environ.get('FAIL_MODE') == 'malformed':
    print('invalid-json', flush=True); time.sleep(20); sys.exit(1)
emit({'type':'item.completed','item':{'id':'r1','type':'reasoning','text':'inspect first'}})
emit({'type':'item.started','item':{'id':'c1','type':'command_execution','command':'inspect resources','aggregated_output':'','status':'in_progress','exit_code':None}})
if os.environ.get('FAIL_MODE') == 'hang' or (os.environ.get('FAIL_MODE') == 'steer' and 'resume' not in sys.argv):
    import subprocess
    time.sleep(3) # Exercise readiness after the former two-second startup deadline.
    child = subprocess.Popen(['sleep','120'])
    with open(os.environ['CHILD_PID'],'w') as f: f.write(str(child.pid))
    time.sleep(120)
if 'read resources' in prompt:
    import pathlib, re
    tool_dirs = re.findall(r'^- tools: (.+)$', prompt, flags=re.M)
    if tool_dirs:
        import subprocess
        assert subprocess.check_output([str(pathlib.Path(tool_dirs[0])/'probe')]).strip() == b'TOOL_OK'
    assert 'SOUL_FIXTURE' in prompt and 'HOW_FIXTURE' in prompt and 'OUTPUT_FIXTURE' in prompt and 'MEMORY_FIXTURE' in prompt
    for cat in ['skills','memory']:
        dirs = re.findall(r'^- '+cat+r': (.+)$', prompt, flags=re.M)
        assert dirs and pathlib.Path(dirs[0]).is_dir()
emit({'type':'item.completed','item':{'id':'c1','type':'command_execution','command':'inspect resources','aggregated_output':'tool result','status':'completed','exit_code':0}})
emit({'type':'item.completed','item':{'id':'a1','type':'agent_message','text':'answer'}})
if os.environ.get('FAIL_MODE') == 'missing_end': sys.exit(0)
emit({'type':'turn.completed','usage':{'input_tokens':12,'output_tokens':5,'cached_input_tokens':3}})
"##).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}
