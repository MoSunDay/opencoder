#!/usr/bin/env python3
"""Public HTTP/CLI/browser and kernel NFS acceptance in a private mount namespace."""
import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling.config import load
from rolling.io import Operations
from rolling.state import Journal, write
from rolling.todo_resources import installation


def sha(content):
    return hashlib.sha256(content).hexdigest()


def exercise(args):
    settings = load(args.config)
    operations = Operations(settings.token_file)
    manifest = json.loads((args.bundle/'manifest.json').read_text())
    state = Journal(settings.state_dir).data
    assert state['current'] == manifest['release_id'] and state['phase'] == 'complete'
    installation.probe(settings, operations)
    output = args.output
    output.mkdir(parents=True, exist_ok=False)
    mount = output/'mount'; mount.mkdir()
    tag = secrets.token_hex(6)
    todo_ids = []
    fixtures = []

    def api(method, route, body=None, status=200, binary=False):
        request = urllib.request.Request(settings.public_url + route, method=method,
            data=body if binary else (json.dumps(body).encode() if body is not None else None),
            headers={'Authorization': 'Bearer ' + operations.token,
                     'Content-Type': 'application/octet-stream' if binary else 'application/json'})
        try:
            response = operations.opener.open(request, timeout=60)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            content = response.read()
            assert response.status == status, (method, route, response.status, content[:200])
        return content if binary else json.loads(content)

    def route(identifier, name):
        return f'/api/project/todos/{identifier}/file?path=' + urllib.parse.quote(name, safe='')

    def identity():
        pid = int(operations.output('systemctl', 'show', 'opencoder-resources.service', '-p', 'MainPID', '--value'))
        return [pid, Path(f'/proc/{pid}/stat').read_text().split()[21]]

    original = identity()
    configuration = settings.server_workdir/'opencoder.json'
    config_sha = sha(configuration.read_bytes())
    mounted = False
    try:
        for index in range(2):
            todo = api('POST', '/api/project/todos', {'title': f'TODO files acceptance {tag}-{index}',
                'draft': '## 关键信息\n\n文件验收及交接。'})
            todo_ids.append(todo['id'])
        samples = [(todo_ids[0], 'empty.bin', b''),
                   (todo_ids[0], 'same.bin', b'\x7b' * (3 * 1024 * 1024)),
                   (todo_ids[0], 'large.bin', b'\xa5' * (64 * 1024 * 1024)),
                   (todo_ids[1], 'same.bin', b'\x42' * (3 * 1024 * 1024))]
        for identifier, name, content in samples:
            api('PUT', route(identifier, name), content, 201, True)
        write(output/'created.json', {'todos': todo_ids, 'release_id': manifest['release_id']})
        subprocess.run(['node', str(Path(__file__).with_name('browser.js')), settings.public_url,
            str(settings.token_file), todo_ids[0], str(output)], check=True, timeout=180)
        samples.append((todo_ids[0], 'handoff/说明.md', '# 交接\n\n当前进展与下一步。\n'.encode()))
        before = api('GET', f'/api/project/todos/{todo_ids[0]}/files')
        api('PUT', route(todo_ids[0], 'same.bin'), b'conflict', 409, True)
        api('DELETE', f'/api/project/todos/{todo_ids[0]}', status=409)
        export = operations.http(settings.todo_resource_url, '/api/project/todos/nfs')['status']
        subprocess.run(['mount', '-t', 'nfs', '-o',
            f'ro,vers=3,tcp,port={export["port"]},mountport={export["port"]},nolock,actimeo=0,soft,timeo=50,retrans=2',
            args.nfs_host + ':/', str(mount)], check=True, timeout=30)
        mounted = True
        cli = [str(args.bundle/'bin/opencoder-cli'), '--server', settings.public_url,
               '--token-file', str(settings.token_file)]
        for index, (identifier, name, content) in enumerate(samples):
            listed = api('GET', f'/api/project/todos/{identifier}/files')
            relative = listed['directory'] + '/' + name
            assert sha(api('GET', route(identifier, name), binary=True)) == sha(content)
            assert sha((mount/relative).read_bytes()) == sha(content)
            target = output/f'download-{index}.bin'
            subprocess.run([*cli, 'project', 'todos', 'files', 'download', identifier, name, '--output', str(target)],
                check=True, stdout=subprocess.DEVNULL, timeout=60)
            assert sha(target.read_bytes()) == sha(content)
            fixtures.append({'id': identifier, 'path': name, 'relative': relative, 'bytes': len(content), 'sha256': sha(content)})
        try:
            (mount/'write-must-fail').write_bytes(b'x')
            raise AssertionError('NFS accepted a write')
        except OSError as error:
            assert error.errno in (errno.EROFS, errno.EACCES, errno.EPERM), error
        write(output/'behavior.json', {'result': 'PASS', 'files': fixtures, 'todos': todo_ids,
            'artifact': manifest, 'nfs_host': args.nfs_host, 'existing_resource_identity': original,
            'public_list': before})
        dist = Path(__file__).resolve().parents[3]/'crates/web/spa/dist'
        assets = sorted(p for p in dist.rglob('*') if p.is_file())
        digest = hashlib.sha256()
        for asset in assets:
            content = asset.read_bytes(); digest.update(content)
            route_path = '/' if asset.name == 'index.html' else '/' + str(asset.relative_to(dist))
            assert sha(api('GET', route_path, binary=True)) == sha(content), route_path
        assert digest.hexdigest() == manifest['spa_sha256']
        # T0 follows the last behavioral input and all final activation/config changes.
        began = time.monotonic(); samples = []
        for index in range(args.observe_seconds // 30 + 1):
            if index:
                time.sleep(max(0, began + index * 30 - time.monotonic()))
            started = time.monotonic()
            state = Journal(settings.state_dir).data
            assert state['current'] == manifest['release_id'] and state['phase'] == 'complete'
            assert sha(configuration.read_bytes()) == config_sha
            installation.probe(settings, operations)
            assert api('GET', '/api/health')['commit'] == manifest['version_long']
            assert identity() == original
            for identifier in todo_ids:
                listed = api('GET', f'/api/project/todos/{identifier}/files')
                expected = {f['path'] for f in fixtures if f['id'] == identifier}
                assert {f['path'] for f in listed['files']} == expected
                context = subprocess.run([*cli, 'project', 'todos', 'context', identifier], check=True,
                    capture_output=True, text=True, timeout=30)
                value = json.loads(context.stdout)
                assert value['complete'] is True
                assert {f['path'] for f in value['record']['files']['files']} == expected
            for fixture in fixtures:
                assert sha((mount/fixture['relative']).read_bytes()) == fixture['sha256']
            assert time.monotonic() - started <= 30, 'observation exceeded the 30-second freshness bound'
            samples.append({'index': index, 'elapsed': time.monotonic() - began,
                'check_seconds': time.monotonic() - started, 'result': 'PASS'})
            write(output/'observation.json', {'t0_unix': time.time() - (time.monotonic() - began), 'samples': samples})
        for fixture in fixtures:
            api('DELETE', route(fixture['id'], fixture['path']))
        for identifier in todo_ids:
            api('DELETE', f'/api/project/todos/{identifier}')
        write(output/'result.json', {'result': 'PASS', 'release_id': manifest['release_id'],
            'file_samples': 5, 'todo_samples': 2, 'observation_samples': len(samples),
            'observation_seconds': time.monotonic() - began, 'canaries_removed': True})
    except BaseException:
        import traceback
        write(output/'failure.json', {'todos': todo_ids, 'error': traceback.format_exc()})
        raise
    finally:
        if mounted:
            subprocess.run(['umount', str(mount)], check=True, timeout=30)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--nfs-host', required=True)
    parser.add_argument('--observe-seconds', type=int, default=900)
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.observe_seconds < 900:
        parser.error('observation requires at least 900 seconds')
    if not args.inside:
        subprocess.run(['unshare', '--mount', '--propagation', 'private', sys.executable,
            str(Path(__file__).resolve()), *sys.argv[1:], '--inside'], check=True)
    else:
        exercise(args)


if __name__ == '__main__':
    main()
