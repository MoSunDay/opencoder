#!/usr/bin/env python3
"""Real Nginx + two Servers/Hosts + three systemd Runtime processes.

Run as root after building: python3 .../main.py --bin-dir <debug-or-bundle/bin>
--nginx <nginx>. Fixtures and databases are retained for inspection. No
production service, credential, database, mount, or admission is changed.
"""
import argparse
import json
from pathlib import Path
import threading
import time
import traceback
import urllib.error
import urllib.request
from environment import Environment, until
from fixture import todo_spec
from streams import Stream
from metrics import verify as verify_traffic
from rolling import probes
from ingress_requests import LateRequest
from ingress_requests.node import NodeChannel


def done(env, identifier):
    value = env.api('/api/executions/' + identifier)
    status = value['execution']['status']
    if status in ['error','cancelled','interrupted']:
        raise RuntimeError(json.dumps(value))
    return value if status in ['done','idle'] else None


def exercise(env):
    first = env.warm('r1')
    first_skill = Path(first['runtime_data']) / 'global-skills/release-reference/SKILL.md'
    assert 'first release bytes' in first_skill.read_text()
    env.switch(first)
    print('first release ready',flush=True)
    probes.public(env.settings,first,env,90)
    todo = {'id':'todos-release-chain','kind':'todos','input':{'spec':todo_spec()}}
    receipt = env.api('/api/executions','POST',todo)
    assert env.model.entered.wait(90), 'TODO dependency did not reach the model'
    dag = {'id':'dag-hold','kind':'dag','input':{'definition':{'name':'hold',
        'steps':[{'name':'hold','timeout_secs':900,'kind':{'type':'wasm','command':'hold.wasm'}}]}}}
    env.api('/api/executions','POST',dag)
    until(lambda:env.api('/api/executions/dag-hold')['dag_steps']['running'] == 1,'live WASI step')
    stream = Stream(env,'dag-hold')
    until(lambda:len(stream.ids) > 0,'initial SSE cursor')
    pid = env.runtime_pid(first)
    shell = {'id':'agent-release-shell','kind':'agent','target':'act',
        'input':{'prompt':'smooth-release-shell','harness':'opencoder'}}
    env.api('/api/executions','POST',shell)
    shell_pid = int(until(lambda:(env.root / 'shell.pid').read_text(),'real shell tool started'))
    shell_start = Path(f'/proc/{shell_pid}/stat').read_text().split()[21]
    def unchanged_shell():
        assert Path(f'/proc/{shell_pid}/stat').read_text().split()[21] == shell_start, 'shell tool process changed'
        assert env.owner(shell['id']) == 'r1'
    container = None
    if env.containers.wasmtime:
        container_dag = json.loads(json.dumps(dag))
        container_dag['id'] = 'dag-container-hold'
        container_dag['input']['definition']['steps'][0]['kind']['sandbox'] = 'runc'
        env.api('/api/executions','POST',container_dag)
        container = until(lambda:(lambda s:s if s['status'] == 'running' else None)(env.containers.state(first['runtime_data'],'dag-container-hold')),'real OCI container started')
    def unchanged_container():
        if container:
            current = env.containers.state(first['runtime_data'],'dag-container-hold')
            assert current['status'] == 'running' and (current['pid'],current['process_start']) == (container['pid'],container['process_start']), 'OCI process changed'
    print('old TODO and WASI tasks running',flush=True)
    # Cold admission may first snapshot the resource pool. Verify that path
    # before measuring uninterrupted requests across release transitions.
    preflight = 'dag-traffic-preflight'
    preflight_started = time.monotonic()
    env.api('/api/executions','POST',{'id':preflight,'kind':'dag','input':{'definition':probes.spec()}})
    until(lambda:done(env,preflight),'traffic preflight completion')
    (env.root / 'traffic-preflight.json').write_text(json.dumps({
        'id':preflight,'seconds':time.monotonic()-preflight_started},indent=2))
    late_id = 'dag-accepted-before-ingress-reload'
    late, node_channel = None, None
    traffic, failures = [], []
    stop = threading.Event()
    def submit():
        while not stop.is_set():
            identifier = 'dag-traffic-' + str(len(traffic))
            started = time.monotonic()
            try:
                env.api('/api/executions','POST',{'id':identifier,'kind':'dag','input':{'definition':probes.spec()}})
                traffic.append({'id':identifier,'seconds':time.monotonic()-started,'at':started})
            except Exception as error:
                failures.append(str(error))
                break
            stop.wait(.1)
    thread = threading.Thread(target=submit,daemon=True)
    thread.start()
    try:
        env.shared_skill.write_text(env.shared_skill.read_text().replace('first release bytes','second release bytes'))
        second = env.warm('r2')
        assert 'second release bytes' in (Path(second['runtime_data']) / 'global-skills/release-reference/SKILL.md').read_text()
        assert 'first release bytes' in first_skill.read_text(), 'new startup changed old Runtime skills'
        late = LateRequest(env, {'id':late_id,'kind':'dag','input':{'definition':probes.spec()}})
        node_channel = NodeChannel(env, f"http://127.0.0.1:{first['host_port']}")
        env.switch(second)
        print('second release active',flush=True)
        assert env.retire(first), 'Server cannot retire while ingress still owns accepted requests'
        env.http(env.settings.host_url,'/servers/r1','POST',{
            'url':f"http://127.0.0.1:{first['server_port']}",'enabled':False})
        until(lambda:len(stream.reconnects) == 1,'SSE release notification')
        until(lambda:len(stream.resume_delays) == 1,'SSE resumed response')
        assert stream.resume_delays[0] < 5, 'SSE reconnect exceeded 5 seconds'
        until(lambda:node_channel.closed.is_set() or node_channel.errors,'Node retirement notification',5)
        assert not node_channel.errors, node_channel.errors
        assert node_channel.closed.is_set(), 'Node channel prevents ingress retirement'
        late.finish()
        assert env.owner(late_id) == 'r2', 'late ingress request missed the active Runtime'
        def retired():
            code = env.children['r1-server'].poll()
            assert code in [None, 0], f'retired Server exited with {code}'
            return code == 0
        until(retired,'old ingress and Server exit',5)
        assert env.runtime_pid(first) == pid, 'old execution process changed'
        unchanged_shell()
        unchanged_container()
        assert env.owner(todo['id']) == 'r1'
        assert env.api('/api/executions','POST',todo) == receipt
        try:
            env.api('/api/executions','POST',{**todo,'input':{'spec':{**todo_spec(),'name':'changed'}}})
            raise AssertionError('changed request reused the old ID')
        except urllib.error.HTTPError as error:
            assert error.code == 409
        third = env.warm('r3')
        env.switch(third)
        print('third release active',flush=True)
        assert env.runtime_pid(first) == pid
        unchanged_shell()
        unchanged_container()
        assert env.model.calls == ['first'], env.model.calls
        env.api('/api/executions','POST',{'id':'dag-latest','kind':'dag','input':{'definition':probes.spec()}})
        until(lambda:done(env,'dag-latest'),'new release execution')
        assert env.owner('dag-latest') == 'r3'
        env.api('/api/executions','POST',{**dag,'id':'dag-r3-hold'})
        until(lambda:env.api('/api/executions/dag-r3-hold')['dag_steps']['running'] == 1,'third-version WASI task')
        third_pid = env.runtime_pid(third)
        env.reopen(second)
        env.switch(second)
        print('rollback active with both old and new work running',flush=True)
        env.api('/api/executions','POST',{'id':'dag-return','kind':'dag','input':{'definition':probes.spec()}})
        until(lambda:done(env,'dag-return'),'rollback new task')
        assert env.owner('dag-return') == 'r2'
        assert env.runtime_pid(first) == pid and env.runtime_pid(third) == third_pid
        unchanged_shell()
        unchanged_container()
        assert env.owner('dag-r3-hold') == 'r3'
        stop.set()
        thread.join(35)
        env.crash_server(second)
        assert env.api('/api/executions','POST',todo) == receipt
        assert env.runtime_pid(first) == pid and env.runtime_pid(third) == third_pid
        unchanged_shell()
        unchanged_container()
    finally:
        if late:
            late.close()
        if node_channel:
            node_channel.close()
        stop.set()
        thread.join(35)
        (env.root / 'traffic.json').write_text(json.dumps({'requests':traffic,'failures':failures},indent=2))
        env.release_work()
    until(lambda:done(env,todo['id']),'old TODO chain completion')
    until(lambda:done(env,shell['id']),'old shell tool completion')
    if container:
        until(lambda:done(env,'dag-container-hold'),'old OCI tool completion')
    until(lambda:done(env,dag['id']),'old WASI completion')
    until(lambda:done(env,'dag-r3-hold'),'rolled-back version task completion')
    assert env.model.calls == ['first','second'], env.model.calls
    until(lambda:stream.finished.is_set() or stream.errors,'SSE completion')
    assert not stream.errors, stream.errors
    assert len(stream.ids) == len(set(stream.ids)), 'duplicate event cursor'
    expected, cursor = [], 0
    while True:
        page = env.api('/api/executions/dag-hold/events-page?after=' + str(cursor))
        rows = page['events']
        if not rows:
            break
        expected.extend(row['seq'] for row in rows)
        cursor = expected[-1]
    assert stream.ids == expected, 'SSE replay lost or changed persisted events'
    for row in traffic:
        until(lambda:done(env,row['id']),'traffic completion')
    until(lambda:done(env,late_id),'accepted ingress request completion')
    continuity = verify_traffic([record['runtime_data'] for record in env.records], traffic)
    (env.root / 'scheduling.json').write_text(json.dumps(continuity,indent=2))
    status = until(lambda:(lambda v:v if v['capacity']['running'] == 0 and v['capacity']['queued'] == 0 else None)(env.http(env.settings.host_url,'/status')),'global capacity release')
    assert status['capacity']['running'] == 0 and status['capacity']['queued'] == 0
    assert status['capacity']['max_runs'] == 6
    until(lambda:env.runtime_pid(first) == 0,'retired Runtime hibernation')
    assert done(env,todo['id']), 'hibernated history wake failed'
    assert env.runtime_pid(first) != 0
    assert not failures, failures
    assert traffic, 'traffic fixture never submitted'
    result = {'result':'PASS','build':env.info,'cases':['three-runtime-processes','two-host-handovers',
        'real-wasi-continues','shell-process-continues','pinned-global-skills','todo-dependency-chain','continuous-submission','request-replay-conflict',
        'sse-cursor-reconnect','hibernate-history-wake','rollback-with-live-new-work','server-sigkill-recovery',
        'independent-readonly-nfs','accepted-ingress-request-before-reload','node-channel-releases-ingress-worker'],'traffic':traffic,'failures':failures,
        'todo_calls':env.model.calls,'sse_ids':stream.ids,'sse_resume_seconds':stream.resume_delays,
        'continuity':continuity['metrics'],
        'runtime_pid_before':pid,'shell_pid':shell_pid,'shell_start':shell_start}
    if container:
        result['cases'].append('real-oci-container-continues')
        result['container'] = container
    (env.root / 'result.json').write_text(json.dumps(result,indent=2))
    print(json.dumps({'result':'PASS','evidence':str(env.root),'submissions':len(traffic),
        'max_accept_seconds':max(t['seconds'] for t in traffic)}),flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir',type=Path,required=True)
    parser.add_argument('--nginx',type=Path,required=True)
    parser.add_argument('--data-parent',type=Path,
        help='Optional isolated fixture storage; production latency acceptance must use production storage')
    parser.add_argument('--wasmtime',type=Path,help='Verified wasmtime executable for real OCI continuation acceptance')
    args = parser.parse_args()
    env = Environment(args.bin_dir.resolve(),args.nginx.resolve(),args.data_parent,args.wasmtime)
    try:
        exercise(env)
    except BaseException:
        (env.root / 'failure.txt').write_text(traceback.format_exc())
        env.diagnose()
        raise
    finally:
        env.close()


if __name__ == '__main__':
    main()
