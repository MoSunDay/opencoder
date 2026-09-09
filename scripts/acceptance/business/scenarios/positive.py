"""Prepare bounded business examples without copying or changing business repos."""
from pathlib import Path
import json
import os
import selectors
import shutil
import subprocess
from common import CONFIG, HERE, read, write


def prepare(root, release):
    runtime = root / 'runtime'
    temporary = runtime / 'fixtures'
    temporary.mkdir(exist_ok=True)
    source = subprocess.Popen(['/usr/bin/node', str(HERE / 'scenarios/fixture.mjs'), str(release)],
        stdout=subprocess.PIPE, stderr=(root / 'evidence/fixture.log').open('w'),
        text=True, stdin=subprocess.DEVNULL, env={**os.environ, 'TMPDIR': str(temporary)})
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(source.stdout, selectors.EVENT_READ)
            if not selector.select(30):
                raise RuntimeError('Controlled business fixture did not become ready')
        fixture = json.loads(source.stdout.readline())
        config = fixture['config']
        installed = read(CONFIG)
        for key in ['codexCommand', 'codexHome', 'codexAuthSlot', 'executorIsolation']:
            config[key] = installed[key]
        auth = runtime / 'auth-source'
        auth.mkdir(mode=0o700)
        for name in ['config.toml', 'accounts', 'auth.json', 'installation_id', 'models_cache.json']:
            original = Path(config['codexHome']) / name
            if original.is_dir(): shutil.copytree(original, auth / name)
            elif original.exists(): shutil.copy2(original, auth / name)
        config['codexHome'] = str(auth)
        capture = runtime / 'context.json'
        write(capture, {'code': {}})
        config['contextCaptureFile'] = str(capture)
        config_file = runtime / 'fixture-config.json'
        write(config_file, config)
        resources = runtime / 'resources'
        resources.mkdir()
        for agent in ['eval-diagnose', 'regression-test']:
            shutil.copytree(Path('/mnt/opencoder-agents') / agent, resources / agent)
            for category in ['prompts', 'skills', 'tools']:
                original = Path('/mnt/opencoder-agents') / category / agent
                meta = read(original / 'meta.json')
                target = resources / category / agent
                target.mkdir(parents=True)
                shutil.copy2(original / 'meta.json', target / 'meta.json')
                shutil.copytree(original / f"v{meta['current']}", target / f"v{meta['current']}")
        requests = fixture['requests']
        requests['eval-diagnose']['eventId'] += '-' + root.name
        write(root / 'evidence/baseline/request.json', requests['eval-diagnose'])
        write(root / 'evidence/controlled-input.json', fixture['input'])
        prepared = {'workspace': fixture['root'], 'repositories': config['repositories'],
                    'contextFiles': config['contextFiles'], 'contextCaptureFile': str(capture),
                    'codexHome': str(auth), 'resources': str(resources), 'release': str(release),
                    'configFile': str(config_file), 'requiresGo': False, 'requests': requests,
                    'expectedVerdicts': {'eval-diagnose': 'clean', 'regression-test': 'pass'}}
        write(root / 'prepared.json', prepared)
        return prepared, source
    except BaseException:
        source.terminate()
        source.wait(timeout=10)
        raise
