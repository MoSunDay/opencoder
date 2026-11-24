"""Rebuild a damaged Turso opencoder database from a copy; the source is never written."""
import argparse
import json
from pathlib import Path
import sys

from .repair import repair


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--database', type=Path, required=True,
                        help='path to opencoder.db (copied to a scratch directory, never written)')
    parser.add_argument('--output', type=Path, required=True, help='path of the rebuilt database')
    parser.add_argument('--sqlite3', default='sqlite3', help='sqlite3 shell binary')
    parser.add_argument('--dump', type=Path, help='reuse a .dump taken from the same database state')
    parser.add_argument('--overwrite', action='store_true', help='replace an existing output file')
    parser.add_argument('--keep-workdir', action='store_true', help='keep the scratch copy and dump')
    parser.add_argument('--json', type=Path, help='also write the report to this file')
    args = parser.parse_args()
    report = repair(args.database, args.output, sqlite3_bin=args.sqlite3, dump=args.dump,
                    overwrite=args.overwrite, keep_workdir=args.keep_workdir,
                    log=lambda line: print(line, file=sys.stderr, flush=True))
    text = json.dumps(report, ensure_ascii=False, indent=2, default=str)
    if args.json:
        args.json.write_text(text)
    print(text)


if __name__ == '__main__':
    main()
