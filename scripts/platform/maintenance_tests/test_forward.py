"""Post-write corrective staging and deployment must keep live data and the sealed backup."""
import copy
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import Fixture, bundle_manifest
from rolling import deployment
from rolling.maintenance import archive, flow
from rolling.maintenance.recovery import forward
from rolling.state import Journal
from signal_release import controller, runner


class PowerLoss(BaseException):
    pass


class ForwardTests(unittest.TestCase):
    def test_alias_of_installed_commit_is_rejected_before_staging(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            self.opened_failure(fixture)
            candidate = bundle_manifest('fix', 2)
            candidate['commit'] = fixture.candidate['commit']
            journal = Journal(fixture.settings.state_dir)
            before = journal.path.read_bytes()
            with patch.object(controller.manifest, 'verify', return_value=candidate), \
                    patch.object(controller.manifest._installer, 'stage_bundle') as stage:
                with self.assertRaisesRegex(ValueError, 'different compiled commit'):
                    controller.stage(fixture.settings, fixture.bundle, maintenance=True)
                stage.assert_not_called()
            self.assertEqual(journal.path.read_bytes(), before)
            self.assertFalse((fixture.settings.state_dir / 'signal-pending.json').exists())

    def opened_failure(self, fixture):
        with fixture.patches(), patch.object(flow.probes, 'public', side_effect=RuntimeError('public failed')):
            with self.assertRaisesRegex(RuntimeError, 'public failed'):
                flow.deploy(fixture.settings, fixture.bundle, fixture)
        journal = Journal(fixture.settings.state_dir)
        self.assertTrue(journal.data['maintenance']['writes_open'])
        with sqlite3.connect(fixture.db) as conn:
            conn.execute("INSERT INTO project_todos VALUES ('new-write','i')")
        return journal.data['maintenance']['backup']

    def test_signal_corrective_release_keeps_new_writes_auth_and_original_backup(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            backup = Path(self.opened_failure(fixture))
            fingerprint = archive.inventory(backup)
            fixture.candidate = bundle_manifest('fix', 2)
            fixture.ingress_workers = lambda: []
            with fixture.patches(), patch.object(deployment.probes, 'ready'), \
                    patch.object(deployment, 'retire_server'), \
                    patch.object(controller.manifest._installer, 'stage_bundle', return_value=fixture.bundle):
                intent = controller.stage(fixture.settings, fixture.bundle, maintenance=True)
                self.assertEqual(intent['release_id'], 'fix')
                result = runner.run(fixture.settings, 'deploy--new', fixture)
                self.assertEqual(result['phase'], 'complete')
            journal = Journal(fixture.settings.state_dir).data
            self.assertEqual(journal['current'], 'fix')
            self.assertEqual(journal['maintenance']['target'], 'new')
            self.assertEqual(journal['maintenance']['repair_origin'], 'new')
            self.assertEqual(journal['maintenance']['backup'], str(backup))
            self.assertEqual(journal['maintenance']['stage'], 'complete')
            self.assertTrue(journal['releases']['old']['maintenance_retired'])
            self.assertEqual(archive.inventory(backup), fingerprint)
            with sqlite3.connect(fixture.db) as conn:
                self.assertEqual(conn.execute("SELECT initiative_id FROM project_todos WHERE id='new-write'").fetchone(), ('i',))
                self.assertEqual(conn.execute('SELECT token_hash FROM platform_users').fetchone(), (b'\x00\x01\x02',))
            with self.assertRaisesRegex(ValueError, 'restoration is forbidden'):
                flow.rollback(fixture.settings, fixture)

    def test_handoff_survives_crash_and_signal_retries_after_pointer_change(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            backup = self.opened_failure(fixture)
            fixture.candidate = bundle_manifest('fix', 2)
            with fixture.patches(), patch.object(deployment, 'deploy', side_effect=PowerLoss):
                with self.assertRaises(PowerLoss):
                    flow.deploy(fixture.settings, fixture.bundle, fixture)
            data = Journal(fixture.settings.state_dir).data
            self.assertTrue(forward.permitted(data, 'fix'))
            self.assertEqual(data['maintenance']['backup'], backup)
            data['releases']['fix'] = copy.deepcopy(data['releases']['new'])
            data['releases']['fix']['manifest'] = fixture.candidate
            data.update(current='fix', candidate=None, phase='complete')
            journal = Journal(fixture.settings.state_dir); journal.data = data; journal.save()
            from rolling.state import write
            write(fixture.settings.state_dir / 'signal-pending.json', {'release_id': 'fix', 'bundle': str(fixture.bundle)})
            self.assertEqual(runner.target_for(fixture.settings, 'deploy', 'new', data), ('fix', fixture.bundle))
            forward.validate(data, fixture.candidate)

    def test_unopened_and_incompatible_candidates_cannot_handoff_or_stage(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = Fixture(Path(directory))
            self.opened_failure(fixture)
            journal = Journal(fixture.settings.state_dir)
            for writes, version in [(False, 2), (True, 1), (True, 3)]:
                with self.subTest(writes=writes, version=version):
                    journal.data['maintenance']['writes_open'] = writes
                    journal.save()
                    before = journal.path.read_bytes()
                    candidate = bundle_manifest('fix', version)
                    with patch.object(controller.manifest, 'verify', return_value=candidate):
                        with self.assertRaises(ValueError):
                            controller.stage(fixture.settings, fixture.bundle, maintenance=True)
                    self.assertEqual(journal.path.read_bytes(), before)
                    self.assertFalse((fixture.settings.state_dir / 'signal-pending.json').exists())
