"""Execute the retained old release's WASM probe using only its old Runtime."""
import hashlib
from pathlib import Path
from rolling.state import atomic_bytes


def execute(record, operations):
    module = bytes.fromhex('0061736d0100000001040160000003020100070a01065f737461727400000a040102000b')
    atomic_bytes(Path(record['runtime_data']) / 'dag/_modules/release-probe.wasm', module, 0o444)
    base = f"http://127.0.0.1:{record['runtime_port']}"
    inventory = operations.http(base, '/inventory')
    if inventory['build']['git_commit'] != record['manifest']['commit']:
        raise AssertionError('legacy activation must use the actual old Runtime')
    identifier = 'dag-probe-' + hashlib.sha256(record['id'].encode()).hexdigest()[:32]
    assignment = {'index': {'id': identifier, 'kind': 'dag', 'node_id': inventory['registration']['id'],
                           'created_at': record['created_at'], 'status': 'pending'},
                  'request': {'id': identifier, 'kind': 'dag', 'input': {}, 'node_id': inventory['registration']['id']},
                  'definition': {'name': 'release-probe', 'steps': [{'name': 'execute', 'kind': {
                      'type': 'wasm', 'command': 'release-probe.wasm'}}]}}
    reply = operations.http(base, '/rpc', 'POST', {'operation': 'create', 'assignment': assignment})
    if reply['status'] >= 300:
        raise AssertionError('old Runtime rejected its own activation probe: ' + str(reply))
    def completed():
        view = operations.http(base, '/inventory')
        index = next((i for i in view['indexes'] if i['id'] == identifier), None)
        if index and index['status'] in ('error', 'cancelled', 'interrupted'):
            raise AssertionError('old execution probe failed: ' + str(index))
        return index and index['status'] == 'done' and view['snapshot']['ready']
    operations.wait(completed, 120)
    return {'id': identifier, 'status': 'done', 'actual_old_runtime': inventory['build']['git_commit']}
