import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from .apply import apply
from .model import digest


class ApplyTests(unittest.TestCase):
    def test_stale_items_are_skipped_and_conditional_deletes_resume(self):
        objects = {name: {'name': name, 'spec': {'name': name}} for name in ('changed', 'refused', 'removed')}
        items = [{'name': name, 'reason': 'idle', 'digest': digest(value)} for name, value in objects.items()]
        plan = dict(data_dir='state', binary_root='binaries', agents_root='agents', config_path='config',
                    retention_days=7, dag=items, binary=[])
        fresh = {**plan, 'dag': [item for item in items if item['name'] != 'changed']}
        deleted, attempts = set(), []

        class Client:
            def call(self, method, path, etag=None):
                if path == '/api/health':
                    return 200, {'catalog_guard': 2}, None
                name = path.rsplit('/', 1)[-1]
                if method == 'DELETE':
                    attempts.append((name, etag))
                    if name == 'refused':
                        return 412, {'error': 'changed'}, None
                    deleted.add(name)
                    return 200, {'ok': True}, None
                if name in deleted:
                    return 404, {}, None
                return 200, objects[name], '"current"'

        with tempfile.TemporaryDirectory() as directory, patch(
                'scripts.maintenance.catalog_cleanup.apply.inventory', return_value=fresh), patch(
                'scripts.maintenance.catalog_cleanup.apply.time.sleep') as yield_readers:
            receipts = apply(plan, directory, Client())
            self.assertEqual([r['status'] for r in receipts], ['skipped', 'skipped', 'deleted'])
            self.assertTrue(receipts[-1]['verified_missing'])
            backup = Path(receipts[-1]['backup']) / 'snapshot.json'
            self.assertEqual(json.loads(backup.read_text()), objects['removed'])
            apply(plan, directory, Client())
            self.assertEqual(attempts, [('refused', '"current"'), ('removed', '"current"'), ('refused', '"current"')])
            self.assertEqual(yield_readers.call_count, 3)
            self.assertTrue(all(call[0] == (0.2,) for call in yield_readers.call_args_list))

    def test_old_server_is_refused_before_reading_or_deleting_data(self):
        class Client:
            def call(self, *args, **kwargs):
                return 200, {'catalog_guard': 1}, None
        with tempfile.TemporaryDirectory() as directory, patch(
                'scripts.maintenance.catalog_cleanup.apply.inventory') as scan:
            with self.assertRaisesRegex(ValueError, 'catalog guard version 2'):
                apply({}, directory, Client())
            scan.assert_not_called()

    def test_missing_acknowledgement_recovers_from_the_persisted_delete_intent(self):
        value = {'name': 'workflow', 'spec': {'name': 'workflow'}}
        item = {'name': 'workflow', 'reason': 'idle', 'digest': digest(value)}
        plan = dict(data_dir='state', binary_root='binaries', agents_root='agents',
                    config_path='config', retention_days=7, dag=[item], binary=[])
        class Client:
            deleted = False
            def call(self, method, path, etag=None):
                if path == '/api/health':
                    return 200, {'catalog_guard': 2}, None
                if method == 'DELETE':
                    self.deleted = True
                    raise KeyboardInterrupt()
                return (404, {}, None) if self.deleted else (200, value, '"current"')
        client = Client()
        with tempfile.TemporaryDirectory() as directory, patch(
                'scripts.maintenance.catalog_cleanup.apply.inventory', return_value=plan) as scan:
            with self.assertRaises(KeyboardInterrupt):
                apply(plan, directory, client)
            self.assertEqual(json.loads((Path(directory) / 'receipts.json').read_text())[0]['status'], 'pending')
            scan.return_value = {**plan, 'dag': []}
            receipts = apply(plan, directory, client)
            self.assertEqual(len(receipts), 1)
            self.assertTrue(receipts[0]['recovered'])
            self.assertEqual(receipts[0]['status'], 'deleted')
