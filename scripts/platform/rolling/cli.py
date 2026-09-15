import argparse
import json
from pathlib import Path
from . import backup, config, deployment, migration
from .io import Operations
from .state import Journal, locked


def main():
    parser = argparse.ArgumentParser(description="Versioned deployment; existing tasks keep their owning runtime")
    parser.add_argument("--config", type=Path, default=Path("/etc/opencoder/server/opencoder.json"))
    parser.add_argument("--bundle", type=Path)
    parser.add_argument("--wait-seconds", type=int, default=90)
    actions = parser.add_mutually_exclusive_group()
    actions.add_argument("--migrate", action="store_true", help="execute the one-time safe migration window")
    actions.add_argument("--migration-receipt", action="store_true", help="print the concrete migration scope without stopping services")
    actions.add_argument("--rollback", action="store_true")
    actions.add_argument("--status", action="store_true")
    actions.add_argument("--backup", type=Path, help="independent online database backups")
    args = parser.parse_args()
    if args.wait_seconds <= 0:
        parser.error("--wait-seconds must be positive")
    settings = config.load(args.config)
    if args.status:
        print(json.dumps(Journal(settings.state_dir).data, indent=2))
        return
    if args.migration_receipt:
        if not args.bundle:
            parser.error("--bundle is required")
        print(json.dumps(migration.receipt(settings, args.bundle), indent=2))
        return
    operations = Operations(settings.token_file)
    with locked(settings.state_dir):
        if args.backup:
            backup.snapshot(settings, args.backup)
            result = {"backup": str(args.backup), "cross_database_snapshot": False}
        elif args.rollback:
            result = deployment.rollback(settings, operations, args.wait_seconds)
        else:
            if not args.bundle:
                parser.error("--bundle is required")
            action = migration.migrate if args.migrate else deployment.deploy
            result = action(settings, args.bundle, operations, args.wait_seconds)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
