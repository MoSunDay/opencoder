"""Independent maintenance controller, including an actual checkpoint exit."""
import argparse
import os
from pathlib import Path
import sys
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'platform'))
from rolling.config import load
from rolling.io import HttpFailure
from rolling.maintenance import flow
from rolling.state import write
from control import RemoteOperations
from fixture import configuration_scope


def closed_gates(settings, operations):
    results = []
    for base, path, method, body in (
            (settings.public_url, '/api/project/goals', 'POST', {'title': 'gate-must-refuse'}),
            (settings.host_url, '/runtimes', 'POST', {})):
        try:
            operations.http(base, path, method, body)
        except HttpFailure as error:
            if error.code != 503:
                raise
            results.append({'url': base + path, 'status': error.code})
        else:
            raise AssertionError('maintenance write gate is open')
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--socket', type=Path, required=True)
    parser.add_argument('--fault', choices=('installing', 'verification', 'none'), required=True)
    args = parser.parse_args()
    settings = load(args.config)
    root = args.config.parent
    operations = RemoteOperations(settings.token_file, args.socket)
    checkpoint = flow.checkpoint
    internal = flow.services.internal
    def crash(journal, stage):
        checkpoint(journal, stage)
        if stage == 'installing' and args.fault == 'installing':
            write(root / 'installing-crash.json', {'stage': stage,
                  'schema_started': journal.data['maintenance'].get('schema_started', False), 'pid': os.getpid()})
            os._exit(86)
    def reject(current, record, effects, seconds):
        internal(current, record, effects, seconds)
        if args.fault == 'verification':
            gates = closed_gates(current, effects)
            write(root / 'verification-injection.json', {'stage': 'verifying', 'gates': gates, 'pid': os.getpid()})
            raise RuntimeError('injected verification failure after real schema migration')
    with configuration_scope(settings, root), \
            patch.object(flow, 'checkpoint', side_effect=crash), \
            patch.object(flow.services, 'internal', side_effect=reject):
        result = flow.deploy(settings, args.bundle, operations, 120)
    write(root / ('controller-' + args.fault + '.json'), {'phase': result['phase'],
          'stage': result['maintenance']['stage'], 'pid': os.getpid()})


if __name__ == '__main__':
    main()
