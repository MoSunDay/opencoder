"""Run history acceptance in isolated real Server/Host/Runtime/runc processes.

python3 main.py --binaries BUNDLE/bin --rootfs PAIRED_ROOTFS --output EVIDENCE
Only this environment's uniquely named units and test data are created.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1] / 'smooth_release'))
from environment import Environment, until
from rolling.native import publish_binary
from model import MARKER, RESULT, answer, plan


def publish_artifact(env):
    content = b'Run history artifact\n'
    declaration = {'files': [{'path': 'report.txt', 'bytes': len(content),
                             'sha256': hashlib.sha256(content).hexdigest()}]}
    source = '#include <stdio.h>\nint main(void){FILE*f;'
    for name, data in [('report.txt', content.decode()), ('artifacts.json', json.dumps(declaration)),
                       ('output.json', json.dumps({'passed': True, 'summary': RESULT}))]:
        source += f'f=fopen({json.dumps(name)},"w");if(!f)return 1;fputs({json.dumps(data)},f);fclose(f);'
    source += 'return 0;}\n'
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / 'fixture.c').write_text(source)
        subprocess.run(['cc', '-static', '-O2', '-s', str(root / 'fixture.c'), '-o', str(root / 'fixture')], check=True)
        resource = publish_binary(env.settings.resource_url, 'history-artifact', (root / 'fixture').read_bytes(), env)
    env.api('/api/dag/defs', 'POST', {'spec': {'name': 'history-artifact', 'steps': [
        {'name': 'write', 'kind': {'type': 'binary', 'resource': resource}}]}})


def run(env, output):
    env.model.answer = answer
    for workdir in [env.settings.agent_workdir, env.settings.server_workdir]:
        config_file = workdir / 'opencoder.json'
        config = json.loads(config_file.read_text())
        config['context_limit'] = 256000
        config_file.write_text(json.dumps(config))
    record = env.warm('history-acceptance')
    env.switch(record)
    # Exercise the same SPA and APIs behind a URL prefix in this private ingress.
    ingress = env.settings.nginx_include
    content = ingress.read_text().replace('client_max_body_size 2m;',
        'client_max_body_size 2m; rewrite ^/history(/.*)$ $1 last;', 1)
    ingress.write_text(content)
    subprocess.run([env.nginx, '-e', str(env.root / 'nginx.log'), '-p', str(env.root),
        '-c', str(env.root / 'nginx.conf'), '-s', 'reload'], check=True)
    publish_artifact(env)
    identifier = 'brain-history-acceptance'
    definition = plan()
    receipt = env.api('/api/brain/runs', 'POST', {'id': identifier, 'schema_version': 7,
        'plan': definition, 'inputs': {'request': MARKER}})
    assert receipt['run_id'] == identifier
    def terminal():
        view = env.api(f'/api/brain/runs/{identifier}/layered')
        (output / 'view.json').write_text(json.dumps(view, indent=2))
        if view['run']['phase'] in ['blocked', 'failed', 'cancelled']:
            raise AssertionError('Run failed: ' + json.dumps(view['run']))
        return view if view['run']['phase'] == 'completed' else None
    view = until(terminal, 'Brain completion', 240)
    assert view['run']['round'] == 2 and len(view['operations']) == 6
    visits = [event for event in view['events'] if event['event_type'] == 'layer_started']
    assert [(event['round'], event['layer']) for event in visits] == [(1, 1), (1, 2), (2, 1), (2, 2)]
    for prior, following in zip(visits, visits[1:]):
        barrier = next(event for event in view['events'] if event['event_type'] == 'layer_barrier_reached'
                       and event['activation'] == prior['activation'])
        assert barrier['seq'] < following['seq']
    assert all(operation['status'] == 'done' for operation in view['operations'])
    history = env.api('/api/brain/runs?limit=1')
    assert history['runs'][0]['name'] == definition['title']
    frozen = env.api(f'/api/executions/{identifier}')
    assert frozen['request']['input']['layered_request']['inputs']['request'] == MARKER
    for operation in view['operations']:
        detail = env.api('/api/executions/' + operation['execution_id'])
        assert RESULT in json.dumps(detail['result']['scheduler_output'])
    ui = HERE.parent / 'ui'
    subprocess.run(['node', str(ui / 'history-live.js'), env.settings.public_url + '/history',
        str(output / 'browser'), str(env.settings.token_file), identifier, '-'], check=True)
    browser = json.loads((output / 'browser/result.json').read_text())
    assert browser['downloads'], 'Browser must download and verify an actual artifact'
    subprocess.run(['node', str(ui / 'run-canvas.js'), env.settings.public_url,
        str(output / 'canvas'), identifier, str(env.settings.token_file)], check=True)
    return {'result': 'PASS', 'run_id': identifier, 'process_evidence': str(env.root),
        'operations': 6, 'rounds': 2, 'browser_checks': len(browser['checks']),
        'downloads': browser['downloads'], 'build_info': env.info}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binaries', required=True, type=Path)
    parser.add_argument('--rootfs', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    names = ['opencoder-server', 'opencoder-agent', 'dag-runner',
             'agent-step-runner', 'agent-session-runner']
    if (args.rootfs / 'usr/share/opencoder/codex-startup.json').exists():
        names.append('opencoder-cli')
    infos = [json.loads(subprocess.check_output([args.binaries / name, '--build-info'])) for name in names]
    assert all(info == infos[0] for info in infos), 'Execution binaries must be built together'
    digest = hashlib.sha256()
    for file in sorted((HERE.parents[3] / 'crates/web/spa/dist').rglob('*')):
        if file.is_file() and not file.is_symlink():
            digest.update(file.read_bytes())
    assert infos[0]['spa_sha256'] == digest.hexdigest(), 'Server must embed the current SPA'
    env = Environment(args.binaries.resolve(), shutil.which('nginx'), args.rootfs, args.output)
    try:
        result = run(env, args.output)
        (args.output / 'result.json').write_text(json.dumps(result, indent=2))
        print(json.dumps(result), flush=True)
    except BaseException:
        env.diagnose()
        raise
    finally:
        env.close()


if __name__ == '__main__':
    main()
