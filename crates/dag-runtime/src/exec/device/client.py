"""Scoped device tool transport. Credentials are read privately, never printed."""
import json
from pathlib import Path
import sys
import urllib.error
import urllib.request


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def reserve(transport):
    identity, inputs = transport['identity'], transport['input']
    body = {'dag_id': identity['dag_id'], 'target_step': 'execute', 'count': inputs['device_count']}
    request = urllib.request.Request(transport['endpoint'].rstrip('/')+'/v1/reservations',
        data=json.dumps(body).encode(), headers={'Authorization':'Bearer '+transport['capability'],
                                               'Content-Type':'application/json'})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=35) as response:
        result = json.load(response)
    assignments = result['assignments']
    if len(assignments) != inputs['device_count']:
        raise ValueError('Allocation count differs from frozen device count')
    ordered = {a['instance_id']: a for a in assignments}
    if set(ordered) != {str(i) for i in range(inputs['device_count'])}:
        raise ValueError('Unexpected or duplicate instance assignments')
    items = []
    for i in range(inputs['device_count']):
        row = ordered[str(i)]
        assignment = {k:row[k] for k in ('instance_id','machine','generation')}
        assignment['reservation_id'] = result['reservation_id']
        assignment['case_ids'] = inputs['case_ids'][i::inputs['device_count']]
        items.append(json.dumps(assignment, separators=(',',':')))
    return {'items':items}


def main():
    if sys.argv[1:] != ['reserve']:
        raise ValueError('Expected reserve action')
    transport = json.loads(Path(__file__).with_name('transport.json').read_text())
    if transport['identity']['step_id'] != 'allocate':
        raise ValueError('Allocator transport required')
    print(json.dumps(reserve(transport)))


if __name__ == '__main__':
    try:
        main()
    except urllib.error.HTTPError as error:
        print('Device API rejected allocation: HTTP '+str(error.code), file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        # Transport/capability values never enter exception output.
        print('Device allocation failed: '+type(error).__name__, file=sys.stderr)
        sys.exit(1)
