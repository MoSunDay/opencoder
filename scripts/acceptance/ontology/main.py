#!/usr/bin/env python3
"""Run the current Server, browser and actual read-only NFS mounts in a private namespace."""
import argparse
import errno
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.request
import uuid


def port():
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        return listener.getsockname()[1]


def evidence_root(requested):
    root = requested.resolve()
    protected = [Path('/etc'), Path('/run'), Path('/var/lib/opencoder-platform'),
                 Path(__file__).resolve().parents[3]]
    if root.exists() or any(root.is_relative_to(path) for path in protected):
        raise ValueError('Use a new evidence directory outside the repository and production state')
    return root


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server', required=True, type=Path)
    parser.add_argument('--root', required=True, type=Path)
    parser.add_argument('--inside', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if not args.inside:
        os.execv('/usr/bin/unshare', ['unshare', '-m', '--propagation', 'private', sys.executable,
            str(Path(__file__).resolve()), *sys.argv[1:], '--inside'])
    root = evidence_root(args.root)
    work, data, files, mount = (root/name for name in ('work', 'server-data', 'files', 'mount'))
    for path in [work, files, mount]: path.mkdir(parents=True)
    http_port, nfs_port = port(), port()
    (work/'opencoder.json').write_text(json.dumps({'storage':{'backend':'libsql'},
        'agent':{'agents_dir':str(root/'agents'),'nfs':{'enabled':False}},
        'dag':{'binary_dir':str(root/'binary'),'workspace_dir':str(root/'workspace'),
               'nfs':{'enabled':False},'workspace_nfs':{'enabled':False}},
        'ontology':{'files_dir':str(files),'nfs':{'enabled':True,'port':nfs_port}}}))
    base = f'http://127.0.0.1:{http_port}'
    def api(path, method='GET', value=None):
        body = json.dumps(value).encode() if value is not None else None
        request = urllib.request.Request(base + '/api/ontology' + path, data=body, method=method,
            headers={'Authorization':'Bearer ontology-fixture','Content-Type':'application/json'})
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=15) as reply: return json.load(reply)
    log = (root/'server.log').open('w')
    process = subprocess.Popen([str(args.server.resolve()), '--workdir', str(work), '--data-dir', str(data),
        '--port', str(http_port), '--token', 'ontology-fixture', '--web'], stdout=log, stderr=log)
    mounted = False
    receipt = {'passed':False}
    try:
        deadline = time.monotonic()+60
        while True:
            try: api('/environments'); break
            except OSError:
                if process.poll() is not None or time.monotonic()>deadline: raise
                time.sleep(.2)
        kind = api('/envs/debug/entity-types','POST',{'key':'service','name':'服务'})['item']['id']
        entities = [api('/envs/debug/entities','POST',{'request_id':str(uuid.uuid4()),'entity_type_id':kind,
            'name':name,'source':'# 来源','ext':'# 拓展'})['item']['id'] for name in ['入口服务','下游服务']]
        relation = api('/envs/debug/relationship-types','POST',{'key':'calls','name':'调用',
            'source_entity_type_id':kind,'target_entity_type_ids':[kind]})['item']['id']
        api('/envs/debug/relationships','POST',{'relationship_type_id':relation,
            'source_entity_id':entities[0],'target_entity_id':entities[1]})
        detail=api('/envs/debug/entities/'+entities[0])
        current=detail['text_attributes'][0]['current']
        options=f'rw,vers=3,tcp,port={nfs_port},mountport={nfs_port},nolock,soft,timeo=20,retrans=1,actimeo=0,lookupcache=none'
        subprocess.run(['mount','-t','nfs','-o',options,'127.0.0.1:/',str(mount)],check=True,timeout=30)
        mounted=True
        assert (mount/current['content_path']).read_bytes()==(files/current['content_path']).read_bytes()
        try: (mount/'forbidden-write').write_text('must fail')
        except OSError as error: assert error.errno in [errno.EROFS,errno.EACCES,errno.EPERM]
        else: raise AssertionError('NFS server accepted a write')
        assert not (files/'forbidden-write').exists() and not (mount/'ontology.db').exists()
        status=api('/nfs'); assert status['status']['read_only'] is True
        browser=subprocess.run(['node',str(Path(__file__).with_name('browser.mjs')),base,str(root)],
            timeout=120,capture_output=True,text=True)
        (root/'browser.log').write_text(browser.stdout+browser.stderr)
        browser.check_returncode()
        receipt.update(passed=True,nfs_read_only=True,browser=json.loads(browser.stdout.strip().splitlines()[-1]))
    finally:
        if mounted: subprocess.run(['umount',str(mount)],check=True,timeout=30)
        process.terminate(); process.wait(timeout=30); log.close()
        receipt['cleanup']=True
        (root/'result.json').write_text(json.dumps(receipt,ensure_ascii=False,indent=2))
    print(json.dumps(receipt,ensure_ascii=False))


if __name__=='__main__': main()
