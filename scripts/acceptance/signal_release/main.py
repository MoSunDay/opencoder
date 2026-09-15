#!/usr/bin/env python3
"""Real Server signals, authenticated Host and independent failing systemd jobs.

Uses private Nginx, model, resources and Runtime units. All databases/evidence
are retained. Valid publish/rollback operations are covered by the live suite.
"""
import argparse
import json
from pathlib import Path
import signal
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'smooth_release'))
from environment import Environment, until
from rolling.io import Operations
from rolling.state import write
from signal_release import controller


def exercise(env):
    record = env.warm('signal-fixture')
    env.switch(record)
    state = env.settings.state_dir
    journal = json.loads((state / 'release-state.json').read_text())
    journal.update(previous=None, phase='complete')
    write(state / 'release-state.json', journal)
    config = env.root / 'controller-config.json'
    write(config, {'deployment':{key:str(value) if isinstance(value,Path) else value
        for key,value in env.settings.__dict__.items()}})
    installed = controller.install(env.settings, config, Operations(env.settings.token_file))
    server = env.children[record['id'] + '-server']
    original_pid = server.pid
    assert env.api('/api/admin/release')['signal_protocol'] == 1
    receipts = []
    for action, number, reason in [('deploy',signal.SIGUSR2,'signal-pending.json'),
                                  ('rollback',signal.SIGUSR1,'no compatible previous release')]:
        instance = f"{action}--{record['id']}"
        path = state / 'signal-receipts' / (instance + '.json')
        for attempt in range(2):
            before = json.loads(path.read_text()) if path.exists() else {}
            server.send_signal(number)
            def finished():
                value = json.loads(path.read_text()) if path.exists() else {}
                return value if value.get('attempt') != before.get('attempt') and value.get('phase') == 'failed' else None
            receipt = until(finished, 'independent job failure receipt')
            assert reason in receipt['failure'], receipt
            assert server.pid == original_pid and server.poll() is None, 'signal terminated Server'
            assert env.api('/api/ready')['ready_nodes'] == 1
            assert env.api('/api/ready')['mode'] == 'open'
            unit = f"{installed['unit_prefix']}@{instance}.service"
            until(lambda:subprocess.check_output(['systemctl','show',unit,'-p','ActiveState','--value'],text=True).strip() == 'failed','job exit')
            result = subprocess.check_output(['systemctl','show',unit,'-p','Result','--value'],text=True).strip()
            assert result == 'exit-code', result
            receipts.append(receipt)
        with (env.root / (action + '-job.log')).open('wb') as log:
            subprocess.run(['journalctl','-u',unit,'--no-pager'],stdout=log,check=True)
        subprocess.run(['systemctl','reset-failed',unit],check=True)
    write(env.root / 'signal-result.json', {'result':'PASS','server_pid':original_pid,
        'cases':['real-usr2','real-usr1','independent-systemd-job','failure-keeps-admission-open','repeated-signal-new-receipt'],
        'receipts':receipts})
    print(json.dumps({'result':'PASS','evidence':str(env.root)}),flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir',type=Path,required=True)
    parser.add_argument('--nginx',type=Path,required=True)
    parser.add_argument('--data-parent',type=Path)
    args = parser.parse_args()
    env = Environment(args.bin_dir.resolve(),args.nginx.resolve(),args.data_parent)
    try:
        exercise(env)
    finally:
        env.close()


if __name__ == '__main__':
    main()
