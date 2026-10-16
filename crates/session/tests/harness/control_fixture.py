#!/usr/bin/python3
"""Local external-protocol fixture: no remote model or credentials."""
import json
import os
from pathlib import Path
import sys
import time

context = json.loads(os.environ['OPENCODER_HARNESS_CONTEXT'])
prompt = sys.stdin.read()
control = context.get('control')
mode = os.environ.get('CONTROL_TEST_MODE', 'accepted')

def append(name, data):
    with Path(name).open('a') as f:
        f.write(json.dumps(data) + '\n')

def emit(data):
    print(json.dumps(data), flush=True)

if control:
    request = json.loads(prompt)
    assert all(request[key] == value for key, value in control.items())
    control = request
    append('controls.jsonl', control)
    key = control['input_id']
    effect = Path('effect-' + key)
    if not effect.exists():
        effect.write_text('once')
        append('effects.jsonl', {'input_id': key, 'action': control['action']})
        if mode == 'pending_once' and control['action'] == 'steer':
            emit({'input_id': key, 'status': 'pending'})
            sys.exit(0)
    Path('released').write_text(key)
    emit({'input_id': key, 'status': 'rejected' if mode == 'rejected' else 'accepted',
          'message': 'expected turn already ended' if mode == 'rejected' else '',
          'evidence': {'original_input': control['parent_input_id']}})
    sys.exit(0)

append('normal.jsonl', {'context': context, 'prompt': prompt, 'pid': os.getpid()})
emit({'type': 'thread.started', 'thread_id': 'fixture-thread'})
emit({'type': 'turn.started'})
Path('started').write_text('ready')
if len(Path('normal.jsonl').read_text().splitlines()) == 1:
    deadline = time.monotonic() + 30
    while not Path('released').exists():
        if time.monotonic() > deadline:
            sys.exit(7)
        time.sleep(.02)
append('finished.jsonl', {'pid': os.getpid()})
emit({'type': 'item.completed', 'item': {'id': 'answer', 'type': 'agent_message', 'text': 'finished original'}})
emit({'type': 'turn.completed', 'usage': {'input_tokens': 1, 'output_tokens': 1}})
