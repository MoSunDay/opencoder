"""Locate only this batch's executions on their actual locally hosted Runtime."""
import json
from pathlib import Path

from environment import require


def runtime_directories(proc=Path('/proc')):
    directories = set()
    for process in proc.iterdir():
        if not process.name.isdigit():
            continue
        try:
            if process.joinpath('exe').resolve().name != 'opencoder-agent':
                continue
            args = process.joinpath('cmdline').read_bytes().decode().split('\0')
            if '--data-dir' in args:
                directory = Path(args[args.index('--data-dir') + 1])
                if directory.is_absolute():
                    directories.add(directory)
        except (OSError, ValueError, IndexError, UnicodeError):
            continue
    return directories


def owned_directory(operation, owner, directories):
    identifier, kind = operation['execution_id'], operation['execution_kind']
    require(all(value and '/' not in value and value not in ('.', '..')
                for value in (identifier, kind)), 'invalid owned execution path')
    matches = set()
    for directory in directories:
        directory = directory.resolve()
        journal = directory / kind / identifier / 'execution.json'
        try:
            if (directory / 'node-id').read_text().strip() != owner:
                continue
            record = json.loads(journal.read_text())
        except (OSError, ValueError):
            continue
        assignment = record['assignment']
        index, request = assignment['index'], assignment['request']
        receipt = request['input'].get('brain_layered', {})
        require(index['id'] == identifier and index['kind'] == kind and
                index['node_id'] == owner and request['id'] == identifier and
                receipt.get('run_id') == operation['run_id'] and
                receipt.get('operation_id') == operation['operation_id'],
                'execution journal does not belong to this batch operation')
        require(journal.resolve().is_relative_to(directory), 'execution journal is outside Runtime')
        matches.add(directory)
    require(len(matches) == 1, 'owning Runtime journal must be uniquely accessible on this machine')
    return matches.pop()
