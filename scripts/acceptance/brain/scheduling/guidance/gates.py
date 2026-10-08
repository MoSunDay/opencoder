"""Hold real child tools until guidance is observed; only touch owned workspaces."""
from pathlib import Path
import json
import shlex

from environment import require

READY = '.brain-guidance-ready'
RELEASE = '.brain-guidance-release'
KINDS = ('agent', 'operator', 'team')


def diagnostic(seconds=600):
    require(1 <= seconds <= 600, 'guidance gate timeout must be 1..600 seconds')
    program = ('from pathlib import Path\nimport time\n'
               f'Path({READY!r}).touch()\n'
               f'deadline = time.monotonic() + {seconds}\n'
               f'while not Path({RELEASE!r}).exists():\n'
               '    if time.monotonic() >= deadline:\n'
               '        raise TimeoutError("guidance gate was not released")\n'
               '    time.sleep(0.2)\n'
               'print("diagnostic finished")\n')
    return f'timeout {seconds} python3 -c ' + shlex.quote(program)


def workspace(env, operation):
    require(operation['run_id'] in env.created, 'guidance gate is not owned by this acceptance')
    kind, identifier = operation['execution_kind'], operation['execution_id']
    require(kind in KINDS and '/' not in identifier and identifier not in ('', '.', '..'),
            'invalid guidance gate execution')
    root = env.execution_data(operation).resolve()
    path = (root / kind / identifier / 'workspace').resolve()
    require(path.is_relative_to(root), 'guidance gate is outside the owned Runtime')
    return path


def ready(env, view):
    targets = [op for op in view['operations'] if op['execution_kind'] in KINDS]
    return len(targets) == 3 and all(op['status'] == 'running' and
        (workspace(env, op) / READY).is_file() for op in targets)


def release(env, view):
    for operation in view['operations']:
        if operation['execution_kind'] in KINDS:
            path = workspace(env, operation)
            if (path / READY).is_file():
                (path / RELEASE).touch()


def delivered(env, identifier, sequence):
    require(identifier in env.created, 'guidance receipt is not owned by this acceptance')
    root = Path(env.record['runtime_data']).resolve()
    path = (root / 'brain' / identifier / 'execution.json').resolve()
    require(path.is_relative_to(root), 'guidance receipt is outside the owned Runtime')
    record = json.loads(path.read_text())
    return record['annotations'].get('layered_guidance_ack', 0) >= sequence
