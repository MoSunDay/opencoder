"""Prepare verified dependency archives in a private cache before starting jobs."""
from pathlib import Path
import os
from common import HERE, TARGET, git, run, sha, write


def prepare_modules(runtime, workspace, environment):
    source = Path(run(['/usr/local/bin/go', 'env', 'GOMODCACHE']).strip()) / 'cache/download'
    seed = runtime / 'go-module-downloads'
    run(['cp', '-a', '--reflink=auto', source, seed])
    root = runtime / 'dependency-preflight'
    home, repository, modules = root / 'home', root / 'repository', root / 'modules'
    for directory in [home, repository, modules / 'cache']:
        directory.mkdir(parents=True, exist_ok=True)
    (modules / 'cache/download').symlink_to(seed)
    for name in ['go.mod', 'go.sum']:
        (repository / name).write_text(git(Path(workspace) / 'repos/jianying-openagent-api',
                                         'show', TARGET + ':' + name) + '\n')
    # Older Git ignores GIT_CONFIG_GLOBAL. A private HOME works on both versions.
    (home / '.gitconfig').write_text(
        '[url "ssh://git@code.byted.org/"]\n\tinsteadOf = https://code.byted.org/\n')
    env = {**environment, 'HOME': str(home), 'GOMODCACHE': str(modules),
           'GOCACHE': str(root / 'build'), 'TMPDIR': str(runtime / 'tmp'),
           'GIT_CONFIG_GLOBAL': str(home / '.gitconfig'),
           'GIT_SSH_COMMAND': 'ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UpdateHostKeys=no'}
    env['NO_PROXY'] = environment.get('NO_PROXY', os.environ.get('NO_PROXY', '')) + ',code.byted.org'
    env['no_proxy'] = env['NO_PROXY']
    run(['/usr/bin/python3', HERE / 'scope.py', runtime.parent, 'go-dependencies',
         '/usr/local/bin/go', 'mod', 'download'], cwd=repository, env=env, timeout=600)
    write(runtime / 'go-module-manifest.json', {
        str(file.relative_to(seed)): sha(file) for file in seed.rglob('*') if file.is_file()})
