"""Compare indexed replay bytes against the isolated fixture's read-only store."""
import base64
import hashlib
import json
from pathlib import Path
import sqlite3
import sys
import urllib.parse
import urllib.request


def digest(value):
    return hashlib.sha256(value).hexdigest()


def audit(config):
    root = Path(config['root']).resolve()
    assert root.name.startswith('opencoder-project-acceptance-')
    database = root / 'node-a/state/runtime.db'
    store = sqlite3.connect(database.as_uri() + '?mode=ro', uri=True)
    store.row_factory = sqlite3.Row
    assert urllib.parse.urlsplit(config['base']).hostname in {'127.0.0.1', '::1'}
    # Fixture HTTP stays on loopback even when the developer shell has a proxy.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def get(route):
        request = urllib.request.Request(config['base'] + route, headers={
            'Authorization': 'Bearer ' + config['token'],
        })
        with opener.open(request, timeout=10) as response:
            return json.load(response)

    def field(run_id, name):
        parts, offset = [], 0
        while True:
            query = urllib.parse.urlencode({'field': name, 'offset': offset})
            chunk = get(f'/api/executions/{run_id}/detail-field?{query}')
            raw = base64.b64decode(chunk['bytes_b64'])
            assert len(raw) <= 65536
            assert chunk['next_offset'] == offset + len(raw)
            parts.append(raw)
            offset = chunk['next_offset']
            if chunk['eof']:
                return b''.join(parts)

    records = []
    for run_id in config['ids']:
        row = store.execute('SELECT * FROM project_todo_runs WHERE id=?', (run_id,)).fetchone()
        assert row is not None
        trace = json.loads(row['trace_manifest'])
        fields = {}
        for name in ['input_snapshot', 'plan_md', 'output_md', 'trace_manifest']:
            if row[name] is not None:
                expected = row[name].encode()
                actual = field(run_id, f'project.run.{run_id}.{name}')
                assert actual == expected, (run_id, name)
                fields[name] = digest(actual)
        expected = {
            message['seq']: message
            for message in store.execute(
                'SELECT seq,role,blocks_json FROM messages WHERE session_id=? AND seq>? AND seq<=? ORDER BY seq',
                (row['session_id'], trace['messages_after'], trace['messages_through']),
            )
        }
        actual, cursor = {}, {'seq': 0, 'offset': 0}
        while True:
            query = urllib.parse.urlencode(cursor)
            page = get(f'/api/executions/{run_id}/messages?{query}')
            for chunk in page['chunks']:
                seq = chunk['seq']
                assert seq in expected
                assert chunk['role'] == expected[seq]['role']
                raw = base64.b64decode(chunk['bytes_b64'])
                collected = actual.setdefault(seq, bytearray())
                assert chunk['offset'] == len(collected)
                collected.extend(raw)
                assert chunk['next_offset'] == len(collected)
            if not page['more']:
                break
            assert page['next_cursor'] != cursor
            cursor = page['next_cursor']
        assert actual.keys() == expected.keys(), run_id
        for seq, raw in actual.items():
            assert raw == expected[seq]['blocks_json'].encode(), (run_id, seq)
        records.append({'id': run_id, 'status': row['status'], 'fields': fields,
                        'messages': {str(seq): digest(raw) for seq, raw in actual.items()}})
    store.close()
    report = {'attempts': len(records), 'messages': sum(len(row['messages']) for row in records),
              'records': records}
    (root / 'storage-audit.json').write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    audit(json.load(sys.stdin))
