#!/usr/bin/env python3
"""Run isolated maintenance acceptance with retained failure and recovery evidence."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time
import traceback

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling import manifest
from rolling.state import write
from evidence import source_inventory, unchanged_source, fingerprint
from fixture import prepare, source_configs, launch, configuration_scope
from inputs import old_input, corrective_input, binary_inventory, image_input, package_debug
import scenarios


def arguments(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    candidate = parser.add_mutually_exclusive_group(required=True)
    candidate.add_argument('--bin-dir', type=Path, help='immutable debug binaries; result release_bundle=false')
    candidate.add_argument('--platform-bundle', type=Path, help='verified immutable release bundle')
    parser.add_argument('--rootfs', type=Path, required=True)
    parser.add_argument('--old-bundle', type=Path, required=True, help='actual retained data-format1 platform bundle')
    parser.add_argument('--corrective-bundle', type=Path, required=True, help='verified compatible bundle from a different compiled commit')
    parser.add_argument('--data-parent', type=Path, default=Path('/root/.cache/opencoder-e2e'))
    return parser.parse_args(argv)


def require_root():
    if os.geteuid() != 0:
        raise ValueError('root is required for private mounts, NFS and runc')



def main():
    args = arguments()
    require_root()
    parent = args.data_parent.resolve()
    if any(parent.is_relative_to(Path(path)) for path in ('/var/lib/opencoder-platform', '/etc', '/run/systemd')):
        raise ValueError('data-parent must be outside production state and system configuration')
    if any(parent.is_relative_to(path.resolve()) for path in
           (args.rootfs, args.old_bundle, args.corrective_bundle, args.platform_bundle or args.bin_dir)):
        raise ValueError('data-parent must be outside the immutable input directories')
    parent.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='mr-', dir=parent))
    root.chmod(0o755)
    print(json.dumps({'evidence': str(root), 'stage': 'starting'}), flush=True)
    receipt = {'passed': False, 'release_bundle': args.platform_bundle is not None, 'evidence': str(root),
               'controller_source': str(Path(__file__).resolve().parents[3]),
               'isolation': {'namespace': os.readlink('/proc/self/ns/mnt'),
                  'host_namespace': os.readlink('/proc/1/ns/mnt'), 'owned_global_mount_paths': True,
                  'services': 'real systemd units with remapped private service dependencies',
                  'real': ['Nginx', 'NFS', 'runc', 'old Server/Host/Runtime/resources', 'candidate Server/Host/Runtime']},
               'limitations': ['Only this run-owned systemd units, mounts, ports, credentials and data are changed.',
                  'Schema 31 -> 32 is covered separately by Store catalog_maintenance and project_tags tests; this harness reports the actual old schema.',
                  'This fixture creates its own token/data/config and never reads production state, tokens or config.']}
    operations = control = None
    started = time.monotonic()
    try:
        old_manifest, old_info = old_input(args.old_bundle)
        if args.platform_bundle:
            bundle = args.platform_bundle.absolute()
            initial_files, info = binary_inventory(bundle / 'bin')
            import subprocess
            source = Path(__file__).resolve().parents[3]
            commit = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
            dirty = subprocess.check_output(['git', '-C', str(source), 'status', '--porcelain'], text=True).strip()
            if dirty or info['git_commit'] != commit:
                raise ValueError('formal acceptance requires the clean controller commit packaged in the bundle')
            receipt['controller_commit'] = commit
        else:
            if args.bin_dir.is_symlink():
                raise ValueError('bin-dir must be a real immutable directory')
            bundle = root / 'candidate'
            candidate, info = package_debug(args.bin_dir.absolute(), bundle)
            initial_files, _ = binary_inventory(args.bin_dir)
            receipt['limitations'].append('bin-dir packaging preserves real build metadata and uses unchanged strict bundle validation; dirty or unknown metadata is rejected.')
        receipt.update(build=info, old_build=old_info,
                       old_bundle=str(args.old_bundle), candidate_input=str(args.platform_bundle or args.bin_dir),
                       binary_inventory=initial_files, rootfs=str(args.rootfs.absolute()))
        candidate = manifest.verify(bundle)
        corrective, corrective_files, corrective_info = corrective_input(args.corrective_bundle, candidate)
        receipt['candidate_manifest'] = candidate
        receipt.update(corrective_manifest=corrective, corrective_build=corrective_info,
                       corrective_binary_inventory=corrective_files)
        receipt['input_rootfs_builds'] = image_input(args.rootfs)
        nginx = shutil.which('nginx') or '/usr/local/sbin/nginx'
        if not Path(nginx).is_file() or not shutil.which('runc'):
            raise ValueError('Nginx and runc are required')
        settings, operations, control = prepare(root, nginx)
        sources, targets, ports = source_configs(settings, root, args.rootfs.absolute())
        operations.source = sources['workspace']
        operations.source_frozen = source_inventory(operations.source)
        write(root / 'source-inventory.json', operations.source_frozen)
        receipt['source_inventory_sha256'] = fingerprint(operations.source_frozen)
        with configuration_scope(settings, root):
            old = launch(settings, operations, args.old_bundle, old_manifest, bundle / 'bin',
                         args.rootfs.absolute(), targets, ports)
            print(json.dumps({'evidence': str(root), 'stage': 'old-ready'}), flush=True)
            scenarios.run(settings, operations, old, bundle, candidate, args.corrective_bundle, control, receipt)
        actual_files, actual_info = binary_inventory(args.bin_dir or (args.platform_bundle / 'bin'))
        if actual_files != initial_files or actual_info != info:
            raise AssertionError('candidate input changed during acceptance')
        if binary_inventory(args.corrective_bundle / 'bin') != (corrective_files, corrective_info):
            raise AssertionError('corrective input changed during acceptance')
        mounts = {name: json.loads(operations.output('findmnt', '-J', '--mountpoint', str(target),
                  '-o', 'TARGET,SOURCE,FSTYPE,OPTIONS'))['filesystems'][0] for name, target in targets.items()}
        if any(value['fstype'] not in ('nfs', 'nfs4') or 'ro' not in value['options'].split(',') for value in mounts.values()):
            raise AssertionError('resource mounts are not actual read-only NFS')
        receipt['readonly_nfs'] = mounts
        receipt['passed'] = all(scenario['passed'] for scenario in receipt['scenarios'].values())
        if not receipt['passed']:
            receipt['error'] = 'one or more scenario preservation checks failed; inspect scenario evidence'
    except BaseException as error:
        receipt['error'] = str(error)
        (root / 'failure.txt').write_text(traceback.format_exc())
    finally:
        if operations is not None:
            receipt['cleanup'] = operations.close()
            try:
                if operations.source_frozen is not None:
                    receipt['source_after_cleanup_sha256'] = unchanged_source(operations.source, operations.source_frozen)
            except Exception as error:
                receipt['passed'] = False
                receipt['cleanup']['errors'].append(str(error))
                receipt['cleanup']['passed'] = False
        else:
            receipt['cleanup'] = {'passed': True, 'processes': [], 'remaining_mounts': [],
                                  'data_and_evidence_preserved': True}
        if control is not None:
            control.close()
        receipt['passed'] = receipt['passed'] and receipt['cleanup']['passed']
        receipt['seconds'] = time.monotonic() - started
        write(root / 'result.json', receipt)
    print(json.dumps(receipt, sort_keys=True), flush=True)
    return 0 if receipt['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
