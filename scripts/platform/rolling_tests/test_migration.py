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
