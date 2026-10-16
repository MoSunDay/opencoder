from pathlib import Path
import json
import sys
import tempfile
from types import SimpleNamespace
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from rolling import backup, units
from rolling.storage import todos
from rolling.maintenance import restore, preflight


class TodoFilesTests(unittest.TestCase):
    def test_custom_export_backup_and_restore_preserves_file_bytes_and_binding(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            data, files = root/'server', root/'files'
            data.mkdir(); files.mkdir(); (root/'files.state').mkdir()
            (data/'todo-storage.json').write_text(json.dumps({'files_root':str(files)}))
            directory = files/'2026-01-01'/'pt-test'; directory.mkdir(parents=True)
            (directory/'note.bin').write_bytes(b'\x00\xfforiginal')
            settings = SimpleNamespace(server_data=data, state_dir=root/'state', legacy_agent_data=None)
            for stopped in [False, True]:
                target = root/('stopped' if stopped else 'online')
                backup.snapshot(settings,target,stopped=stopped)
                self.assertEqual((target/'todo-files/2026-01-01/pt-test/note.bin').read_bytes(), b'\x00\xfforiginal')
                manifest=json.loads((target/'backup-manifest.json').read_text())
                self.assertEqual(manifest['todo_files_root'], str(files))
                self.assertTrue((target/'server/todo-storage.json').exists())
            (directory/'note.bin').write_bytes(b'changed')
            container=root/'restore'; container.mkdir(); (root/'stopped').rename(container/'data')
            restore.data(settings,container,{'data_owners':{}})
            self.assertEqual((directory/'note.bin').read_bytes(),b'\x00\xfforiginal')
            self.assertEqual(todos.files_root(data),files)

    def test_root_rebinding_overlaps_and_writable_exports_fail(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp); data=root/'data'; data.mkdir()
            files=root/'files'
            (data/'todo-storage.json').write_text(json.dumps({'files_root':str(files)}))
            config={'project':{'todo_storage':{'files_dir':str(files),'nfs':{'enabled':True}}}}
            todos.validate(config,data)
            self.assertEqual(todos.port(config),2053)
            config['project']['todo_storage']['files_dir']=str(root/'other')
            with self.assertRaisesRegex(ValueError,'bound'): todos.validate(config,data)
            config['project']['todo_storage']['files_dir']=str(files)
            config['ontology']={'files_dir':str(files/'nested')}
            with self.assertRaisesRegex(ValueError,'overlap'): todos.validate(config,data)
            config['project']['todo_storage']['nfs']['read_only']=False
            with self.assertRaisesRegex(ValueError,'always read-only'): todos.port(config)

    def test_fifth_port_conflicts_and_nginx_upload_scope(self):
        agent={'agent':{'agents_dir':'/mount/a'},'dag':{'binary_dir':'/mount/b','workspace_dir':'/mount/w','rootfs_dir':'/image'}}
        server={'agent':{'agents_dir':'/exports/a','nfs':{'enabled':True}},'dag':{'binary_dir':'/exports/b','workspace_dir':'/source',
            'nfs':{'enabled':True},'workspace_nfs':{'enabled':True}},'project':{'todo_storage':{'nfs':{'enabled':True}}}}
        preflight.configuration(agent,server)
        server['project']['todo_storage']['nfs']['port']=2050
        with self.assertRaisesRegex(ValueError,'distinct'): preflight.configuration(agent,server)
        rendered=units.nginx(SimpleNamespace(listen='127.0.0.1:8080',host_port=8081),8082,8083)
        self.assertIn('^/api/project/todos/[^/]+/file$',rendered)
        self.assertIn('client_max_body_size 0;',rendered)
        self.assertIn('proxy_request_buffering off;',rendered)

if __name__=='__main__': unittest.main()
