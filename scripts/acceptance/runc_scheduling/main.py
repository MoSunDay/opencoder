#!/usr/bin/python3
"""Exercise one Server and two real native DAG Workers in a private namespace."""
import argparse
import json
import os
from pathlib import Path
import sys
import traceback
from runtime import Runtime
from samples import exercise, observe


def bundle_binaries(bundle):
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
    from rolling.manifest import verify
    verify(bundle)
    return {name: str(bundle.resolve() / 'bin' / name)
            for name in ('opencoder-server', 'opencoder-agent')}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--rootfs', required=True, type=Path)
    parser.add_argument('--observe-seconds', type=int, default=0)
    binaries = parser.add_mutually_exclusive_group(required=True)
    binaries.add_argument('--platform-bundle', type=Path)
    binaries.add_argument('--bin-dir', type=Path)
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.observe_seconds < 0:
        parser.error('--observe-seconds must not be negative')
    root = args.root.resolve()
    if root.parent != Path('/root/.cache/opencoder-e2e') or not root.name.startswith('20'):
        raise ValueError('Use a new dated private opencoder-e2e directory')
    if not args.inside:
        os.execv('/usr/bin/unshare', ['unshare', '-m', '--propagation', 'private',
            '/usr/bin/python3', str(Path(__file__).resolve()), *sys.argv[1:], '--inside'])
    if root.exists():
        raise ValueError('The evidence directory must not already exist')
    if args.platform_bundle:
        paths = bundle_binaries(args.platform_bundle)
    else:
        paths = {name: str(args.bin_dir.resolve() / name) for name in ['opencoder-server', 'opencoder-agent']}
    runtime = Runtime(root, paths, args.rootfs.resolve())
    receipt = {'passed': False, 'release_bundle': bool(args.platform_bundle)}
    try:
        nodes = runtime.launch()
        receipt.update(exercise(runtime, nodes))
        if args.observe_seconds:
            receipt.update(observe(runtime, nodes, args.observe_seconds))
        receipt['passed'] = True
    except Exception as error:
        receipt['error'] = str(error)
        (root / 'evidence/failure.txt').write_text(traceback.format_exc())
        raise
    finally:
        try:
            receipt['cleanup'] = runtime.close()
        except Exception as error:
            receipt.update(passed=False, cleanup_error=str(error))
            raise
        finally:
            (root / 'evidence/result.json').write_text(json.dumps(receipt, indent=2))
    print(json.dumps(receipt), flush=True)


if __name__ == '__main__':
    main()
