import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from rolling.config import Settings
from rolling.maintenance.engine import programs
from rolling.state import Journal
from rolling.units import service


class RetainedProgramTests(unittest.TestCase):
    def test_separate_program_preserves_identity_config_resources_and_old_image(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            settings = Settings(root / 'state', root / 'token', systemd_dir=root / 'units')
            settings.systemd_dir.mkdir()
            managed = settings.state_dir / 'runtimes/old'
            managed.mkdir(parents=True)
            alias = root / 'retained-runtimes'
            alias.symlink_to(managed.parent, target_is_directory=True)
            data = alias / 'old'
            image = data / 'dag/rootfs'
            (image / 'usr/bin').mkdir(parents=True)
            (image / 'workspace').mkdir()
            resource = image / 'usr/kept'
            resource.write_text('frozen resource')
            bundle = root / 'candidate'
            (bundle / 'bin').mkdir(parents=True)
            for name in ('dag-runner', 'agent-step-runner', 'agent-session-runner', 'opencoder', 'opencoder-cli'):
                (image / 'usr/bin' / name).write_text('old runner')
                (bundle / 'bin' / name).write_text('new runner')
            workdir = data / 'workdir'
            workdir.mkdir()
            original = {'dag': {'rootfs_dir': str(image), 'data_dir': str(data / 'saved-runs'),
                                'ops': {'legacy': 'retained'}, 'wasm_dir': str(data / 'old-wasm')},
                        'llm': {'model': 'retained-model'}}
            (workdir / 'opencoder.json').write_text(json.dumps(original))
            record = {'id': 'old', 'runtime_data': str(data), 'runtime_unit': 'old.service',
                      'manifest': {'release_id': 'old'}}
            unit = settings.systemd_dir / record['runtime_unit']
            unit.write_text(service(['/releases/old/bin/opencoder-agent', '--workdir', str(workdir),
                                     '--data-dir', str(data), 'runtime', '--port', '4001'], 'old', runtime=True))
            accepted_unit = unit.read_bytes()
            candidate = {'release_id': 'new', 'storage_engine': 'turso'}
            prepared = programs.stage_one(settings, bundle, candidate, record, image)
            self.assertEqual(Path(prepared['rootfs']).parent, managed / 'programs/new')
            self.assertEqual(unit.read_bytes(), accepted_unit)
            self.assertEqual((image / 'usr/bin/dag-runner').read_text(), 'old runner')
            self.assertEqual((Path(prepared['rootfs']) / 'usr/bin/dag-runner').read_text(), 'new runner')
            for name in ('opencoder', 'opencoder-cli'):
                self.assertEqual((image / 'usr/bin' / name).read_text(), 'old runner')
                self.assertEqual((Path(prepared['rootfs']) / 'usr/bin' / name).read_text(), 'new runner')
            self.assertEqual((Path(prepared['rootfs']) / 'usr/kept').stat().st_ino, resource.stat().st_ino)
            config = json.loads((Path(prepared['workdir']) / 'opencoder.json').read_text())
            self.assertEqual(config['llm'], original['llm'])
            self.assertEqual(config['dag']['data_dir'], original['dag']['data_dir'])
            self.assertNotIn('ops', config['dag'])
            self.assertNotIn('wasm_dir', config['dag'])
            journal = Journal(settings.state_dir)
            journal.data['releases']['old'] = record
            programs.install(settings, journal, {'old': prepared})
            programs.install(settings, journal, {'old': prepared})
            self.assertEqual(record['manifest'], {'release_id': 'old'})
            self.assertEqual(record['runtime_data'], str(data))
            self.assertIn(str(data), unit.read_text())
            self.assertIn('/releases/new/bundle/bin/opencoder-agent', unit.read_text())
            self.assertEqual(json.loads((workdir / 'opencoder.json').read_text()), original)


if __name__ == '__main__':
    unittest.main()
