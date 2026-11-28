"""Each native connection names the side of the writer lock it takes."""
import json
from pathlib import Path
import sys
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.storage import connection

CLI = '/tools/opencoder-cli'
REPLY = json.dumps({'ok': True, 'autocommit': True, 'columns': [], 'rows': [], 'changes': 0,
                    'last_insert_rowid': 0, 'total_changes': 0}) + '\n'


def launched(directory, **options):
    """Open a marked Turso file through connect() with the CLI and its process replaced."""
    path = Path(directory) / 'store.db'
    Path(str(path) + '.engine.json').write_text(json.dumps({'engine': 'turso', 'format': 1}))
    process = MagicMock()
    process.stdout.readline.return_value = REPLY
    with patch.object(connection.database, 'executable', return_value=CLI), \
            patch.object(connection.subprocess, 'Popen', return_value=process) as popen:
        handle = connection.connect(path, **options)
    argv = popen.call_args.args[0]
    handle.close()
    return argv


def test_default_connection_explicitly_takes_the_writable_side(tmp_path):
    argv = launched(tmp_path)
    assert argv == [CLI, 'storage', 'serve', '--database', str(tmp_path / 'store.db'), '--writable']
    assert '--read-only' not in argv


def test_readonly_connection_keeps_the_read_only_side(tmp_path):
    argv = launched(tmp_path, readonly=True)
    assert argv == [CLI, 'storage', 'serve', '--database', str(tmp_path / 'store.db'), '--read-only']
    assert '--writable' not in argv


def test_both_sides_differ_only_in_the_mode_flag(tmp_path):
    writable, readonly = launched(tmp_path), launched(tmp_path, readonly=True)
    assert writable[:-1] == readonly[:-1]
    assert (writable[-1], readonly[-1]) == ('--writable', '--read-only')
