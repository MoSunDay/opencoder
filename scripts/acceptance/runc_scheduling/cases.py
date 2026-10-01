import hashlib
import json
from runtime import wait, write
from samples import submit, finished, clean


def boundaries(runtime, nodes, resource):
    for mode, status in [('blob', 'done'), ('bad-artifact', 'error'), ('flood', 'error')]:
        identifier = 'dag-native-' + mode
        submit(runtime, identifier, resource, nodes['node-a'], mode)
        value = wait(lambda: finished(runtime, identifier), identifier)
        assert value['execution']['status'] == status, value
        root = runtime.run_root('node-a', identifier)
        if mode == 'blob':
            archive = root / 'first/report.bin'
            assert archive.stat().st_size == 16777216
            digest = hashlib.sha256()
            with archive.open('rb') as source:
                for chunk in iter(lambda: source.read(65536), b''):
                    digest.update(chunk)
            declaration = json.loads((root / 'first/artifacts.json').read_text())['files'][0]
            assert digest.hexdigest() == declaration['sha256']
            assert not (root / 'first/not-declared.txt').exists()
        if mode == 'flood':
            assert 0 < (root / 'first/output.txt').stat().st_size <= 65536
        clean(root)
    identifier = 'dag-native-dynamic'
    runtime.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'node_id': nodes['node-b'],
        'input': {'items': [['one two'], ['three'], []], 'definition': {'name': 'native-dynamic', 'steps': [
            {'name': 'process', 'kind': {'type': 'dynamic', 'source': {'type': 'input', 'pointer': '/items'},
                'template': {'type': 'binary', 'resource': resource, 'args': ['dynamic']}}}]}}})
    value = wait(lambda: finished(runtime, identifier), identifier)
    assert value['execution']['status'] == 'done', value
    root = runtime.run_root('node-b', identifier)
    outputs = [(root / f'process/instances/{index}/output.txt').read_text() for index in range(3)]
    assert len({output.splitlines()[0] for output in outputs}) == 1
    assert 'argument=one two\n' in outputs[0]
    for index, output in enumerate(outputs):
        assert f'cwd=/workspace/process/instances/{index}\n' in output
    assert json.loads((root / 'process/output.json').read_text()) == [{'ok': True}] * 3
    clean(root)


def recovery(runtime, nodes, resource):
    identifier = 'dag-native-recovery'
    runtime.api('/api/executions', 'POST', {'id': identifier, 'kind': 'dag', 'node_id': nodes['node-a'],
        'input': {'definition': {'name': 'native-recovery', 'steps': [
            {'name': 'first', 'kind': {'type': 'binary', 'resource': resource, 'args': ['write']}},
            {'name': 'second', 'depends_on': ['first'], 'kind': {'type': 'binary', 'resource': resource, 'args': ['recover']}}]}}})
    def waiting():
        root = runtime.run_root('node-a', identifier)
        return root if root and (root / 'upper/second/waiting').is_file() else None
    root = wait(waiting, 'Recovery step started')
    first = (root / 'first/meta.json').read_bytes()
    programs = [(root / step / 'meta/program').read_bytes() for step in ['first', 'second']]
    runtime.publish('native-acceptance', 'int main(void) { return 99; }', update=True)
    node = runtime.root / 'runtime/node-a'
    previous = json.loads((node / 'opencoder.json').read_text())
    drifted = json.loads(json.dumps(previous))
    drifted['dag'].update(data_dir=str(node / 'wrong-run-directory'), binary_dir='/does-not-exist', rootfs_dir='/does-not-exist')
    write(node / 'opencoder.json', drifted)
    runtime.restart('node-a')
    wait(lambda: runtime.api('/api/executions/' + identifier)['execution']['status'] == 'interrupted', 'Recovery interruption indexed')
    clean(root)
    (root / 'upper/second/release').touch()
    runtime.api('/api/executions/' + identifier + '/commands', 'POST', {'action': 'resume'})
    value = wait(lambda: finished(runtime, identifier), identifier)
    assert value['execution']['status'] == 'done', value
    assert runtime.run_root('node-a', identifier) == root
    assert (root / 'first/meta.json').read_bytes() == first
    assert (root / 'upper/first/attempts').read_text() == '1'
    assert [(root / step / 'meta/program').read_bytes() for step in ['first', 'second']] == programs
    assert not (node / 'wrong-run-directory').exists()
    clean(root)
    write(node / 'opencoder.json', previous)
    write(runtime.root / 'evidence/recovery.json', {'id': identifier, 'root': str(root), 'first_checkpoint_unchanged': True})
