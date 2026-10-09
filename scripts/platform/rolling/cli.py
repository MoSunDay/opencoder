import argparse
import contextlib
import json
from pathlib import Path
from . import backup, config, deployment, migration, maintenance
from .maintenance.recovery import capacity
from .maintenance import services
from .io import Operations
from .state import Journal, locked
from signal_release import controller
from signal_release.trigger import trigger


def main():
    parser = argparse.ArgumentParser(description="Versioned deployment; existing tasks keep their owning runtime")
    parser.add_argument("--config", type=Path, default=Path("/etc/opencoder/server/opencoder.json"))
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--wait-seconds", type=int, default=90)
    parser.add_argument("--signal", action="store_true", help="ask the running Server to launch an independent release job")
    parser.add_argument("--maintenance", action="store_true", help="stop writers and back up before an incompatible upgrade")
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--migrate", action="store_true", help="execute first migration or a stopped maintenance upgrade")
    actions.add_argument("--migration-receipt", action="store_true", help="print the concrete migration scope without stopping services")
    actions.add_argument("--rollback", action="store_true")
    actions.add_argument("--status", action="store_true")
    actions.add_argument("--backup", type=Path, help="independent online database backups")
    actions.add_argument("--stage", action="store_true", help="verify and retain a candidate for a later SIGUSR2")
    actions.add_argument("--recover-capacity", action="store_true",
                         help="recover proven stale capacity reservations after a restart")
    args = parser.parse_args()
    if args.wait_seconds <= 0:
        parser.error("--wait-seconds must be positive")
    if args.signal and (args.migrate or args.migration_receipt or args.backup or args.status or args.stage or args.recover_capacity):
        parser.error("--signal supports deploy or --rollback only")
    if args.rollback and args.bundle:
        parser.error("--rollback does not accept --bundle")
    if args.maintenance and (args.migrate or args.migration_receipt or args.backup or args.status or args.rollback or args.recover_capacity):
        parser.error('--maintenance supports deploy or --stage only; --rollback detects maintenance recovery')
    settings = config.load(args.config)
    if args.status:
        print(json.dumps({**Journal(settings.state_dir).data, "signals": controller.status(settings)}, indent=2))
        return
    if args.migration_receipt:
        if not args.bundle:
            parser.error("--bundle is required")
        print(json.dumps(migration.receipt(settings, args.bundle), indent=2))
        return
    operations = Operations(settings.token_file)
    with controller.request_lock(settings) if args.signal or args.stage else contextlib.nullcontext():
        with locked(settings.state_dir):
            if args.recover_capacity:
                journal = Journal(settings.state_dir)
                current = journal.record(journal.data['current'])
                # A reboot can leave more than one versioned Host connected to
                # the same stable node. Stop every managed Host/Server first;
                # keeping an old connection alive makes the new Server reject
                # the current node as a duplicate owner.
                active_units = []
                for record in journal.data['releases'].values():
                    for key in ('server_unit', 'host_unit'):
                        unit = record[key]
                        if operations.output('systemctl', 'show', unit, '-p', 'LoadState', '--value').strip() == 'not-found':
                            continue
                        state = operations.output('systemctl', 'show', unit, '-p', 'ActiveState', '--value').strip()
                        if state in ('active', 'activating', 'deactivating'):
                            active_units.append((record['id'], key, unit))
                for release_id, key, unit in sorted(set(active_units)):
                    services.stop_unit(unit, operations, args.wait_seconds)
                    if release_id != current['id']:
                        operations.run('systemctl', 'disable', unit)
                result = capacity.recover_running(settings, operations, args.wait_seconds)
                for runtime in result.get('runtime_units', []):
                    operations.run('systemctl', 'start', runtime['unit'])
                    operations.wait(lambda endpoint=runtime['endpoint']: operations.http(endpoint, '/inventory'), args.wait_seconds)
                    operations.http(runtime['endpoint'], '/rpc', 'POST', {'operation': 'admission', 'command': 'reopen'})
                operations.run('systemctl', 'start', current['host_unit'])
                host_endpoint = f"http://127.0.0.1:{current['host_port']}"
                operations.wait(lambda: operations.http(host_endpoint, '/status'), args.wait_seconds)
                operations.run('systemctl', 'start', current['server_unit'])
                server_endpoint = f"http://127.0.0.1:{current['server_port']}"
                operations.wait(lambda: operations.http(server_endpoint, '/api/ready'), args.wait_seconds)
                result['services'] = {'runtime': [item['unit'] for item in result.get('runtime_units', [])],
                                      'host': current['host_unit'], 'server': current['server_unit']}
            elif args.backup:
                backup.snapshot(settings, args.backup)
                result = {"backup": str(args.backup), "cross_database_snapshot": False}
            elif args.rollback:
                if args.signal:
                    controller.install(settings, args.config, operations)
                result = None if args.signal else deployment.rollback(settings, operations, args.wait_seconds)
            else:
                if not args.bundle:
                    parser.error("--bundle is required")
                if args.stage or args.signal:
                    result = controller.stage(settings, args.bundle, maintenance=args.maintenance,
                                              wait_seconds=args.wait_seconds)
                    controller.install(settings, args.config, operations)
                else:
                    if not args.maintenance:
                        controller.install(settings, args.config, operations)
                    action = maintenance.deploy if args.maintenance else migration.migrate if args.migrate else deployment.deploy
                    result = action(settings, args.bundle, operations, args.wait_seconds)
                    if args.maintenance:
                        controller.install(settings, args.config, operations)
        if args.signal:
            target = result["release_id"] if result else None
            result = trigger(settings, "rollback" if args.rollback else "deploy", operations, args.wait_seconds)
            if target is not None and result["current"] != target:
                raise ValueError("signal job completed for a different staged release")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
