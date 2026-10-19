"""Explicit preview/apply commands; this tool never schedules future deletion."""
import argparse
import json
from pathlib import Path
from .client import Client, write, require
from .inventory import inventory
from .apply import apply
from .restore import restore


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    preview = commands.add_parser('preview')
    for option in ('data-dir', 'binary-root', 'agents-root', 'config-path', 'output'):
        preview.add_argument('--' + option, required=True)
    preview.add_argument('--retention-days', type=int, default=7)
    execute = commands.add_parser('apply')
    for option in ('plan', 'base-url', 'token-file'):
        execute.add_argument('--' + option, required=True)
    recover = commands.add_parser('restore')
    for option in ('plan', 'kind', 'name'):
        recover.add_argument('--' + option, required=True)
    args = parser.parse_args()
    if args.command == 'preview':
        require(args.retention_days >= 1, 'retention must be positive')
        output = Path(args.output)
        output.mkdir(mode=0o700, parents=True, exist_ok=False)
        plan = inventory(args.data_dir, args.binary_root, args.agents_root, args.config_path, args.retention_days)
        write(output / 'plan.json', plan)
        print(json.dumps({'dag_candidates': len(plan['dag']), 'binary_candidates': len(plan['binary']),
                          'plan': str(output / 'plan.json')}))
    elif args.command == 'apply':
        path = Path(args.plan)
        receipts = apply(json.loads(path.read_text()), path.parent, Client(args.base_url, args.token_file))
        print(json.dumps({'deleted': sum(r['status'] == 'deleted' for r in receipts),
                          'skipped': sum(r['status'] == 'skipped' for r in receipts),
                          'pending': sum(r['status'] == 'pending' for r in receipts)}))
        if any(r['status'] == 'pending' for r in receipts):
            raise SystemExit(2)
    else:
        path = Path(args.plan)
        receipts = json.loads((path.parent / 'receipts.json').read_text())
        receipt = next(r for r in receipts if r['kind'] == args.kind and r['name'] == args.name and r['status'] == 'deleted')
        result = restore(json.loads(path.read_text()), receipt)
        write(path.parent / 'last-restore.json', result)
        print(json.dumps(result))


if __name__ == '__main__':
    main()
