"""Usage: python -m scripts.maintenance.brain_cleanup --output /tmp/review.json"""
import argparse
import json
from pathlib import Path
from .preview import scan
from .model import digest

parser = argparse.ArgumentParser(description='Read-only retired Brain cleanup preview')
parser.add_argument('--config', default='/etc/opencoder/server/opencoder.json')
parser.add_argument('--runtime-root', action='append', default=[], type=Path, help='Additional node data directory, validated by node-id')
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--apply-reviewed', help='Digest of the reviewed --output manifest')
parser.add_argument('--backup', type=Path)
args = parser.parse_args()
if args.apply_reviewed:
    from .apply import apply
    if not args.backup:
        parser.error('--backup is required when applying a reviewed manifest')
    apply(json.loads(args.output.read_text()), args.apply_reviewed, args.backup)
    print(json.dumps({'applied': str(args.output), 'backup': str(args.backup)}))
    raise SystemExit(0)
manifest = scan(args.config, args.runtime_root)
args.output.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + '\n')
print(json.dumps({'manifest': str(args.output), 'digest': digest(manifest),
                  'roots': len(manifest['roots']), 'executions': len(manifest['executions']),
                  'plan_versions': len(manifest['plan_versions']),
                  'database_rows': sum(len(change['rows']) for change in manifest['databases']),
                  'directories': len(manifest['directories'])}, ensure_ascii=False))
