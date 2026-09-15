import argparse
import contextlib
import json
from pathlib import Path
from . import backup, config, deployment, migration
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
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--migrate", action="store_true", help="execute the one-time safe migration window")
    actions.add_argument("--migration-receipt", action="store_true", help="print the concrete migration scope without stopping services")
    actions.add_argument("--rollback", action="store_true")
    actions.add_argument("--status", action="store_true")
    actions.add_argument("--backup", type=Path, help="independent online database backups")
    actions.add_argument("--stage", action="store_true", help="verify and retain a candidate for a later SIGUSR2")
    args = parser.parse_args()
    if args.wait_seconds <= 0:
        parser.error("--wait-seconds must be positive")
    if args.signal and (args.migrate or args.migration_receipt or args.backup or args.status or args.stage):
        parser.error("--signal supports deploy or --rollback only")
    if args.rollback and args.bundle:
        parser.error("--rollback does not accept --bundle")
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
            if args.backup:
                backup.snapshot(settings, args.backup)
                result = {"backup": str(args.backup), "cross_database_snapshot": False}
            elif args.rollback:
                controller.install(settings, args.config, operations)
                result = None if args.signal else deployment.rollback(settings, operations, args.wait_seconds)
            else:
                if not args.bundle:
                    parser.error("--bundle is required")
                controller.install(settings, args.config, operations)
                if args.stage or args.signal:
                    result = controller.stage(settings, args.bundle)
                else:
                    action = migration.migrate if args.migrate else deployment.deploy
                    result = action(settings, args.bundle, operations, args.wait_seconds)
        if args.signal:
            target = result["release_id"] if result else None
            result = trigger(settings, "rollback" if args.rollback else "deploy", operations, args.wait_seconds)
            if target is not None and result["current"] != target:
                raise ValueError("signal job completed for a different staged release")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
