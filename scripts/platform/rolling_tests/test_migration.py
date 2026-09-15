"""First migration replays switch intent without touching a running legacy node."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from rolling import migration
from rolling.config import Settings
from rolling.deployment import record_for
from rolling.state import Journal
from test_deployment import Operations, PowerLoss, manifest


class MigrationTests(unittest.TestCase):
    def test_service_stage_preserves_existing_resource_mounts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root,root/'work',root/'server',root/'agent',root/'token',
                nginx_include=root/'nginx.conf',systemd_dir=root/'units')
            settings.systemd_dir.mkdir()
            mount = settings.systemd_dir / 'mnt-opencoder-agents.mount'
            mount.write_text('[Mount]\nWhere=/mnt/opencoder-agents\n')
            bundle = root / 'bundle'
            (bundle/'bin').mkdir(parents=True)
            (bundle/'bin/opencoder-server').write_bytes(b'fixture resource binary')
            journal = Journal(root)
            record = record_for(settings,manifest('r1'),0)
            journal.data.update(candidate='r1',phase='migrating',migration_stage='services',
                releases={'r1':record})
            journal.save()
            operations = Operations()
            with patch('rolling.migration.receipt',return_value={'release_id':'r1','node_id':'node'}), \
                 patch('rolling.migration.manifest.verify',return_value=manifest('r1')), \
                 patch('rolling.migration.manifest.resources'), \
                 patch('rolling.migration.shutil.chown'), \
                 patch('rolling.migration.units.prepare'), \
                 patch('rolling.migration.units.validate'), \
                 patch('rolling.migration.probes.candidate'), \
                 patch('rolling.migration.probes.ready') as ready, \
                 patch('rolling.migration.probes.public'):
                result = migration.migrate(settings,bundle,operations)
            self.assertEqual(result['migration_stage'],'complete')
            self.assertTrue(result['migration_mounts_ready'])
            self.assertIn(('systemctl','start',mount.name),operations.calls)
            self.assertFalse(any(c[:2] in [('systemctl','restart'),('systemctl','stop')]
                and mount.name in c for c in operations.calls))
            ready.assert_called_once()

    def test_resume_after_current_pointer_was_written_finishes_ingress(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root,root/'work',root/'server',root/'agent',root/'token',
                nginx_include=root/'nginx.conf',systemd_dir=root/'units')
            journal = Journal(root)
            record = record_for(settings,manifest('r1'),0)
            journal.data.update(current='r1',candidate=None,phase='switching',migration_stage='switching',
                releases={'r1':record})
            journal.save()
            operations = Operations()
            with patch('rolling.migration.receipt',return_value={'release_id':'r1','node_id':'node'}), \
                 patch('rolling.migration.manifest.verify',return_value=manifest('r1')), \
                 patch('rolling.migration.manifest.resources'), \
                 patch('rolling.migration.probes.public'):
                result = migration.migrate(settings,root/'bundle',operations)
            self.assertEqual(result['phase'],'complete')
            self.assertEqual(result['migration_stage'],'complete')
            self.assertIn('proxy_pass http://127.0.0.1:3000',settings.nginx_include.read_text())
            self.assertFalse(any(c[:2] == ('systemctl','stop') for c in operations.calls))
            self.assertFalse(any('/api/admin/drain' in c for c in operations.calls))


if __name__ == '__main__':
    unittest.main()
