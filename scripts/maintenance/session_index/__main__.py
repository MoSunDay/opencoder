"""Preview, then apply using the saved database identity."""
import argparse
import json
import os
from pathlib import Path
import tempfile
from . import maintain


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path, required=True)
    parser.add_argument('--preview', type=Path, help='Apply the exact saved read-only preview')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--timeout-seconds', type=float, default=20)
    args = parser.parse_args()
    database = args.database.resolve(strict=True)
    destination = args.output.resolve()
    protected = [database, Path(str(database) + '-wal'), Path(str(database) + '-shm')]
    if any(destination == path or (destination.exists() and path.exists()
                                  and destination.samefile(path)) for path in protected):
        parser.error('receipt must not overwrite the database or its companions')
    expected = json.loads(args.preview.read_bytes()) if args.preview else None
    result = maintain(args.database, expected, args.timeout_seconds)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', dir=args.output.parent, delete=False) as receipt:
        json.dump(result, receipt, indent=2)
        receipt.write('\n')
        receipt.flush()
        os.fsync(receipt.fileno())
    Path(receipt.name).replace(args.output)
    print(json.dumps(result))


if __name__ == '__main__':
    main()
