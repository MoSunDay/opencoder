"""Drive the operator CLI, including signal rollback with live new work."""
from pathlib import Path
import subprocess
import sys
from rolling import probes
from rolling.state import Journal, atomic_bytes
from fixture import HOLD_WASM


def command(args, rollback=False):
    cli = Path(__file__).resolve().parents[2] / 'platform/rolling_cli.py'
    result = [sys.executable, str(cli), '--config', str(args.config), '--wait-seconds', '300']
    if getattr(args, 'signal', False):
        result.append('--signal')
    return [*result, '--rollback'] if rollback else [*result, '--bundle', str(args.bundle)]


def execute(args, root, env, continuity):
    def invoke(label, rollback=False):
        with (root / (label + '.log')).open('wb') as log:
            subprocess.run(command(args, rollback), stdout=log, stderr=subprocess.STDOUT, check=True)
        continuity()

    invoke('deployment')
    if not getattr(args, 'signal_roundtrip', False):
        return
    current = Journal(env.settings.state_dir).data
    record = current['releases'][current['current']]
    data = Path(record['runtime_data'])
    identifier = 'dag-' + root.name + '-new-hold'
    module = identifier + '.wasm'
    atomic_bytes(data / 'dag/_modules' / module, HOLD_WASM.encode(), 0o444)
    runtime_pid = subprocess.check_output(['systemctl','show',record['runtime_unit'],'-p','MainPID','--value']).strip()
    env.api('/api/executions','POST',{'id':identifier,'kind':'dag','input':{'definition':{
        'name':'signal rollback continuation','steps':[{'name':'hold','timeout_secs':1800,
        'kind':{'type':'wasm','command':module}}]}}})
    try:
        env.wait(lambda:env.api('/api/executions/' + identifier)['dag_steps']['running'] == 1,90)
        for label, rollback in [('signal-rollback',True),('signal-republish',False)]:
            invoke(label,rollback)
            assert subprocess.check_output(['systemctl','show',record['runtime_unit'],'-p','MainPID','--value']).strip() == runtime_pid
            assert env.api('/api/executions/' + identifier)['dag_steps']['running'] == 1
            active = Journal(env.settings.state_dir).data
            if not rollback:
                assert active['releases'][record['id']]['server_unit'] != record['server_unit'], 'republish reused a retiring Server'
                assert active['releases'][record['id']]['host_unit'] != record['host_unit'], 'republish reused a retiring Host'
            probe = 'dag-' + root.name + '-' + label
            env.api('/api/executions','POST',{'id':probe,'kind':'dag','input':{'definition':probes.spec()}})
            env.wait(lambda:env.completed(probe),30)
            target = Path(active['releases'][active['current']]['runtime_data'])
            assert (target / 'dag' / probe / 'execution.json').is_file(), 'post-signal task has incorrect owner'
    finally:
        for context in (data / 'dag' / identifier).glob('*/context.json'):
            (context.parent.parent / 'release').touch()
    env.wait(lambda:env.completed(identifier),90)
