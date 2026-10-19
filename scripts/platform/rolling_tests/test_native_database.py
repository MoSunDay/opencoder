"""Exercise the real CLI, transaction adapter and online snapshot protocol."""
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.storage import database
from rolling.storage.connection import connect


@unittest.skipUnless(os.environ.get('OPENCODER_STORAGE_CLI'), 'set the compiled native CLI path')
class NativeDatabaseTests(unittest.TestCase):
    def test_transaction_error_snapshot_blob_and_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / 'source.db'
            # A stopped legacy file is the input to this native engine test.
            with sqlite3.connect(source) as seed:
                seed.execute('CREATE TABLE items(id INTEGER PRIMARY KEY, value BLOB)')
                seed.execute("INSERT INTO items VALUES(1, X'0102')")
            seed.close()
            Path(str(source) + '.engine.json').write_text(json.dumps({'engine': 'turso', 'format': 1}))
            writer = connect(source)
            reader = connect(source, readonly=True)
            try:
                reader.row_factory = sqlite3.Row
                reader.execute('BEGIN')
                row = reader.execute('SELECT id,value FROM items').fetchone()
                self.assertEqual(dict(row), {'id': 1, 'value': b'\x01\x02'})
                with writer:
                    writer.execute('INSERT INTO items VALUES (?,?)', (2, b'\x03\x04'))
                self.assertEqual(reader.execute('SELECT count(*) FROM items').fetchone()[0], 1)
                reader.rollback()
                self.assertEqual(reader.execute('SELECT count(*) FROM items').fetchone()[0], 2)
                with self.assertRaises(sqlite3.IntegrityError), writer:
                    writer.execute('INSERT INTO items VALUES (?,?)', (3, b'\x05'))
                    writer.execute('INSERT INTO items VALUES (?,?)', (1, b'\x06'))
                self.assertEqual(writer.execute('SELECT count(*) FROM items').fetchone(), (2,))
                with self.assertRaises(sqlite3.OperationalError):
                    reader.execute('DELETE FROM items')
                parent = Path(directory) / ("quoted'directory" if sys.platform == 'linux' else 'copies')
                parent.mkdir()
                target = parent / "quoted'copy.db"
                database.native('backup', '--database', source, '--output', target)
                with writer:
                    writer.execute('DELETE FROM items WHERE id=2')
                saved = sqlite3.connect(target.as_uri() + '?mode=ro&immutable=1', uri=True)
                try:
                    self.assertEqual(saved.execute('SELECT count(*) FROM items').fetchone(), (2,))
                finally:
                    saved.close()
            finally:
                reader.close()
                writer.close()
            database.native('checkpoint', '--database', source, '--writers-stopped')


if __name__ == '__main__':
    unittest.main()
