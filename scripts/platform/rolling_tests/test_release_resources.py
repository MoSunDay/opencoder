"""Port conflicts and concurrent backup writes use isolated local resources."""
from contextlib import closing
from pathlib import Path
import socket
import sqlite3
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import backup, deployment
from rolling.network.ports import available, first_available


class PortTests(unittest.TestCase):
    def test_skips_a_real_listener_without_sending_protocol_bytes(self):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            port = listener.getsockname()[1]
            self.assertFalse(available(port, 1))
            selected = first_available(port, 2)
            self.assertGreater(selected, port)
            listener.settimeout(.02)
            with self.assertRaises(TimeoutError):
                listener.accept()

    def test_selection_checks_whole_blocks_and_exhaustion(self):
        busy = {3306, 3308}
        probe = lambda port, count: not busy.intersection(range(port, port + count))
        self.assertEqual(first_available(3305, 2, probe), 3309)
        self.assertEqual(first_available(65535, 1, lambda *_: True), 65535)
        with self.assertRaisesRegex(ValueError, 'no ports'):
            first_available(65535, 2, lambda *_: True)
        with self.assertRaisesRegex(ValueError, 'no ports'):
            first_available(65534, 2, lambda *_: False)

    def test_new_release_and_reactivation_use_available_blocks(self):
        settings = SimpleNamespace(port_base=3000, state_dir=Path('/unused'))
        with patch('rolling.deployment.first_available', return_value=3500) as pick:
            original = deployment.record_for(settings, {'release_id': 'r1'}, 2)
            pick.assert_called_once_with(3006, 3)
        with patch('rolling.deployment.first_available', return_value=3600) as pick:
            result = deployment.fresh_frontends(original, [original], 'rollback')
            pick.assert_called_once_with(3503, 2)
        self.assertEqual((result['host_port'], result['server_port']), (3600, 3601))
        self.assertEqual(result['runtime_port'], original['runtime_port'])
        self.assertEqual(result['runtime_unit'], original['runtime_unit'])
        self.assertEqual(original['host_port'], 3502)


class BackupTests(unittest.TestCase):
    def test_snapshot_is_pinned_before_concurrent_commit_and_connections_close(self):
        with tempfile.TemporaryDirectory() as directory:
            source, target = Path(directory) / 'source.db', Path(directory) / 'backup.db'
            connect = sqlite3.connect
            with closing(connect(source)) as writer:
                writer.execute('PRAGMA journal_mode=WAL')
                writer.execute('CREATE TABLE versions(id INTEGER PRIMARY KEY, version INTEGER)')
                writer.executemany('INSERT INTO versions VALUES(?,0)', [(i,) for i in range(1000)])
                writer.commit()
            opened = []

            class Reader(sqlite3.Connection):
                def backup(self, output, **kwargs):
                    with closing(connect(source)) as writer:
                        writer.execute('UPDATE versions SET version=1')
                        writer.commit()
                    return super().backup(output, **kwargs)

            def tracked(path, **kwargs):
                connection = connect(path, factory=Reader, **kwargs) if kwargs.get('uri') else connect(path, **kwargs)
                opened.append(connection)
                return connection

            with patch('rolling.backup.sqlite3.connect', side_effect=tracked):
                backup.database(source, target)
            with closing(connect(target)) as result:
                self.assertEqual(result.execute('SELECT count(*),min(version),max(version) FROM versions').fetchone(), (1000, 0, 0))
            with closing(connect(source)) as result:
                self.assertEqual(result.execute('SELECT min(version) FROM versions').fetchone(), (1,))
            for connection in opened:
                with self.assertRaises(sqlite3.ProgrammingError):
                    connection.execute('SELECT 1')
            with self.assertRaisesRegex(ValueError, 'already exists'):
                backup.database(source, target)


if __name__ == '__main__':
    unittest.main()
