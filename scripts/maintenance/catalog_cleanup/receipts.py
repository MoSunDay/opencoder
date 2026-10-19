"""Resolve uncertain deletion outcomes before allowing another attempt."""
from .backup import verify_receipt
from .client import endpoint
from .model import digest


def uncertain(receipt):
    if receipt['status'] == 'pending':
        return True
    # Older writers marked transport failures as skipped after persisting intent.
    return (receipt['status'] == 'skipped' and not receipt.get('reconciled')
            and all(receipt.get(key) for key in ('backup', 'digest', 'etag'))
            and (receipt.get('http_status') is None or receipt['http_status'] >= 500))


def reconcile(receipts, client):
    for receipt in receipts:
        if not uncertain(receipt):
            continue
        receipt['status'] = 'pending'
        try:
            verify_receipt(receipt)
            status, value, _ = client.call('GET', endpoint(receipt['kind'], receipt['name']))
            if status == 404:
                receipt.update(status='deleted', verified_missing=True, recovered=True)
                receipt.pop('error', None)
            elif status == 200:
                same = digest(value) == receipt['digest']
                receipt.update(status='skipped', reconciled=True,
                    error='object still present; recheck selection' if same else 'name has been reused')
            else:
                receipt['error'] = 'delete result remains unknown; readback HTTP ' + str(status)
        except Exception as error:
            receipt['error'] = 'delete result remains unknown: ' + str(error)
