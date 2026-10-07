"""Publish bounded native capabilities and a business-only milestone plan."""
import json
from pathlib import Path
import subprocess

from rolling.native import publish_binary


def prepare(env):
    script = (Path(__file__).parent / 'fixtures/native.py').read_text()
    source = '#include <unistd.h>\nint main(int n,char **v){execl("/usr/bin/python3","python3","-c",' + json.dumps(script) + ',n>1?v[1]:"",n>2?v[2]:"abs(a) + abs(b)",(char*)0);return 127;}\n'
    directory = env.output / 'native'
    directory.mkdir()
    (directory / 'fixture.c').write_text(source)
    subprocess.run(['cc', '-static', '-O2', '-s', str(directory / 'fixture.c'),
                    '-o', str(directory / 'fixture')], check=True)
    resource = publish_binary(env.settings.resource_url, env.tag,
                              (directory / 'fixture').read_bytes(), env)
    capabilities = {}
    descriptions = {
        'prepare': 'Prepare the imported arithmetic source or apply a corrected arithmetic expression. '
                   'Input args is a one-element array containing an expression using a, b, abs and arithmetic. '
                   'The initial source is abs(a) + abs(b). Output check contains source, revision and args. '
                   'This is the only capability that produces a revised source artifact.',
        'fast': 'Run ordinary positive-number tests on exactly the supplied source expression.',
        'hold': 'Run independent ordinary zero and positive tests; the acceptance controller releases a gate automatically.',
        'edge': 'Run signed-number boundary tests on the supplied expression, returning the actual checks and passed boolean.',
        'oversize': 'Return intentionally oversized diagnostic evidence for output-contract acceptance.',
        'missing': 'Return evidence without the required field for output-contract acceptance.',
        'error': 'Terminate with an intentional native execution error for barrier acceptance.',
    }
    for mode, description in descriptions.items():
        name = env.tag + '-' + mode
        env.api('POST', '/api/dag/defs', {'spec': {'name': name,
            'description': description + ' Bind args from the latest prepared output /check/args. '
                'Each DAG receives its own source input; never assume another execution workspace exists.',
            'steps': [{'name': 'check', 'timeout_secs': 960,
                       'kind': {'type': 'binary', 'resource': resource, 'args': [mode]}}]}})
        capabilities[mode] = 'dag-' + name
    env.save('native/resources', {'resource': resource, 'capabilities': capabilities})
    return capabilities


def plan(capabilities):
    layers = [
        ('prepare', '准备版本', 'Import the specified baseline source first. After actual diagnostics, apply a concrete correction.',
         'A source artifact, its revision and args exist. Preparation does not prove business correctness.'),
        ('normal', '常规验证', 'Run both independent ordinary tests on the latest prepared version.',
         'Both current-version test outputs have passed=true and identical revision.'),
        ('boundary', '边界验收', 'Verify signed arithmetic on the same current version.',
         'The actual boundary checks pass and passed=true for the latest prepared revision.'),
    ]
    return {'schema_version': 7, 'title': '按执行证据验收加法实现',
        'objective': 'Deliver calculate(a,b) implementing signed addition. First import and test the supplied '
            'baseline exactly as supplied; make changes only after actual test diagnostics. '
            'Use complete output fields to decide if milestones are met. Translate any necessary repair '
            'into a concrete expression passed to the capability that can modify source. '
            'A successful process exit and summary do not establish that checks passed. '
            'Test capabilities are read-only and cannot repair source. Do not ask a human to release gates; '
            'the acceptance controller does that. Never instruct child tasks to schedule other layers.',
        'inputs': {'initial_args': ['abs(a) + abs(b)']}, 'max_rounds': 5,
        'layers': [{'layer_id': key, 'title': title, 'task': task, 'objective': task,
                    'success_criteria': criteria} for key, title, task, criteria in layers],
        'nodes': [{'node_id': mode, 'layer_id': layer, 'title': mode,
                   'objective': ('Bind args from root initial_args on first import; for repairs bind a concrete '
                                 'corrected expression using actual failure evidence.' if mode == 'prepare' else
                                 'Bind args through the latest prepared execution output JSON pointer /check/args.'),
                   'capability_id': capabilities[mode]}
                  for mode, layer in [('prepare', 'prepare'), ('fast', 'normal'),
                                      ('hold', 'normal'), ('edge', 'boundary')]]}
