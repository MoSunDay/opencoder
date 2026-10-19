"""Back up exact objects, then use guarded conditional DELETE requests."""
import hashlib
import json
from pathlib import Path
import shutil
from .client import endpoint, write, require, sync_tree
from .inventory import inventory
from .model import digest


def verify_pool(path, snapshot):
    require(not path.is_symlink() and not any(p.is_symlink() for p in path.rglob('*')), 'pool symlink')
    meta = json.loads((path / 'meta.json').read_text())
    require({**meta, 'history': snapshot['history']} == snapshot, 'pool metadata changed')
    require(meta['history'] == [v['version'] for v in snapshot['history']], 'version list changed')
    expected = {'meta.json'}
    for version in snapshot['history']:
        require(type(version['version']) is int and version['version'] > 0, 'invalid version')
        directory = path / ('v' + str(version['version']))
        require(json.loads((directory / 'meta.json').read_text()) == version, 'version metadata changed')
        data = (directory / 'binary.bin').read_bytes()
        require(len(data) == version['size_bytes'] and hashlib.sha256(data).hexdigest() == version['sha256'], 'binary checksum mismatch')
        expected.add(directory.name)
        require({p.name for p in directory.iterdir()} == {'meta.json', 'binary.bin'}, 'unlisted version data')
    require({p.name for p in path.iterdir()} == expected, 'unlisted pool data')


def apply(plan, directory, client):
    directory = Path(directory)
    require(directory.is_dir(), 'evidence directory must exist')
    status, health, _ = client.call('GET', '/api/health')
    require(status == 200 and health.get('catalog_guard') == 1, 'guarded server is required before cleanup')
    fresh = inventory(plan['data_dir'], plan['binary_root'], plan['agents_root'], plan['config_path'], plan['retention_days'])
    allowed = {(kind, item['name']): item for kind in ('dag', 'binary') for item in fresh[kind]}
    receipts_path = directory / 'receipts.json'
    receipts = json.loads(receipts_path.read_text()) if receipts_path.exists() else []
    # A process may exit after DELETE succeeds but before its acknowledgement
    # is saved. Recover only a previously persisted intent with a valid backup.
    for receipt in receipts:
        if receipt['status'] != 'pending':
            continue
        backup = Path(receipt['backup'])
        snapshot = json.loads((backup / 'snapshot.json').read_text())
        require(digest(snapshot) == receipt['digest'], 'pending backup changed')
        if receipt['kind'] == 'binary':
            verify_pool(backup / 'pool', snapshot)
        status, _, _ = client.call('GET', endpoint(receipt['kind'], receipt['name']))
        if status == 404:
            receipt.update(status='deleted', verified_missing=True, recovered=True)
        else:
            receipt.update(status='skipped', error='previous delete acknowledgement missing; rechecking')
    write(receipts_path, receipts)
    finished = {(r['kind'], r['name']) for r in receipts if r['status'] == 'deleted'}
    for kind in ('dag', 'binary'):
        for item in plan[kind]:
            name = item['name']
            if (kind, name) in finished:
                continue
            receipt = {'kind': kind, 'name': name, 'reason': item['reason'], 'keep': item.get('keep')}
            try:
                current = allowed.get((kind, name))
                if not current or current['digest'] != item['digest']:
                    raise ValueError('selection or content changed since preview')
                path = endpoint(kind, name)
                status, value, etag = client.call('GET', path)
                require(status == 200 and etag and digest(value) == item['digest'], 'live object differs from preview')
                backup = directory / (kind + '-' + hashlib.sha256(name.encode()).hexdigest()[:24])
                if backup.exists():
                    require(json.loads((backup / 'snapshot.json').read_text()) == value, 'backup identity conflict')
                else:
                    backup.mkdir(mode=0o700)
                    write(backup / 'snapshot.json', value)
                    if kind == 'binary':
                        source = Path(plan['binary_root']) / name
                        verify_pool(source, value)
                        shutil.copytree(source, backup / 'pool', symlinks=True)
                if kind == 'binary':
                    verify_pool(backup / 'pool', value)
                sync_tree(backup)
                receipt['backup'] = str(backup)
                receipt.update(status='pending', digest=item['digest'], etag=etag)
                receipts.append(receipt)
                write(receipts_path, receipts)
                status, body, _ = client.call('DELETE', path, etag=etag)
                receipt.update(http_status=status, response=body)
                if status != 200:
                    raise ValueError('conditional delete refused')
                receipt['status'] = 'deleted'
                write(receipts_path, receipts)
                status, _, _ = client.call('GET', path)
                require(status == 404, 'deleted object reappeared')
                receipt['verified_missing'] = True
            except Exception as error:
                if receipt.get('status') != 'deleted':
                    receipt.update(status='skipped', error=str(error))
                    if receipt not in receipts:
                        receipts.append(receipt)
                else:
                    receipt['verification_error'] = str(error)
            write(receipts_path, receipts)
    return receipts
