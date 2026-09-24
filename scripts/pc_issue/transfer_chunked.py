"""Transfer a verified candidate in bounded device actions."""
import json
from pathlib import Path
import select
import shlex
import subprocess

from controller.storage import ps_string, save, sha
from controller.transport.http.listener import REMOTE_BIND_SOURCE

CHUNK = 32 * 1024 * 1024
SERVER = REMOTE_BIND_SOURCE + r'''
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
import json, os, sys
file = Path(sys.argv[1]); chunk = int(sys.argv[3]); size = file.stat().st_size
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  route = urlsplit(self.path)
  offset = int(parse_qs(route.query).get('offset', ['-1'])[0])
  if route.path != '/' + file.name or offset < 0 or offset >= size or offset % chunk:
   self.send_error(400); return
  count = min(chunk, size - offset)
  self.send_response(200); self.send_header('Content-Length', str(count)); self.end_headers()
  with file.open('rb') as source:
   source.seek(offset)
   remaining = count
   while remaining:
    data = source.read(min(1024 * 1024, remaining))
    if not data: raise IOError('short candidate archive')
    self.wfile.write(data); remaining -= len(data)
 def log_message(self,*args): pass
server = bind_transfer_server(HTTPServer, Handler, sys.argv[2])
print(json.dumps({'pid': os.getpid(), 'port': server.server_port}), flush=True)
try: server.serve_forever(poll_interval=.5)
finally: server.server_close()
'''


def upload_candidate(driver, local, remote, digest):
    source = Path(local)
    if sha(source.read_bytes()) != digest:
        raise ValueError('Candidate archive changed before transfer')
    host = driver.settings.get('transfer_host', '10.199.81.94')
    base = driver.settings.get('transfer_root', '/data00/windows-eval/native-harness-transfer')
    staged = base + '/' + digest + '.zip'
    subprocess.run(['ssh', '-o', 'BatchMode=yes', host,
                    'mkdir -p ' + shlex.quote(base) + ' && chmod 700 ' + shlex.quote(base)],
                   check=True, capture_output=True)
    check = subprocess.run(['ssh', '-o', 'BatchMode=yes', host,
                            'sha256sum ' + shlex.quote(staged)], capture_output=True, text=True)
    if check.returncode or check.stdout.split()[0] != digest:
        subprocess.run(['scp', '-q', str(source), host + ':' + shlex.quote(staged)],
                       check=True, capture_output=True)
        check = subprocess.run(['ssh', '-o', 'BatchMode=yes', host,
                                'sha256sum ' + shlex.quote(staged)], check=True,
                               capture_output=True, text=True)
        if check.stdout.split()[0] != digest:
            raise ValueError('Staged candidate archive hash differs')
    command = 'python3 -u -c ' + shlex.quote(SERVER) + ' ' + shlex.quote(staged)
    command += ' ' + shlex.quote(host) + ' ' + str(CHUNK)
    process = subprocess.Popen(['ssh', '-o', 'BatchMode=yes', host, command],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    info = None
    temporary = remote + '.upload-' + digest[:16] + '.tmp'
    try:
        if not select.select([process.stdout], [], [], 20)[0]:
            raise TimeoutError('Candidate transfer server did not start')
        line = process.stdout.readline()
        if not line:
            raise RuntimeError('Candidate transfer server failed: ' + process.stderr.read()[:1000])
        info = json.loads(line)
        driver.ps(f"$p={ps_string(temporary)};$f=[IO.File]::Open($p,[IO.FileMode]::Create);$f.Dispose()")
        for offset in range(0, source.stat().st_size, CHUNK):
            count = min(CHUNK, source.stat().st_size - offset)
            url = f"http://{host}:{info['port']}/{digest}.zip?offset={offset}"
            script = (f"$c=New-Object Net.WebClient;$c.Proxy=$null;"
                      f"try{{$b=$c.DownloadData({ps_string(url)})}}finally{{$c.Dispose()}};"
                      f"if($b.Length -ne {count}){{throw 'Candidate chunk length differs'}};"
                      f"$f=[IO.File]::Open({ps_string(temporary)},[IO.FileMode]::Open);"
                      f"try{{$f.Seek({offset},[IO.SeekOrigin]::Begin)|Out-Null;$f.Write($b,0,$b.Length)}}"
                      f"finally{{$f.Dispose()}}")
            driver.ps(script)
        final = (f"$p={ps_string(temporary)};"
                 f"if((Get-Item $p).Length -ne {source.stat().st_size}){{throw 'Candidate size differs'}};"
                 f"if((Get-FileHash $p).Hash.ToLowerInvariant() -ne '{digest}')"
                 "{throw 'Candidate hash differs'};"
                 f"[IO.File]::Move($p,{ps_string(remote)})")
        driver.ps(final)
        save(driver.root / 'candidate-transfer.json',
             {'sha256': digest, 'bytes': source.stat().st_size, 'chunk_bytes': CHUNK,
              'chunks': (source.stat().st_size + CHUNK - 1) // CHUNK})
    finally:
        if process.poll() is None:
            if info:
                stop = ("import os,signal;from pathlib import Path;p=" + str(info['pid'])
                        + ";f=" + repr(staged) + ";b=Path('/proc/%d/cmdline'%p);"
                        + "assert not b.exists() or f.encode() in b.read_bytes();"
                        + "os.kill(p,signal.SIGTERM) if b.exists() else None")
                subprocess.run(['ssh', '-o', 'BatchMode=yes', host,
                                'python3 -c ' + shlex.quote(stop)], capture_output=True, timeout=20)
            process.terminate()
            process.wait(timeout=10)
