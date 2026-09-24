"""Scoped device tool transport. Credentials are read privately, never printed."""
import hashlib
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
    ui = transport.get('work_type') == 'ui'
    if ui:
        body.update(work_type='ui', case_ids=[case['case_id'] for case in inputs['cases']],
                    case_specs=inputs['cases'])
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
        if ui:
            assignment['cases'] = inputs['cases'][i::inputs['device_count']]
        else:
            assignment['case_ids'] = inputs['case_ids'][i::inputs['device_count']]
        items.append(json.dumps(assignment, separators=(',',':')))
    return {'items':items}


def post(transport, route, body, timeout=130):
    request = urllib.request.Request(transport['endpoint'].rstrip('/')+route,
        data=json.dumps(body, separators=(',', ':')).encode(),
        headers={'Authorization':'Bearer '+transport['capability'],
                 'Content-Type':'application/json'})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=timeout) as response:
        return json.load(response)


def ui_identity(transport, case_id):
    if transport.get('work_type') != 'ui' or transport['identity']['step_id'] != 'execute':
        raise ValueError('UI execute transport required')
    cases = [case for case in transport['assignment']['cases'] if case['case_id'] == case_id]
    if len(cases) != 1:
        raise ValueError('Case is outside host assignment')
    assignment, identity = transport['assignment'], transport['identity']
    key = json.dumps([assignment['reservation_id'],assignment['instance_id'],case_id],
                     separators=(',', ':')).encode()
    run_id = 'ui-' + hashlib.sha256(key).hexdigest()[:40]
    common = {k:assignment[k] for k in ('instance_id','generation')}
    common['session_id'] = identity['session_id']
    return cases[0], run_id, common


def ui_register(transport, case_id):
    case, run_id, common = ui_identity(transport, case_id)
    body = {**common,'ui_run_id':run_id,'execution_id':transport['identity']['dag_id'],
            'case_id':case_id,'spec_sha256':case['spec_sha256'],
            'input_version':case['input_version']}
    result = post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-work',body)
    if result.get('ui_run_id') != run_id:
        raise ValueError('UI work identity changed')
    return {k:result.get(k) for k in ('ui_run_id','case_id','recovery_only','recovery_verified')}


def ui_action(transport, case_id, operation_id, action, args_path):
    _, run_id, common = ui_identity(transport, case_id)
    arguments = json.loads(Path(args_path).read_text())
    if not isinstance(arguments, dict):
        raise ValueError('Action arguments must be an object')
    body = {**common,'ui_run_id':run_id,'case_id':case_id,
            'operation_id':operation_id,'action':action,'arguments':arguments}
    return post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-actions',body)


def ui_complete(transport, case_id, receipts_path):
    _, run_id, common = ui_identity(transport, case_id)
    receipts = json.loads(Path(receipts_path).read_text())
    if not isinstance(receipts, dict):
        raise ValueError('UI receipts must be an object')
    body = {**common, 'ui_run_id':run_id, 'case_id':case_id, 'receipts':receipts}
    result = post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-completions',body)
    if result.get('ui_run_id') != run_id or result.get('case_id') != case_id or result.get('recovery_verified') is not True:
        raise ValueError('UI completion authority mismatch')
    return {k:result.get(k) for k in ('ui_run_id','case_id','recovery_verified','evidence_complete','case_status')}


def ui_screenshot(transport, case_id, operation_id):
    _, run_id, common = ui_identity(transport, case_id)
    body = {**common,'ui_run_id':run_id,'case_id':case_id,
            'operation_id':operation_id,'action':'screenshot','arguments':{}}
    result = post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-screenshots',body)
    evidence = result.get('result') if isinstance(result,dict) else None
    if not isinstance(result,dict) or result.get('status') != 'done' or not isinstance(evidence,dict) or not evidence.get('file', '').startswith('screenshots/'):
        raise ValueError('Scoped screenshot evidence is incomplete')
    return evidence


def ui_capture(transport, case_id, operation_id, phase):
    if phase not in ('start','stop'):
        raise ValueError('UI capture phase must be start or stop')
    _, run_id, common = ui_identity(transport, case_id)
    body = {**common,'ui_run_id':run_id,'case_id':case_id,
            'operation_id':operation_id,'action':'capture-'+phase,'arguments':{}}
    result = post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-capture',
                  body, timeout=1900 if phase=='stop' else 400)
    if not isinstance(result,dict) or result.get('status') != 'done' or result.get('action') != 'capture-'+phase:
        raise ValueError('Scoped capture operation is unconfirmed')
    return result.get('result')


def ui_reconcile(transport, case_id, operation_id, action):
    if action not in ('screenshot','capture-start','capture-stop'):
        raise ValueError('Only evidence operations can be reconciled automatically')
    _, run_id, common = ui_identity(transport, case_id)
    body = {**common,'ui_run_id':run_id,'case_id':case_id,
            'operation_id':operation_id,'action':action}
    result = post(transport,'/v1/reservations/'+transport['assignment']['reservation_id']+'/ui-reconcile',body)
    if not isinstance(result,dict) or result.get('status') != 'done' or result.get('action') != action:
        raise ValueError('UI evidence operation remains uncertain')
    return result.get('result')


def main():
    action = sys.argv[1:2]
    transport = json.loads(Path(__file__).with_name('transport.json').read_text())
    if action == ['reserve'] and len(sys.argv) == 2:
        if transport['identity']['step_id'] != 'allocate':
            raise ValueError('Allocator transport required')
        result = reserve(transport)
    elif action == ['ui-register'] and len(sys.argv) == 3:
        result = ui_register(transport,sys.argv[2])
    elif action == ['ui-action'] and len(sys.argv) == 6:
        result = ui_action(transport,*sys.argv[2:])
    elif action == ['ui-complete'] and len(sys.argv) == 4:
        result = ui_complete(transport,*sys.argv[2:])
    elif action == ['ui-screenshot'] and len(sys.argv) == 4:
        result = ui_screenshot(transport,*sys.argv[2:])
    elif action == ['ui-capture'] and len(sys.argv) == 5:
        result = ui_capture(transport,*sys.argv[2:])
    elif action == ['ui-reconcile'] and len(sys.argv) == 5:
        result = ui_reconcile(transport,*sys.argv[2:])
    else:
        raise ValueError('Expected reserve, ui-register CASE, ui-action CASE OP_ID ACTION ARGS_FILE, ui-screenshot CASE OP_ID, ui-capture CASE OP_ID start|stop, ui-reconcile CASE OP_ID ACTION, or ui-complete CASE RECEIPTS_FILE')
    print(json.dumps(result))


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
