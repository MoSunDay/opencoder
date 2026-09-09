"""Prepare and pin an opt-in offline environment for the fixed metricw target."""
from pathlib import Path
import copy
import re
import shutil


NAMES = ('req_resp_clip', 'overpass_clip', 'business_clip', 'filter')


def prepare(runtime, workspace):
    from common import TARGET, git, run, sha, write
    repository = Path(workspace) / 'repos/jianying-openagent-api'
    manifest = git(repository, 'show', TARGET + ':go.mod')
    if not re.search(r'code\.byted\.org/webcast/libs_misc/metricw v0\.0\.4\s', manifest):
        raise ValueError('Offline metrics fixture requires the verified metricw v0.0.4 dependency')
    version = re.search(r'^go (\d+\.\d+\.\d+)$', manifest, re.MULTILINE)
    if not version:
        raise ValueError('Fixed target must declare an exact Go toolchain version')
    private = runtime / 'dependency-preflight'
    toolchain = Path(run(['/usr/local/bin/go', 'env', 'GOROOT'], env={
        'GOTOOLCHAIN': 'go' + version[1], 'GOMODCACHE': str(private / 'modules'),
        'GOCACHE': str(private / 'build'), 'TMPDIR': str(runtime / 'tmp')}).strip())
    if not toolchain.is_dir() or runtime.resolve() not in toolchain.resolve().parents:
        raise ValueError('Fixture toolchain must be extracted into this private runtime')
    seed = runtime / 'go-module-downloads'
    write(runtime / 'go-module-manifest.json', {
        str(file.relative_to(seed)): sha(file) for file in seed.rglob('*') if file.is_file()})
    wrapper = runtime / 'bin/metricw_fixture.py'
    shutil.copy2(Path(__file__).with_name('metricw_fixture.py'), wrapper)
    adapter = runtime / 'bin/metricw.py'
    shutil.copy2(__file__, adapter)
    tree = git(repository, 'ls-tree', '-r', '--name-only', TARGET).splitlines()
    packages = sorted({str(Path(file).parent) for file in tree if file.endswith('_test.go')})
    files = [{'path': str(Path(package) / 'conf' / (name + '.yaml')),
              'content': 'local_first: true\nConfig: {}\n'} for package in packages for name in NAMES
             if str(Path(package) / 'conf' / (name + '.yaml')) not in tree]
    result = {'name': 'metricw-offline', 'target': TARGET, 'goVersion': version[1],
              'goroot': str(toolchain), 'wrapper': str(wrapper), 'files': files,
              'sha256': {str(wrapper): sha(wrapper), str(adapter): sha(adapter),
                         str(toolchain / 'bin/go'): sha(toolchain / 'bin/go')},
              'network': 'loopback only', 'configuration': 'absent metricw sampling/clipping; local logging',
              'targetCodeUnchanged': True}
    write(runtime / 'regression-fixture.json', result)
    write(runtime.parent / 'evidence/regression-fixture.json', result)
    return result


def configure(specification, fixture):
    """Preserve the requested Go assertions and add only declared test environment inputs."""
    result = copy.deepcopy(specification)
    if result.get('command', [])[:2] != ['go', 'test']:
        return result
    if result.get('repository') != 'repos/jianying-openagent-api':
        raise ValueError('Metrics fixture cannot apply to another repository')
    result.setdefault('readOnly', []).extend([fixture['goroot'], str(Path(fixture['wrapper']).parent)])
    supplied = {item['path'] for item in result.get('files', [])}
    if supplied.intersection(item['path'] for item in fixture['files']):
        raise ValueError('Test files conflict with the declared logging fixture')
    result.setdefault('files', []).extend(fixture['files'])
    environment = result.setdefault('environment', {})
    if environment.get('GOFLAGS'):
        raise ValueError('Fixture cannot replace existing Go build flags')
    environment.update({'GOROOT': fixture['goroot'], 'GOTOOLCHAIN': 'local',
        'PATH': fixture['goroot'] + '/bin:/usr/local/bin:/usr/bin:/bin',
        'TMPDIR': '/cache/tmp', 'GOFLAGS': '-ldflags=-checklinkname=0'})
    result['command'] = ['python3', fixture['wrapper'], *result['command']]
    return result
