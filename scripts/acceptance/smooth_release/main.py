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
from rolling import probes


def done(env, identifier):
    value = env.api('/api/executions/' + identifier)
    status = value['execution']['status']
    if status in ['error','cancelled','interrupted']:
        raise RuntimeError(json.dumps(value))
    return value if status in ['done','idle'] else None


class Stream:
    def __init__(self, env, identifier):
        self.ids = []
        self.reconnects = []
        self.resume_delays = []
        self.errors = []
        self.finished = threading.Event()
        def consume():
            try:
                cursor = 0
                disconnected = None
                for _ in range(8):
                    request = urllib.request.Request(env.settings.public_url + '/api/executions/' + identifier + '/events?after=' + str(cursor),
                        headers={'Authorization':'Bearer ' + env.token,'Last-Event-ID':str(cursor)})
                    with env.opener.open(request,timeout=90) as response:
                        if disconnected is not None:
                            self.resume_delays.append(time.monotonic() - disconnected)
                            disconnected = None
                        event, seq = None, None
                        for raw in response:
                            line = raw.decode().strip()
                            if line.startswith('id:'):
                                seq = int(line[3:])
                            elif line.startswith('event:'):
                                event = line[6:].strip()
                            elif not line:
                                if event == 'reconnect':
                                    self.reconnects.append(time.monotonic())
                                    disconnected = time.monotonic()
                                    break
                                if seq is not None:
                                    assert seq > cursor, (seq,cursor)
                                    cursor = seq
                                    self.ids.append(seq)
                                if event == 'stream_end':
                                    self.finished.set()
                                    return
                                event, seq = None, None
                raise RuntimeError('event stream did not finish')
            except Exception as error:
                self.errors.append(str(error))
        self.thread = threading.Thread(target=consume,daemon=True)
        self.thread.start()


def exercise(env):
    first = env.warm('r1')
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
    print('old TODO and WASI tasks running',flush=True)
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
        second = env.warm('r2')
        env.switch(second)
        print('second release active',flush=True)
        env.http(f"http://127.0.0.1:{first['server_port']}",'/api/admin/release/retire','POST',{})
        env.http(env.settings.host_url,'/servers/r1','POST',{
            'url':f"http://127.0.0.1:{first['server_port']}",'enabled':False})
        until(lambda:len(stream.reconnects) == 1,'SSE release notification')
        until(lambda:len(stream.resume_delays) == 1,'SSE resumed response')
        assert stream.resume_delays[0] < 5, 'SSE reconnect exceeded 5 seconds'
        assert env.runtime_pid(first) == pid, 'old execution process changed'
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
        assert env.owner('dag-r3-hold') == 'r3'
        stop.set()
        thread.join(35)
        env.crash_server(second)
        assert env.api('/api/executions','POST',todo) == receipt
        assert env.runtime_pid(first) == pid and env.runtime_pid(third) == third_pid
    finally:
        stop.set()
        thread.join(35)
        (env.root / 'traffic.json').write_text(json.dumps({'requests':traffic,'failures':failures},indent=2))
        env.release_work()
    until(lambda:done(env,todo['id']),'old TODO chain completion')
    until(lambda:done(env,dag['id']),'old WASI completion')
    until(lambda:done(env,'dag-r3-hold'),'rolled-back version task completion')
    assert env.model.calls == ['first','second'], env.model.calls
    until(lambda:stream.finished.is_set() or stream.errors,'SSE completion')
    assert not stream.errors, stream.errors
    assert len(stream.ids) == len(set(stream.ids)), 'duplicate event cursor'
    for row in traffic:
        until(lambda:done(env,row['id']),'traffic completion')
    status = until(lambda:(lambda v:v if v['capacity']['running'] == 0 and v['capacity']['queued'] == 0 else None)(env.http(env.settings.host_url,'/status')),'global capacity release')
    assert status['capacity']['running'] == 0 and status['capacity']['queued'] == 0
    assert status['capacity']['max_runs'] == 4
    until(lambda:env.runtime_pid(first) == 0,'retired Runtime hibernation')
    assert done(env,todo['id']), 'hibernated history wake failed'
    assert env.runtime_pid(first) != 0
    assert not failures, failures
    assert traffic, 'traffic fixture never submitted'
    result = {'result':'PASS','build':env.info,'cases':['three-runtime-processes','two-host-handovers',
        'real-wasi-continues','todo-dependency-chain','continuous-submission','request-replay-conflict',
        'sse-cursor-reconnect','hibernate-history-wake','rollback-with-live-new-work','server-sigkill-recovery',
        'independent-readonly-nfs'],'traffic':traffic,'failures':failures,
        'todo_calls':env.model.calls,'sse_ids':stream.ids,'sse_resume_seconds':stream.resume_delays,'runtime_pid_before':pid}
    (env.root / 'result.json').write_text(json.dumps(result,indent=2))
    print(json.dumps({'result':'PASS','evidence':str(env.root),'submissions':len(traffic),
        'max_accept_seconds':max(t['seconds'] for t in traffic)}),flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir',type=Path,required=True)
    parser.add_argument('--nginx',type=Path,required=True)
    args = parser.parse_args()
    env = Environment(args.bin_dir.resolve(),args.nginx.resolve())
    try:
        exercise(env)
    except BaseException:
        (env.root / 'failure.txt').write_text(traceback.format_exc())
        raise
    finally:
        env.close()


if __name__ == '__main__':
    main()
