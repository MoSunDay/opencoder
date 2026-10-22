#!/usr/bin/env python3
"""Cross-version schema and ACK acceptance with private native processes only."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling import manifest
import reports
import schema


def guard_checks(old, bridge, candidate, legacy_source):
    old, bridge, candidate = [json.loads((p / 'manifest.json').read_text()) for p in (old, bridge, candidate)]
    spec = importlib.util.spec_from_file_location('legacy_manifest', legacy_source)
    legacy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(legacy)
    assert manifest.report_state(old) == {'reader': 0, 'writer': 0}
    assert manifest.report_state(bridge) == {'reader': 1, 'writer': 0}
    assert manifest.report_state(candidate) == {'reader': 1, 'writer': 1}
    manifest.writer_preflight(candidate, bridge)
    manifest.compatible(candidate, [old, bridge])
    manifest.compatible(bridge, [candidate])
    results = []
    for label, action in [
        ('old controller rejects writer overlap', lambda: legacy.compatible(candidate, [old, bridge])),
        ('old controller rejects rollback across writer format', lambda: legacy.compatible(old, [candidate])),
        ('new controller rejects skipping reader bridge', lambda: manifest.writer_preflight(candidate, old)),
        ('new controller rejects rollback without reader', lambda: manifest.compatible(old, [candidate])),
    ]:
        try:
            action()
        except ValueError as error:
            results.append({'scenario': label, 'rejected': str(error)})
        else:
            raise AssertionError(label)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['old', 'bridge', 'candidate', 'legacy-controller', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    args = parser.parse_args()
    root = args.output.resolve()
    repo = Path(__file__).resolve().parents[3]
    if root.exists() or root.is_relative_to(repo):
        parser.error('output must be a new directory outside the repository')
    root.mkdir(parents=True)
    receipt = {'passed': False, 'bundles': {}}
    try:
        for name in ['old', 'bridge', 'candidate']:
            bundle = getattr(args, name).resolve()
            receipt['bundles'][name] = {'path': str(bundle), 'manifest_sha256': hashlib.sha256((bundle / 'manifest.json').read_bytes()).hexdigest()}
        manifest.verify(args.candidate)
        receipt['guards'] = guard_checks(args.old, args.bridge, args.candidate, args.legacy_controller)
        receipt['schema'] = schema.exercise(root / 'schema', args.old, args.candidate)
        receipt['reports'] = reports.exercise(root / 'reports', args.bridge, args.candidate)
        receipt['passed'] = True
    finally:
        (root / 'receipt.json').write_text(json.dumps(receipt, indent=2))
    print(json.dumps({'passed': True, 'evidence': str(root)}))


if __name__ == '__main__':
    main()
