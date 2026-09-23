"""Isolated HTTP contract checks: no device API or business submissions."""
import importlib.util
import json
from pathlib import Path
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

spec = importlib.util.spec_from_file_location('device_client', Path(__file__).with_name('client.py'))
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)

class TransportTest(unittest.TestCase):
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
