"""Crash boundaries and backup integrity, using only temporary paths."""
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling.config import Settings
from rolling import backup, deployment
from rolling.state import Journal
from test_deployment import Operations, PowerLoss, manifest


class Faults(Operations):
    def __init__(self, fail_at=None):
        super().__init__()
        self.fail_at = fail_at
        self.effects = 0

    def after_effect(self):
        self.effects += 1
        if self.effects == self.fail_at:
            self.fail_at = None
            raise PowerLoss()

    def http(self, *args, **kwargs):
        result = super().http(*args, **kwargs)
        self.after_effect()
        return result

    def run(self, *args):
        super().run(*args)
        self.after_effect()

    def ingress_workers(self):
        workers = super().ingress_workers()
        self.after_effect()
        return workers


class RecoveryTests(unittest.TestCase):
    def test_every_external_switch_boundary_can_resume_without_duplicate_release(self):
        def exercise(fault):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                settings = Settings(root,root/'work',root/'server',root/'agent',root/'token',
                    bin_dir=root/'bin',systemd_dir=root/'units')
                journal = Journal(root)
                journal.data.update(current='r1',phase='complete',releases={
                    'r1':deployment.record_for(settings,manifest('r1'),0)})
                journal.save()
                operations = Faults(fault)
                with patch('rolling.deployment.manifest.verify',return_value=manifest('r2')), \
                     patch('rolling.deployment.manifest.resources'), \
                     patch('rolling.deployment.units.prepare'), \
                     patch('rolling.deployment.units.switch_ingress'), \
                     patch('rolling.deployment.probes.candidate',return_value='node'), \
                     patch('rolling.deployment.probes.ready'), \
                     patch('rolling.deployment.probes.public'):
                    try:
                        deployment.deploy(settings,root/'bundle',operations)
                    except PowerLoss:
                        pass
                    effects = operations.effects
                    operations.fail_at = None
                    result = deployment.deploy(settings,root/'bundle',operations)
                self.assertEqual(result['current'],'r2')
                self.assertEqual(result['phase'],'complete')
                self.assertEqual(set(result['releases']),{'r1','r2'})
                self.assertFalse(any('/api/admin/drain' in c for c in operations.calls))
                return effects
        for fault in range(1, exercise(None) + 1):
            with self.subTest(effect=fault):
                exercise(fault)

    def test_stopped_backup_preserves_nested_databases_and_retry_never_reuses_partial(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            server = root/'server'
            nested = server/'nested/runtime.db'
            nested.parent.mkdir(parents=True)
            with sqlite3.connect(nested) as connection:
                connection.execute('CREATE TABLE evidence(value TEXT)')
                connection.execute("INSERT INTO evidence VALUES ('preserved')")
            settings = Settings(root/'state',root/'work',server,root/'agent',root/'token')
            output = root/'backups/recovery'
            with patch('rolling.backup.database',side_effect=OSError('interrupted copy')):
                with self.assertRaises(OSError):
                    backup.snapshot(settings,output,stopped=True)
            self.assertFalse(output.exists())
            self.assertEqual(len(list(output.parent.glob('.recovery.incomplete-*'))),1)
            backup.snapshot(settings,output,stopped=True)
            with sqlite3.connect(output/'server/nested/runtime.db') as connection:
                self.assertEqual(connection.execute('SELECT value FROM evidence').fetchall(),[('preserved',)])
            self.assertTrue((output/'backup-manifest.json').exists())
            with sqlite3.connect(nested) as connection:
                self.assertEqual(connection.execute('SELECT value FROM evidence').fetchall(),[('preserved',)])


if __name__ == '__main__':
    unittest.main()
