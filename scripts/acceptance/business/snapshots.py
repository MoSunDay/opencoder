"""Copy independently owned Git stores and selected source/context snapshots."""
from pathlib import Path
import shutil
from common import BASE, CASE, CONFIG, SOURCE, TARGET, emit, git, read, run, sha, write


def clone(source, target, revision, checkout=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    if not (target / '.git').exists():
        # A root checkout creates empty directories for its Git submodules.
        # Never let git walk up to the parent repository and mistake it for this one.
        if target.exists():
            if any(target.iterdir()):
                raise RuntimeError(f'Expected an empty submodule placeholder: {target}')
            target.rmdir()
        run(['git', '-c', 'core.hooksPath=/dev/null', 'clone', '--quiet', '--no-hardlinks',
             '--no-checkout', '--', source, target], timeout=3600)
    git(target, 'cat-file', '-e', revision + '^{commit}')
    git(target, 'update-ref', 'HEAD', revision)
    if checkout:
        git(target, 'checkout', '--detach', revision)
    objects = target / '.git/objects/info/alternates'
    assert not objects.exists(), 'Source Git objects must not remain shared'
    return str(target)


def prepare(root, release):
    runtime = root / 'runtime'
    state = root / 'prepared.json'
    if state.exists():
        return read(state)
    config = read(CONFIG)
    old = Path(config['dataDir']) / 'events' / CASE / 'evidence'
    brief = read(old / 'brief.json')
    baseline = root / 'evidence' / 'baseline'
    baseline.mkdir(parents=True, exist_ok=True)
    # Preserve request and collection context, never old model conclusions.
    write(baseline / 'request.json', brief['request'])
    write(baseline / 'context.json', brief['context'])
    revisions = {name: value['revision'] for name, value in brief['context']['repositories'].items()}
    workspace = runtime / 'workspace'
    root_revision = git(SOURCE, 'rev-parse', 'HEAD')
    emit('snapshot_workspace', revision=root_revision)
    clone(SOURCE, workspace, root_revision, checkout=True)
    repositories = {}
    for name, source in config['repositories'].items():
        emit('snapshot_repository', repository=name, revision=revisions[name])
        repositories[name] = clone(source, workspace / 'repos' / name, revisions[name])
    # Explicitly retain the real branch and base/target ancestry in the copied repository.
    target_repo = repositories['jianying-openagent-api']
    git(target_repo, 'merge-base', '--is-ancestor', BASE, TARGET)
    git(target_repo, 'merge-base', '--is-ancestor', TARGET, 'refs/remotes/origin/master')
    contexts = []
    for i, source in enumerate(config['contextFiles']):
        target = runtime / 'context' / f'{i}-{Path(source).name}'
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        contexts.append(str(target))
    context_capture = runtime / 'context' / 'capture.json'
    shutil.copy2(config['contextCaptureFile'], context_capture)
    resources = runtime / 'resources'
    resources.mkdir(parents=True, exist_ok=True)
    original = Path('/mnt/opencoder-agents')
    for name in ['eval-diagnose', 'regression-test']:
        shutil.copytree(original / name, resources / name)
        for category in ['prompts', 'skills', 'tools']:
            source = original / category / name
            meta = read(source / 'meta.json')
            target = resources / category / name
            target.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source / 'meta.json', target / 'meta.json')
            shutil.copytree(source / f"v{meta['current']}", target / f"v{meta['current']}")
    # Existing credential material is copied to a private home, never switched in place.
    codex_home = runtime / 'auth-source'
    codex_home.mkdir(parents=True, exist_ok=True)
    for name in ['config.toml', 'accounts', 'auth.json', 'installation_id', 'models_cache.json']:
        source = Path(config['codexHome']) / name
        if source.is_dir():
            shutil.copytree(source, codex_home / name)
        elif source.exists():
            shutil.copy2(source, codex_home / name)
    result = {'repositories': repositories, 'revisions': revisions, 'workspace': str(workspace),
        'workspaceRevision': root_revision, 'contextFiles': contexts,
        'contextCaptureFile': str(context_capture), 'resources': str(resources),
        'codexHome': str(codex_home), 'release': str(release)}
    write(state, result)
    write(root / 'evidence' / 'snapshot-manifest.json', {
        'workspaceRevision': root_revision, 'repositories': revisions,
        'target': TARGET, 'base': BASE,
        'contextSha256': {str(p): sha(p) for p in [*contexts, str(context_capture)]},
        'gitObjectsIndependent': True, 'uncommittedSourceExcluded': True})
    return result
