"""Private real processes; only uniquely named test Runtime units are managed."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling.config import Settings
from rolling import ingress, probes, units
from rolling.state import atomic_bytes, write
from fixture import Model, HOLD_WASM
from resources import Resources
from containers import Containers


_ports = set()


def port():
    # Keep listener choices outside this host's ephemeral client-port range.
    # A bind(0) port released during warmup can be reused by a long-lived RPC.
    low = int(Path('/proc/sys/net/ipv4/ip_local_port_range').read_text().split()[0])
    for _ in range(500):
        candidate = 2000 + secrets.randbelow(min(low, 10000) - 2000)
        if candidate in _ports:
            continue
        with socket.socket() as sock:
            try:
                sock.bind(('127.0.0.1',candidate))
            except OSError:
                continue
        _ports.add(candidate)
        return candidate
    raise RuntimeError('no isolated listener port is available')


def until(check, label, seconds=90):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            value = check()
            if value:
                return value
        except (OSError, ValueError, KeyError) as error:
            last = str(error)
        time.sleep(.1)
    raise TimeoutError(f'{label}: {last}')


class Environment:
    def __init__(self, binaries, nginx, data_parent=None, wasmtime=None):
        self.root = Path(tempfile.mkdtemp(prefix='opencoder-smooth-',dir=data_parent))
        self.root.chmod(0o755)
        print(json.dumps({'evidence':str(self.root)}), flush=True)
        self.prefix = self.root.name
        self.children = {}
        self.records = []
        self.nginx = nginx
        self.model = Model(self.root)
        self.containers = Containers(wasmtime)
        self.shared_skill = self.root / '.opencoder/skills/release-reference/SKILL.md'
        self.shared_skill.parent.mkdir(parents=True)
        self.shared_skill.write_text('---\nname: release-reference\ndescription: fixture\n---\nfirst release bytes\n')
        self.resources = Resources(self.root,port)
        self.token = secrets.token_hex(24)
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        # The caller supplies an immutable bundle (or a completed debug build)
        # and keeps those binaries unchanged for the duration of this run.
        self.bin = binaries
        self.info = json.loads(subprocess.check_output([self.bin / 'opencoder-agent','--build-info']))
        write(self.root / 'build-info.json',self.info)
        self.settings = Settings(self.root / 'state',self.root / 'server-work',self.root / 'server-data',
            self.root / 'agent-work',self.root / 'token',public_url=f'http://127.0.0.1:{port()}',
            host_port=port(),resource_port=port(),nginx_include=self.root / 'ingress.conf',max_runs=6)
        s = self.settings
        self.settings = Settings(**{**s.__dict__,'listen':s.public_url.removeprefix('http://')})
        atomic_bytes(s.token_file,self.token.encode())
        for directory in [s.server_workdir,s.agent_workdir]:
            resource_config = self.resources.server_config() if directory == s.server_workdir else self.resources.client_config()
            write(directory / 'opencoder.json',{**self.model.config(),**resource_config})
            write(directory / '.opencoder/ap.json',{'mode':'off'})
        self.env = {**os.environ,'HOME':str(self.root),'XDG_CONFIG_HOME':str(self.root / 'config'),
            'XDG_DATA_HOME':str(self.root / 'data')}
        self.start('resources','opencoder-server',['--resources','--workdir',s.server_workdir,
            '--data-dir',self.root / 'resources','--port',s.resource_port,'--token-file',s.token_file])
        until(lambda:self.http(s.resource_url,'/api/health'),'resource service')
        self.resources.mount()

    def http(self, base, path, method='GET', body=None):
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(base + path,data=data,method=method,headers={
            'Authorization':'Bearer '+self.token,'Content-Type':'application/json'})
        try:
            with self.opener.open(request,timeout=30) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            error.msg += ': ' + error.read(4096).decode(errors='replace')
            raise

    def api(self,path,method='GET',body=None):
        return self.http(self.settings.public_url,path,method,body)

    def diagnose(self):
        evidence = {'processes': {label: {'pid': child.pid, 'exit_code': child.poll()}
                                  for label, child in self.children.items()}}
        if hasattr(self, 'last_ingress_workers'):
            evidence['ingress_workers'] = [{**worker,
                'drained': ingress.drained([worker])} for worker in self.last_ingress_workers]
        for record in self.records:
            probes = {}
            for kind, paths in [('server', ['/api/nodes', '/api/ready']), ('host', ['/status'])]:
                for path in paths:
                    try:
                        probes[kind + path] = self.http(f"http://127.0.0.1:{record[kind + '_port']}", path)
                    except Exception as error:
                        probes[kind + path] = {'error': str(error)}
            evidence[record['id']] = probes
        write(self.root / 'failure-state.json', evidence)

    def start(self,label,binary,args):
        log = (self.root / (label + '.log')).open('ab')
        self.children[label] = subprocess.Popen([self.bin / binary,*map(str,args)],env=self.env,stdout=log,stderr=log)
        log.close()
        return self.children[label]

    def warm(self, label):
        timings = []
        def mark(step):
            timings.append({'step':step,'at':time.monotonic()})
        mark('start')
        s = self.settings
        record = {'id':label,'server_port':port(),'runtime_port':port(),'host_port':port(),
            'runtime_data':str(s.state_dir / label),'manifest':{'commit':self.info['git_commit']},
            'created_at':int(time.time()*1000),'runtime_unit':f'opencoder-runtime-{self.prefix}-{label}.service'}
        self.records.append(record)
        host = f"http://127.0.0.1:{record['host_port']}"
        self.start(label+'-host','opencoder-agent',['--name',self.prefix,'--data-dir',s.state_dir / 'host',
            '--workdir',s.agent_workdir,'--remote',s.public_url,'--max-runs',s.max_runs,
            '--token-file',s.token_file,'host','--port',record['host_port'],'--standby'])
        status = until(lambda:self.http(host,'/status'),'candidate Host')
        mark('host_ready')
        node_id = status['node']['id']
        data = Path(record['runtime_data'])
        atomic_bytes(data / 'node-id',node_id.encode())
        write(data / 'host-binding.json',{'database':str(s.state_dir / 'host/host.db'),'runtime_id':label})
        atomic_bytes(data / 'dag/_modules/release-probe.wasm',probes.WASM)
        atomic_bytes(data / 'dag/_modules/hold.wasm',HOLD_WASM.encode())
        self.containers.prepare(data)
        mark('data_prepared')
        config = {'endpoint':f"http://127.0.0.1:{record['runtime_port']}",'data_dir':str(data),'unit':record['runtime_unit']}
        self.http(host,'/runtimes','POST',{'id':label,'release_id':label,'mode':'staged','config':config})
        mark('runtime_registered')
        command = [self.bin / 'opencoder-agent','--workdir',s.agent_workdir,'--data-dir',data,
            '--max-runs',65535,'--token-file',s.token_file,'runtime','--port',record['runtime_port']]
        content = units.service(command,'Isolated release acceptance',True)
        content = content.replace('Type=simple','Type=simple\n'+'\n'.join('Environment='+units.argument(k+'='+self.env[k]) for k in ['HOME','XDG_CONFIG_HOME','XDG_DATA_HOME']))
        unit = Path('/etc/systemd/system') / record['runtime_unit']
        atomic_bytes(unit,content.encode(),0o644)
        subprocess.run(['systemd-analyze','verify',str(unit)],check=True)
        mark('unit_verified')
        subprocess.run(['systemctl','daemon-reload'],check=True)
        mark('manager_reloaded')
        subprocess.run(['systemctl','start',record['runtime_unit']],check=True)
        mark('runtime_started')
        probes.candidate(s,record,self,90)
        mark('candidate_ready')
        platform = {'release_id':label,'state_dir':str(s.state_dir),'host_service':s.host_url,'resource_service':s.resource_url}
        write(data / 'release.json',platform)
        self.start(label+'-server','opencoder-server',['--host','127.0.0.1','--port',record['server_port'],
            '--workdir',s.server_workdir,'--data-dir',s.server_data,'--release-config',data / 'release.json','--token-file',s.token_file])
        self.http(host,f'/servers/{label}','POST',{'url':f"http://127.0.0.1:{record['server_port']}",'enabled':True})
        if len(self.records) == 1:
            self.http(host,f'/runtimes/{label}/activate','POST',{})
            self.http(host,'/activate-host','POST',{})
        probes.ready(s,record,node_id,self,90)
        mark('ready')
        (self.root / (label + '-warm-timings.json')).write_text(json.dumps(timings,indent=2))
        return record

    def wait(self,check,seconds=90):
        return until(check,'deployment probe',seconds)

    def switch(self, record):
        s = self.settings
        write(s.state_dir / 'release-state.json',{'schema_version':1,'current':record['id'],
            'candidate':None,'phase':'switching','releases':{r['id']:r for r in self.records}})
        host = f"http://127.0.0.1:{record['host_port']}"
        self.http(host,f"/runtimes/{record['id']}/activate",'POST',{})
        self.http(host,'/activate-host','POST',{})
        self.last_ingress_workers = self.ingress_workers()
        self.last_successor_port = record["server_port"]
        atomic_bytes(s.nginx_include,units.nginx(s,record['server_port'],record['host_port']).encode(),0o644)
        config = self.root / 'nginx.conf'
        if not config.exists():
            config.write_text(f'pid {self.root}/nginx.pid; error_log {self.root}/nginx.log; events {{worker_connections 256;}} http {{ access_log off; include {s.nginx_include}; }}')
            subprocess.run([self.nginx,'-e',str(self.root / 'nginx.log'),'-p',str(self.root),'-c',str(config)],check=True)
        else:
            subprocess.run([self.nginx,'-e',str(self.root / 'nginx.log'),'-p',str(self.root),'-c',str(config),'-t'],check=True)
            subprocess.run([self.nginx,'-e',str(self.root / 'nginx.log'),'-p',str(self.root),'-c',str(config),'-s','reload'],check=True)
        self.http(host,'/commit-host','POST',{})
        until(lambda:self.api('/api/admin/release')['instance_release'] == record['id'],'public version')
        self.resources.check()
        assert self.children['resources'].poll() is None, 'resource service exited during handoff'

    def ingress_workers(self):
        path = self.root / 'nginx.pid'
        return ingress.snapshot(int(path.read_text())) if path.exists() else []

    def ingress_drained(self, workers):
        return ingress.drained(workers)

    def retire(self, record):
        return ingress.retire(self, f"http://127.0.0.1:{record['server_port']}", self.last_ingress_workers, self.last_successor_port)

    def runtime_pid(self,record):
        return int(subprocess.check_output(['systemctl','show',record['runtime_unit'],'-p','MainPID','--value']))

    def reopen(self, record):
        """Warm fresh old-version Server/Host instances for a real rollback."""
        s = self.settings
        record['host_port'], record['server_port'] = port(), port()
        label = record['id'] + '-return'
        host = f"http://127.0.0.1:{record['host_port']}"
        self.start(label+'-host','opencoder-agent',['--name',self.prefix,'--data-dir',s.state_dir / 'host',
            '--workdir',s.agent_workdir,'--remote',s.public_url,'--max-runs',s.max_runs,
            '--token-file',s.token_file,'host','--port',record['host_port'],'--standby'])
        until(lambda:self.http(host,'/status'),'rollback Host')
        record['server_label'] = label + '-server'
        self.start(record['server_label'],'opencoder-server',['--host','127.0.0.1','--port',record['server_port'],
            '--workdir',s.server_workdir,'--data-dir',s.server_data,'--release-config',Path(record['runtime_data']) / 'release.json','--token-file',s.token_file])
        self.http(host,f"/servers/{record['id']}",'POST',{'url':f"http://127.0.0.1:{record['server_port']}",'enabled':True})
        node = self.http(host,'/status')['node']['id']
        probes.ready(s,record,node,self,90)
        return record

    def crash_server(self,record):
        label = record.get('server_label',record['id']+'-server')
        child = self.children[label]
        child.kill()
        child.wait(timeout=30)
        self.start(label,'opencoder-server',child.args[1:])
        until(lambda:self.api('/api/ready')['ready_nodes'] == 1,'Server crash recovery')

    def owner(self,identifier):
        import sqlite3
        with sqlite3.connect(f'file:{self.settings.state_dir}/host/host.db?mode=ro',uri=True) as conn:
            row = conn.execute('SELECT runtime_id FROM runtime_owners WHERE execution_id=?',(identifier,)).fetchone()
            return row[0] if row else None

    def release_work(self):
        self.model.release.set()
        (self.root / 'shell.release').touch()
        for record in self.records:
            for context in Path(record['runtime_data']).rglob('context.json'):
                if any(name in str(context) for name in ['dag-hold','dag-r3-hold','dag-container-hold']):
                    (context.parent.parent / 'release').touch()

    def close(self):
        self.release_work()
        # Stop producers before Runtime units, so an outbox retry cannot wake a
        # Runtime between its quiescence check and the end of fixture cleanup.
        producers = {label:child for label,child in self.children.items() if label != 'resources'}
        for child in producers.values():
            if child.poll() is None:
                child.terminate()
        for label,child in producers.items():
            try:
                child.wait(timeout=60)
            except subprocess.TimeoutExpired:
                print('fixture producer still draining:',label,child.pid,flush=True)
                return
        safe = True
        for record in self.records:
            try:
                if not self.runtime_pid(record):
                    continue
                endpoint = f"http://127.0.0.1:{record['runtime_port']}"
                until(lambda:self.http(endpoint,'/inventory')['can_hibernate'],'fixture quiescence',60)
                subprocess.run(['systemctl','stop',record['runtime_unit']],check=True,timeout=30)
            except Exception as error:
                safe = False
                print('fixture runtime cleanup:',record['runtime_unit'],str(error),flush=True)
        if not safe:
            print('fixture resources retained for unfinished runtime cleanup',flush=True)
            return
        self.resources.close()
        for child in self.children.values():
            if child.poll() is None:
                child.terminate()
        if (self.root / 'nginx.pid').exists():
            subprocess.run([self.nginx,'-e',str(self.root / 'nginx.log'),'-p',str(self.root),'-c',str(self.root / 'nginx.conf'),'-s','quit'],check=True)
        self.model.server.shutdown()
        for label, child in self.children.items():
            try:
                child.wait(timeout=30)
            except subprocess.TimeoutExpired:
                print('fixture process still draining:',label,child.pid,flush=True)
