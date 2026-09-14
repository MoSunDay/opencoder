#!/usr/bin/python3
"""Deterministic exec process used by orchestration integration tests."""
import json
import os
import re
import time
import sys
import uuid
from pathlib import Path

prompt = sys.stdin.read()
args = sys.argv[1:]
thread = args[2] if len(args) > 2 and args[1] == 'resume' else str(uuid.uuid4())
if 'Decide the next workflow operation.' in prompt:
    state = json.loads(prompt.split('STATE=')[-1].split('\nTODO_SUMMARY=')[0])
    if all(t['status'] == 'passed' for t in state['todos'].values()):
        answer = {'operation': 'complete', 'reason': 'all accepted'}
    else:
        answer = {'operation': 'dispatch', 'todos': [{'todo_id': 't1', 'context_mode': 'new'}], 'reason': 'ready'}
elif 'Accept or reject one TODO candidate.' in prompt:
    answer = {'operation': 'accept', 'reason': 'verified', 'mark_milestone': True}
elif 'MATRIX_CANDIDATE' in prompt:
    answer = {'status': 'candidate', 'summary': 'done', 'result': 'MATRIX_ANSWER', 'verification': 'checked',
              'evidence_refs': [], 'recovery_context': {'summary': 'done', 'refs': []}}
elif '你是团队队长' in prompt:
    # Captain decision prompts (plan/summary/closing) arrive un-prefixed; the
    # one JSON reply satisfies every decision shape and free-text answers.
    answer = {'question': 'inspect', 'participants': ['plan'], 'summary': 'aligned', 'aligned': True,
              'complete': True, 'final_summary': 'MATRIX_TEAM_DONE'}
else:
    answer = 'MATRIX_ANSWER'
record = {'args': args, 'prompt': prompt, 'cwd': os.getcwd(), 'pid': os.getpid(), 'thread': thread, 'answer': answer}
with Path(os.environ['MATRIX_CAPTURE']).open('a') as stream:
    stream.write(json.dumps(record) + '\n')
def emit(value):
    print(json.dumps(value), flush=True)
emit({'type': 'thread.started', 'thread_id': thread})
emit({'type': 'turn.started'})
if 'MATRIX_HANG' in prompt:
    time.sleep(120)
manifest = re.findall(r'^OPENCODER_DELIVERABLE_MANIFEST=(.+)$', prompt, flags=re.M)
if manifest:
    manifest = Path(manifest[-1])
    name = manifest.name[:-len('-deliverables.json')] + '.txt'
    Path(name).write_text('MATRIX_DELIVERABLE:' + name)
    paths = ['../outside-workdir'] if 'MATRIX_INVALID_ARTIFACT' in prompt else [name]
    manifest.write_text(json.dumps(paths))
emit({'type': 'item.completed', 'item': {'type': 'agent_message', 'id': 'answer',
      'text': answer if isinstance(answer, str) else json.dumps(answer)}})
emit({'type': 'turn.completed', 'usage': {'input_tokens': 10, 'output_tokens': 5, 'cached_input_tokens': 0}})
