"""Back up exact objects, then use guarded conditional DELETE requests."""
import json
from pathlib import Path
import time
from .backup import create_backup, verify_pool
from .client import endpoint, write, require
from .inventory import inventory
from .model import digest
from .receipts import reconcile


def apply(plan, directory, client):
    directory = Path(directory)
    require(directory.is_dir(), 'evidence directory must exist')
    status, health, _ = client.call('GET', '/api/health')
    require(status == 200 and health.get('catalog_guard') == 2, 'catalog guard version 2 is required before cleanup')
    receipts_path = directory / 'receipts.json'
    receipts = json.loads(receipts_path.read_text()) if receipts_path.exists() else []
    # Readback recovery is independent of a new inventory. An unrelated unreadable
    # resource must not prevent reconciling an already-issued deletion.
    reconcile(receipts, client)
    write(receipts_path, receipts)
    fresh = inventory(plan['data_dir'], plan['binary_root'], plan['agents_root'], plan['config_path'], plan['retention_days'])
    allowed = {(kind, item['name']): item for kind in ('dag', 'binary') for item in fresh[kind]}
    settled_or_pending = {(r['kind'], r['name']) for r in receipts if r['status'] in ('deleted', 'pending')}
    for kind in ('dag', 'binary'):
        for item in plan[kind]:
            name = item['name']
            if (kind, name) in settled_or_pending:
                continue
            receipt = {'kind': kind, 'name': name, 'reason': item['reason'], 'keep': item.get('keep')}
            try:
                current = allowed.get((kind, name))
                require(current and current['digest'] == item['digest'], 'selection or content changed since preview')
                path = endpoint(kind, name)
                status, value, etag = client.call('GET', path)
                require(status == 200 and etag and digest(value) == item['digest'], 'live object differs from preview')
                backup = create_backup(directory, kind, name, value, plan['binary_root'])
                receipt.update(status='pending', backup=str(backup), digest=item['digest'], etag=etag)
                receipts.append(receipt)
                write(receipts_path, receipts)
                try:
                    status, body, _ = client.call('DELETE', path, etag=etag)
                    receipt.update(http_status=status, response=body)
                finally:
                    # Give queued admission/reference readers a chance to acquire
                    # the shared lock before the next exclusive catalog mutation.
                    time.sleep(0.2)
                reconcile([receipt], client)
            except Exception as error:
                if receipt.get('status') == 'pending':
                    # A timeout or disconnect is not evidence that DELETE failed.
                    # Persist the original intent; a later readback resolves it.
                    receipt['error'] = str(error)
                else:
                    receipt.update(status='skipped', error=str(error))
                    if receipt not in receipts:
                        receipts.append(receipt)
            write(receipts_path, receipts)
    return receipts
