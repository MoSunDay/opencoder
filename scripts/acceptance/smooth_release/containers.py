"""Private, mandatory DAG images and one container per run."""
import json
from pathlib import Path
import subprocess
from rolling.units import freeze_rootfs


class Containers:
    def __init__(self, rootfs, binaries):
        self.rootfs = Path(rootfs).resolve()
        self.binaries = Path(binaries).resolve()
        for name in ['dag-runner', 'agent-step-runner']:
            if not (self.rootfs / 'usr/bin' / name).is_file():
                raise ValueError('native DAG rootfs is missing ' + name)

    def prepare(self, data):
        return freeze_rootfs({'runtime_data': str(data)}, self.rootfs, self.binaries)

    def state(self, runtime_data, execution):
        journal = Path(runtime_data) / 'dag' / execution / 'execution.json'
        if not journal.is_file():
            return {'status': 'not_created'}
        record = json.loads(journal.read_text())
        root = Path(record['annotations']['dag_parent']) / execution / 'runc-state'
        result = subprocess.run(['runc', '--root', str(root), 'state', 'dag-run-' + execution], capture_output=True, text=True)
        if result.returncode and 'does not exist' in result.stderr:
            return {'status': 'not_created'}
        result.check_returncode()
        state = json.loads(result.stdout)
        if state['status'] == 'running':
            state['process_start'] = Path(f"/proc/{state['pid']}/stat").read_text().split()[21]
        return state
