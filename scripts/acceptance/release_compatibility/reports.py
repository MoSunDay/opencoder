"""A real writer ACK must survive restarting its Runtime with the reader bridge."""
import hashlib
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'runtime_memory'))
from probe import fixture
from processes import NativeSql, Processes, port, request, wait_http


def exercise(root, bridge, candidate):
    root.mkdir()
    _, _, data, work, home = fixture(root, 0)
    identifier = 'agent-compatibility-leaf'
    record = {
        'assignment': {
            'index': {'id': identifier, 'kind': 'agent', 'node_id': 'node-memory-fixture',
                      'status': 'done', 'created_at': 1},
            'request': {'id': identifier, 'kind': 'agent', 'target': None, 'node_id': None, 'input': {'schema_version': 7,
                'brain_layered': {'parent': {'run_id': 'brain-parent', 'operation_id': 'operation-a',
                                           'node_id': 'step-a', 'layer': 1}}}},
            'runtime': {'profiles': {}, 'frozen': 'x' * 8192}, 'definition': {'frozen': 'y' * 8192}},
        'annotations': {}, 'result': {'retained': True}, 'error': None,
        'events': [{'seq': 3, 'ts': 1, 'kind': 'done', 'data': {'retained': True}}],
    }
    path = data / 'agent' / identifier / 'execution.json'
    path.parent.mkdir(parents=True)
    path.write_text(json.dumps(record))
    original = path.read_bytes()
    listen = port()
    endpoint = f'http://127.0.0.1:{listen}'
    processes, sql = Processes(root), None
    evidence = {'passed': False}
    reference = {'id': identifier, 'kind': 'agent'}
    try:
        writer = processes.runtime('writer', candidate / 'bin/opencoder-agent', data, work, home, listen)
        inventory = wait_http(writer, endpoint, '/inventory')
        writer_build = inventory['build']
        assert inventory['build']['report_state'] == {'reader': 1, 'writer': 1}
        frames = request(endpoint, '/frames')
        assert len(frames) == 1 and frames[0]['action'] == 'layered_terminal'
        assert frames[0]['input']['source_sequence'] == 3
        ack = request(endpoint, '/rpc', {'operation': 'brain', 'execution': reference,
                                       'action': 'notice_ack', 'input': {'sequence': 3}})
        assert ack['status'] == 200, ack
        assert request(endpoint, '/frames') == []
        assert path.read_bytes() == original, 'writer must not rewrite the large journal'
        processes.stop(writer)
        sql = NativeSql(candidate / 'bin/opencoder-cli', data / 'runtime.db', root)
        rows = sql.query('SELECT execution_id,action,cursor FROM execution_report_acks')
        assert [identifier, 'layered_terminal', 3] in rows, rows
        sql.close()
        sql = None
        reader = processes.runtime('reader', bridge / 'bin/opencoder-agent', data, work, home, listen)
        inventory = wait_http(reader, endpoint, '/inventory')
        assert inventory['build']['report_state'] == {'reader': 1, 'writer': 0}
        for _ in range(5):
            assert request(endpoint, '/frames') == [], 'acknowledged terminal was replayed'
        detail = request(endpoint, '/rpc', {'operation': 'inspect', 'execution': reference})
        assert detail['status'] == 200 and detail['body']['result'] == record['result'], detail
        restored = json.loads(path.read_text())
        for key in ['index', 'request', 'runtime', 'definition']:
            assert restored['assignment'][key] == record['assignment'][key]
        for key in ['result', 'error', 'events']:
            assert restored[key] == record[key]
        assert restored['annotations']['brain_layered_ack'] == 3
        processes.stop(reader)
        preserved = path.read_bytes()
        restarted = processes.runtime('reader-restart', bridge / 'bin/opencoder-agent', data, work, home, listen)
        wait_http(restarted, endpoint, '/inventory')
        assert request(endpoint, '/frames') == []
        assert path.read_bytes() == preserved
        evidence.update(passed=True, writer_build=writer_build, reader_build=inventory['build'], ack_rows=rows,
                        writer_journal_sha256=hashlib.sha256(original).hexdigest(),
                        bridge_journal_sha256=hashlib.sha256(preserved).hexdigest(), repeated_empty_reports=6)
    finally:
        if sql:
            sql.close()
        processes.close()
        (root / 'receipt.json').write_text(json.dumps(evidence, indent=2))
    return evidence
