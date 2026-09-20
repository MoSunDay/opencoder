"""Provision a private real wasmtime image; never touch production rootfs."""
import json
from pathlib import Path
import re
import shutil
import subprocess


class Containers:
    def __init__(self, wasmtime):
        self.wasmtime = wasmtime
        self.sources = []
        if wasmtime:
            result = subprocess.run(['ldd',str(wasmtime)],capture_output=True,text=True)
            if result.returncode and 'not a dynamic executable' not in result.stderr + result.stdout:
                raise ValueError('could not inspect wasmtime dependencies: ' + result.stderr)
            self.sources = [Path(p) for p in re.findall(r'(/[^\s()]+)',result.stdout)]

    def prepare(self, data):
        if not self.wasmtime:
            return
        rootfs = data / 'dag/rootfs'
        for source, target in [(self.wasmtime,rootfs / 'usr/bin/wasmtime'),
                *((p,rootfs / str(p).lstrip('/')) for p in self.sources)]:
            target.parent.mkdir(parents=True,exist_ok=True)
            shutil.copy2(source,target)

    def state(self, runtime_data, execution, step='hold'):
        # Static StepCtx.execution_key is step-<name>; retain the exact fixture owner.
        root = Path(runtime_data) / 'dag/bundles' / execution / step / 'runc-state'
        result = subprocess.run(['runc','--root',str(root),'state',execution + '-step-' + step],capture_output=True,text=True)
        if result.returncode and 'does not exist' in result.stderr:
            return {'status':'not_created'}
        result.check_returncode()
        state = json.loads(result.stdout)
        if state['status'] == 'running':
            state['process_start'] = Path(f"/proc/{state['pid']}/stat").read_text().split()[21]
        return state
