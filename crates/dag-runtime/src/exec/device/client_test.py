"""Isolated HTTP contract checks: no device API or business submissions."""
import importlib.util
import json
from pathlib import Path
import threading
import tempfile
import unittest
from unittest.mock import patch
from http.server import BaseHTTPRequestHandler, HTTPServer

spec = importlib.util.spec_from_file_location('device_client', Path(__file__).with_name('client.py'))
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)

class TransportTest(unittest.TestCase):
    def test_ui_reservation_carries_frozen_specs_without_private_capability(self):
        seen=[]
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                seen.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                self.send_response(200);self.end_headers()
                self.wfile.write(json.dumps({'reservation_id':'a'*32,'assignments':[
                    {'instance_id':'0','machine':'win-02','generation':4,'capability':'never-export'}]}).encode())
            def log_message(self,*args):pass
        server=HTTPServer(('127.0.0.1',0),Handler)
        thread=threading.Thread(target=server.serve_forever);thread.start()
        try:
            case={'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}
            result=client.reserve({'endpoint':f'http://127.0.0.1:{server.server_port}',
                'capability':'private','identity':{'dag_id':'dag-ui'},'work_type':'ui',
                'input':{'device_count':1,'cases':[case]}})
            self.assertEqual(seen[0]['case_specs'],[case])
            self.assertEqual(seen[0]['case_ids'],['BITS-1-a'])
            self.assertEqual(json.loads(result['items'][0])['cases'],[case])
            self.assertNotIn('never-export',json.dumps(result))
        finally:
            server.shutdown();thread.join();server.server_close()

    def test_ui_identity_is_host_bound_and_registration_uses_frozen_hash(self):
        transport={'endpoint':'http://127.0.0.1:1','capability':'private',
            'work_type':'ui','identity':{'dag_id':'dag-ui','step_id':'execute','session_id':'host'},
            'assignment':{'reservation_id':'a'*32,'instance_id':'0','generation':4,'machine':'win-02',
                          'cases':[{'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}]}}
        case, run_id, common=client.ui_identity(transport,'BITS-1-a')
        self.assertEqual(run_id,'ui-'+__import__('hashlib').sha256(
            json.dumps(['a'*32,'0','BITS-1-a'],separators=(',',':')).encode()).hexdigest()[:40])
        self.assertEqual(common['generation'],4)
        with self.assertRaises(ValueError):
            client.ui_identity(transport,'foreign')
        with patch.object(client,'post',return_value={'ui_run_id':run_id,'case_id':'BITS-1-a'}) as send:
            client.ui_register(transport,'BITS-1-a')
            self.assertEqual(send.call_args.args[2]['spec_sha256'],'b'*64)
            self.assertEqual(send.call_args.args[2]['session_id'],'host')

    def test_ui_completion_uses_scoped_work_and_requires_verified_restore(self):
        transport={'endpoint':'http://127.0.0.1:1','capability':'private',
            'work_type':'ui','identity':{'dag_id':'dag-ui','step_id':'execute','session_id':'host'},
            'assignment':{'reservation_id':'a'*32,'instance_id':'0','generation':4,
                          'cases':[{'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}]}}
        _,run_id,_=client.ui_identity(transport,'BITS-1-a')
        with tempfile.TemporaryDirectory() as root:
            receipt=Path(root)/'receipts.json';receipt.write_text(json.dumps({'owner':'c'*64}))
            answer={'ui_run_id':run_id,'case_id':'BITS-1-a','recovery_verified':True,
                    'evidence_complete':True,'case_status':'failed'}
            with patch.object(client,'post',return_value=answer) as send:
                self.assertEqual(client.ui_complete(transport,'BITS-1-a',receipt)['case_status'],'failed')
                self.assertEqual(send.call_args.args[2]['receipts'],{'owner':'c'*64})
                self.assertEqual(send.call_args.args[2]['generation'],4)
            with patch.object(client,'post',return_value={**answer,'recovery_verified':False}):
                with self.assertRaisesRegex(ValueError,'authority mismatch'):
                    client.ui_complete(transport,'BITS-1-a',receipt)

    def test_ui_screenshot_returns_only_scoped_evidence_reference(self):
        transport={'endpoint':'http://127.0.0.1:1','capability':'private',
            'work_type':'ui','identity':{'dag_id':'dag-ui','step_id':'execute','session_id':'host'},
            'assignment':{'reservation_id':'a'*32,'instance_id':'0','generation':4,
                          'cases':[{'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}]}}
        result={'status':'done','result':{'file':'screenshots/op-shot.png','sha256':'f'*64,'bytes':30}}
        with patch.object(client,'post',return_value=result) as send:
            self.assertEqual(client.ui_screenshot(transport,'BITS-1-a','op-shot')['sha256'],'f'*64)
            self.assertTrue(send.call_args.args[1].endswith('/ui-screenshots'))
            self.assertEqual(send.call_args.args[2]['action'],'screenshot')

    def test_ui_capture_is_bound_to_case_and_has_bounded_stop_timeout(self):
        transport={'endpoint':'http://127.0.0.1:1','capability':'private',
            'work_type':'ui','identity':{'dag_id':'dag-ui','step_id':'execute','session_id':'host'},
            'assignment':{'reservation_id':'a'*32,'instance_id':'0','generation':4,
                          'cases':[{'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}]}}
        with patch.object(client,'post',return_value={'status':'done','action':'capture-stop','result':{'state':'stopped'}}) as send:
            self.assertEqual(client.ui_capture(transport,'BITS-1-a','op-stop','stop')['state'],'stopped')
            self.assertEqual(send.call_args.kwargs['timeout'],1900)
            self.assertEqual(send.call_args.args[2]['case_id'],'BITS-1-a')
        with self.assertRaises(ValueError):
            client.ui_capture(transport,'BITS-1-a','op-bad','restart')

    def test_ui_reconcile_only_accepts_owned_evidence_actions(self):
        transport={'endpoint':'http://127.0.0.1:1','capability':'private',
            'work_type':'ui','identity':{'dag_id':'dag-ui','step_id':'execute','session_id':'host'},
            'assignment':{'reservation_id':'a'*32,'instance_id':'0','generation':4,
                          'cases':[{'case_id':'BITS-1-a','spec_sha256':'b'*64,'input_version':'v1'}]}}
        with patch.object(client,'post',return_value={'status':'done','action':'screenshot','result':{'sha256':'f'*64}}) as send:
            self.assertEqual(client.ui_reconcile(transport,'BITS-1-a','op-shot','screenshot')['sha256'],'f'*64)
            self.assertTrue(send.call_args.args[1].endswith('/ui-reconcile'))
        with self.assertRaisesRegex(ValueError,'evidence operations'):
            client.ui_reconcile(transport,'BITS-1-a','op-click','click')

    def test_scope_header_and_exact_public_items(self):
        seen = []
        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                seen.append((self.path, self.headers['Authorization'], json.loads(self.rfile.read(int(self.headers['Content-Length'])))))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(json.dumps({'reservation_id':'r1','assignments':[
                    {'instance_id':'1','machine':'win-03','generation':4,'capability':'must-not-export'},
                    {'instance_id':'0','machine':'win-02','generation':4}]}).encode())
            def log_message(self, *args):
                pass
        server = HTTPServer(('127.0.0.1',0), Handler)
        worker = threading.Thread(target=server.serve_forever)
        worker.start()
        try:
            result = client.reserve({'endpoint':'http://127.0.0.1:'+str(server.server_port),
                'capability':'fixture-scoped-secret','identity':{'dag_id':'dag-real'},
                'input':{'device_count':2,'case_ids':['a','b','c']}})
            self.assertEqual(seen, [('/v1/reservations','Bearer fixture-scoped-secret',
                {'dag_id':'dag-real','target_step':'execute','count':2})])
            items = [json.loads(i) for i in result['items']]
            self.assertEqual(items[0]['case_ids'], ['a','c'])
            self.assertEqual(items[1]['case_ids'], ['b'])
            self.assertEqual(items[0]['instance_id'], '0')
            self.assertNotIn('secret', json.dumps(result))
            self.assertNotIn('capability', json.dumps(result))
            self.assertNotIn('must-not-export', json.dumps(result))
        finally:
            server.shutdown()
            worker.join()
            server.server_close()

if __name__ == '__main__':
    unittest.main()
