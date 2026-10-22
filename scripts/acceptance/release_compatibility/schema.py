"""Real old/new Servers: rejected prewarm, stopped migration and rollback."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
from types import SimpleNamespace

from processes import NativeSql, Processes, configure, port, request, wait_http
from rolling.native import ontology_schema
from rolling.state import Journal
from rolling.storage.database import using
from rolling.maintenance import archive, flow


def exercise(root, old, candidate):
    root.mkdir()
    work = configure(root)
    data = root / 'server'
    old_port, new_port, resources_port = port(), port(), port()
    old_url, new_url = f'http://127.0.0.1:{old_port}', f'http://127.0.0.1:{new_port}'
    resource_url = f'http://127.0.0.1:{resources_port}'
    manifest = json.loads((candidate / 'manifest.json').read_text())
    original = json.loads((old / 'manifest.json').read_text())
    settings = SimpleNamespace(state_dir=root, server_data=data)
    journal = Journal(root)
    journal.data.update(current='old', candidate='candidate', phase='warming', releases={
        'old': {'id': 'old', 'server_port': old_port, 'manifest': original},
        'candidate': {'id': 'candidate', 'server_port': new_port, 'manifest': manifest}})
    journal.save()
    release = root / 'release.json'
    release.write_text(json.dumps({'release_id': 'candidate', 'state_dir': str(root),
        'host_service': resource_url, 'resource_service': resource_url}))
    processes, sql = Processes(root), None
    evidence = {'passed': False}
    try:
        old_server = processes.server('old', old / 'bin/opencoder-server', work, data, old_port)
        before = wait_http(old_server, old_url)
        sql = NativeSql(old / 'bin/opencoder-cli', data / 'ontology.db', root)
        assert not sql.query("SELECT name FROM sqlite_schema WHERE name='catalog_entries'"), 'old fixture must predate catalog schema'
        sql.query('BEGIN')
        sql.query('SELECT count(*) FROM entities')
        new_server = processes.server('rejected-candidate', candidate / 'bin/opencoder-server', work, data, new_port, release)
        assert new_server.wait(timeout=90) != 0
        assert '--maintenance' in (root / 'rejected-candidate.log').read_text()
        sql.query('COMMIT')
        for _ in range(5):
            sql.query('BEGIN')
            sql.query('SELECT count(*) FROM entities')
            sql.query('COMMIT')
            assert request(old_url, '/api/ontology/environments') == before
        assert not sql.query("SELECT name FROM sqlite_schema WHERE name='catalog_entries'")
        evidence['old_reads_after_rejection'] = 5
        sql.close()
        sql = None
        processes.stop(old_server)
        backup = root / 'backup'
        backup.mkdir()
        subprocess.run([str(old / 'bin/opencoder-cli'), 'storage', 'backup', '--database', str(data / 'ontology.db'),
                        '--output', str(backup / 'ontology.db')], check=True, capture_output=True)
        shutil.copytree(root / 'files', backup / 'files')
        metadata = {'files': archive.inventory(backup)}
        (backup / 'manifest.json').write_text(json.dumps(metadata))
        digest = hashlib.sha256((backup / 'manifest.json').read_bytes()).hexdigest()
        (backup / 'manifest.sha256').write_text(digest)
        # Lose a process in the middle of DDL: no partial structure may remain.
        sql = NativeSql(candidate / 'bin/opencoder-cli', data / 'ontology.db', root)
        sql.query('BEGIN IMMEDIATE')
        sql.query(ontology_schema.contract()['catalog_entries'])
        sql.child.kill()
        sql.child.wait(timeout=10)
        sql.close()
        sql = None
        with using(candidate / 'bin/opencoder-cli'):
            assert 'catalog_entries' in ontology_schema.inspect(data / 'ontology.db')['missing']
        evidence['ddl_process_crash_rolled_back'] = True
        resource = processes.server('resources', candidate / 'bin/opencoder-server', work,
                                     root / 'resources', resources_port, resources=True)
        wait_http(resource, resource_url, '/api/health')
        record = journal.data['releases']['candidate']
        for attempt in range(2):
            journal.data.update(current='old', candidate='candidate', phase='migrating')
            journal.data['maintenance'] = {'stage': 'installing', 'target': 'candidate', 'origin': 'old',
                'schema_started': True, 'writes_open': False, 'backup': str(backup)}
            with using(candidate / 'bin/opencoder-cli'):
                ontology_schema.authorize(settings, journal, record)
            new_server = processes.server(f'migration-{attempt}', candidate / 'bin/opencoder-server', work, data, new_port, release)
            assert wait_http(new_server, new_url) == before
            if attempt == 0:
                # DDL committed, but its release completion receipt was lost.
                new_server.kill()
                new_server.wait(timeout=10)
                assert 'ontology_schema_complete' not in journal.data['maintenance']
                new_server = processes.server('migration-resumed', candidate / 'bin/opencoder-server', work, data, new_port, release)
                assert wait_http(new_server, new_url) == before
                evidence['committed_migration_resumed'] = True
            with using(candidate / 'bin/opencoder-cli'):
                ontology_schema.completed(settings, journal, record)
            assert archive.verify(backup) == metadata
            if attempt == 0:
                processes.stop(new_server)
                # Only this isolated Ontology database is restored; auth and
                # execution stores are deliberately outside the fixture backup.
                for suffix in ['-wal', '-shm', '-tshm']:
                    Path(str(data / 'ontology.db') + suffix).unlink(missing_ok=True)
                shutil.copy2(backup / 'ontology.db', data / 'ontology.db')
                old_server = processes.server('rolled-back-old', old / 'bin/opencoder-server', work, data, old_port)
                assert wait_http(old_server, old_url) == before
                request(old_url, '/api/ontology/environments', {'key': 'retained', 'name': 'Retained'})
                before = request(old_url, '/api/ontology/environments')
                processes.stop(old_server)
        journal.data.update(current='candidate', candidate=None, phase='complete', previous='old')
        journal.data['maintenance'].update(writes_open=True, stage='complete')
        journal.save()
        request(new_url, '/api/ontology/environments', {'key': 'after', 'name': 'After migration'})
        try:
            flow.rollback(settings, object())
        except ValueError as error:
            assert 'old backup restoration is forbidden' in str(error), str(error)
        else:
            raise AssertionError('rollback after writes reopened was accepted')
        after = request(new_url, '/api/ontology/environments')
        assert len(after['items']) == len(before['items']) + 1
        processes.stop(new_server)
        journal.data['maintenance']['ontology_migration']['backup_verified'] = False
        journal.save()
        restarted = processes.server('ordinary-restart', candidate / 'bin/opencoder-server', work, data, new_port, release)
        assert wait_http(restarted, new_url) == after
        evidence.update(passed=True, migration_attempts=2, rollback_before_writes=True,
                        rollback_after_writes_rejected=True, regular_restart=True, backup_sha256=digest)
    finally:
        if sql:
            sql.close()
        processes.close()
        (root / 'receipt.json').write_text(json.dumps(evidence, indent=2))
    return evidence
