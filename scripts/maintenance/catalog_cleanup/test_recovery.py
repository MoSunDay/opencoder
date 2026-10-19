import hashlib
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
from .apply import apply
from .backup import create_backup, verify_receipt
from .client import write
from .model import digest
from .restore import restore


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.value = {'name': 'workflow', 'spec': {'name': 'workflow'}}
        self.plan = dict(data_dir=str(self.root), binary_root='unused', agents_root='unused',
            config_path='unused', retention_days=7, binary=[], dag=[dict(name='workflow',
            reason='idle', digest=digest(self.value))])
        scan = patch('scripts.maintenance.catalog_cleanup.apply.inventory', return_value=self.plan)
        self.scan = scan.start(); self.addCleanup(scan.stop)
        delay = patch('scripts.maintenance.catalog_cleanup.apply.time.sleep')
        delay.start(); self.addCleanup(delay.stop)
        self.exists = True
        self.deletes = 0
        self.readback_status = 200
        self.delete_error = TimeoutError('response lost after commit')

    def call(self, method, path, etag=None):
        if path == '/api/health':
            return 200, {'catalog_guard': 1}, None
        if method == 'DELETE':
            self.deletes += 1
            self.exists = False
            if self.delete_error:
                raise self.delete_error
            return 500, {'error': 'response failed'}, None
        if self.readback_status != 200:
            return self.readback_status, {}, None
        return (200, self.value, '"current"') if self.exists else (404, {}, None)

    def test_transport_errors_remain_pending_then_reconcile_and_restore(self):
        for error in (TimeoutError('timeout'), ConnectionError('disconnected'), ValueError('bad response')):
            with self.subTest(error=type(error).__name__):
                directory = self.root / type(error).__name__; directory.mkdir()
                self.exists = True; self.deletes = 0; self.delete_error = error
                first = apply(self.plan, directory, self)
                self.assertEqual(first[0]['status'], 'pending')
                backup = Path(first[0]['backup']) / 'snapshot.json'
                original = backup.read_bytes()
                self.scan.return_value = {**self.plan, 'dag': []}
                resumed = apply(self.plan, directory, self)
                self.assertEqual(len(resumed), 1)
                self.assertEqual(resumed[0]['status'], 'deleted')
                self.assertTrue(resumed[0]['verified_missing'])
                self.assertEqual(self.deletes, 1)
                self.assertEqual(backup.read_bytes(), original)
                (directory / 'control.locks').mkdir()
                with sqlite3.connect(directory / 'control.db') as db:
                    db.execute('CREATE TABLE fleet_definitions(kind TEXT,id TEXT,body TEXT,PRIMARY KEY(kind,id))')
                restore({'data_dir': str(directory)}, resumed[0])
                with sqlite3.connect(directory / 'control.db') as db:
                    row = db.execute("SELECT body FROM fleet_definitions WHERE kind='dag'").fetchone()
                self.assertEqual(json.loads(row[0]), self.value)
                self.scan.return_value = self.plan

    def test_5xx_acknowledgement_is_resolved_by_readback(self):
        self.delete_error = None
        result = apply(self.plan, self.root, self)
        self.assertEqual(result[0]['http_status'], 500)
        self.assertEqual(result[0]['status'], 'deleted')
        self.assertTrue(result[0]['verified_missing'])

    def test_unavailable_readback_does_not_repeat_delete(self):
        apply(self.plan, self.root, self)
        self.readback_status = 503
        for _ in range(2):
            result = apply(self.plan, self.root, self)
            self.assertEqual(len(result), 1)
            self.assertEqual(result[0]['status'], 'pending')
        self.assertEqual(self.deletes, 1)
        self.readback_status = 200
        self.assertEqual(apply(self.plan, self.root, self)[0]['status'], 'deleted')

    def test_legacy_skipped_intents_and_inventory_failure_preserve_recovery(self):
        result = apply(self.plan, self.root, self)
        result[0]['status'] = 'skipped'
        write(self.root / 'receipts.json', result)
        self.scan.side_effect = ValueError('unrelated resource is unreadable')
        with self.assertRaises(ValueError):
            apply(self.plan, self.root, self)
        saved = json.loads((self.root / 'receipts.json').read_text())
        self.assertEqual(saved[0]['status'], 'deleted')
        self.assertTrue(saved[0]['verified_missing'])

    def test_changed_backup_never_becomes_a_restore_anchor(self):
        result = apply(self.plan, self.root, self)
        (Path(result[0]['backup']) / 'snapshot.json').write_text(json.dumps({**self.value, 'changed': True}))
        resumed = apply(self.plan, self.root, self)
        self.assertEqual(resumed[0]['status'], 'pending')
        self.assertIn('digest mismatch', resumed[0]['error'])
        resumed[0]['status'] = 'deleted'
        with self.assertRaisesRegex(ValueError, 'digest mismatch'):
            restore(self.plan, resumed[0])

    def test_backup_interruption_does_not_publish_a_partial_anchor(self):
        pool = self.root / 'source' / 'tool'; (pool / 'v1').mkdir(parents=True)
        data = b'native test bytes'
        version = dict(version=1, sha256=hashlib.sha256(data).hexdigest(), size_bytes=len(data))
        meta = dict(name='tool', current=1, history=[1])
        snapshot = {**meta, 'history': [version]}
        (pool / 'meta.json').write_text(json.dumps(meta))
        (pool / 'v1/meta.json').write_text(json.dumps(version))
        (pool / 'v1/binary.bin').write_bytes(data)
        with patch('scripts.maintenance.catalog_cleanup.backup.shutil.copytree', side_effect=OSError('interrupted')):
            with self.assertRaises(OSError):
                create_backup(self.root, 'binary', 'tool', snapshot, pool.parent)
        self.assertEqual(list(self.root.glob('binary-*')), [])
        backup = create_backup(self.root, 'binary', 'tool', snapshot, pool.parent)
        receipt = dict(kind='binary', name='tool', backup=str(backup), digest=digest(snapshot))
        self.assertEqual(verify_receipt(receipt), snapshot)
        self.assertEqual(create_backup(self.root, 'binary', 'tool', snapshot, pool.parent), backup)

    def test_cli_reports_pending_and_returns_failure(self):
        from .__main__ import main
        plan = self.root / 'plan.json'; plan.write_text(json.dumps(self.plan))
        args = ['cleanup', 'apply', '--plan', str(plan), '--base-url', 'https://server.example', '--token-file', 'unused']
        with patch('sys.argv', args), patch('scripts.maintenance.catalog_cleanup.__main__.Client'), patch(
                'scripts.maintenance.catalog_cleanup.__main__.apply', return_value=[{'status': 'pending'}]), patch('sys.stdout', new_callable=io.StringIO) as output:
            with self.assertRaises(SystemExit) as error:
                main()
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(json.loads(output.getvalue())['pending'], 1)
