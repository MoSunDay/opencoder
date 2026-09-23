#!/usr/bin/env python3
"""Controlled PC issue device/build submissions and evidence inspection."""
import argparse
import json
from pathlib import Path
import sys
from client import Client
from contracts import build_request, device_request, identity, inspection
from evidence import evidence, freeze_source, verify_source


def parser():
    root = argparse.ArgumentParser(description=__doc__)
    root.add_argument('--endpoint', default='http://127.0.0.1:18081')
    root.add_argument('--token-file', default='/etc/opencoder/server.token')
    sub = root.add_subparsers(dest='command', required=True)
    sub.add_parser('install', help='Register immutable built-in PC issue plan')
    sub.add_parser('preflight', help='Read server, registered nodes, team and device readiness')
    for name in ('device', 'build'):
        command = sub.add_parser(name)
        command.add_argument('--root', required=True)
        command.add_argument('--parent', required=True, help='Owning PC stage execution ID')
        command.add_argument('--node', required=True)
        command.add_argument('--round', type=int, choices=(1, 2), default=1)
        if name == 'device':
            command.add_argument('--stage', choices=('reproduce', 'verify'), required=True)
            command.add_argument('--source', type=Path, required=True)
            command.add_argument('--case-id', action='append', required=True)
            command.add_argument('--candidate', type=Path, help='Verified portable candidate manifest')
            command.add_argument('--artifacts', type=Path, default=Path('/data00/workspace/artifacts/pc-issues'))
        else:
            command.add_argument('--request', type=Path, required=True)
    for name in ('inspect', 'cancel'):
        command = sub.add_parser(name)
        command.add_argument('execution_id')
    command = sub.add_parser('evidence')
    command.add_argument('paths', type=Path, nargs='+')
    return root


def run(args):
    if args.command == 'evidence':
        return [evidence(path) for path in args.paths]
    client = Client(args.endpoint, args.token_file)
    if args.command == 'install':
        return client.call('POST', '/api/brain/pc-issue/plan', {})
    if args.command == 'preflight':
        devices = Client('http://127.0.0.1:18279', '/etc/device-manager/admin.token').call('GET', '/v1/devices')
        return {'server': client.call('GET', '/api/health'),
                'nodes': client.call('GET', '/api/nodes'),
                'teams': client.call('GET', '/api/teams'), 'devices': devices}
    if args.command == 'inspect':
        return inspection(client.call('GET', '/api/executions/' + args.execution_id))
    if args.command == 'cancel':
        return client.call('POST', '/api/executions/' + args.execution_id + '/commands', {'action': 'cancel'})
    if args.command == 'device':
        eid = identity(args.root, args.stage, args.round, 'dag')
        destination = args.artifacts / args.root / eid / 'cases.json'
        source = freeze_source(args.source, destination)
        request = device_request(args.root, args.stage, args.round, args.node, source, args.case_id)
        if args.candidate:
            from candidate import load_manifest
            ref=evidence(args.candidate)
            candidate={'manifest_path':ref['path'],'sha256':ref['sha256']}
            load_manifest(candidate)
            probe=client.call('GET','/api/nodes/'+args.node+'/execution-capabilities')
            if 'pc_candidate_v1' not in probe.get('features',[]):
                raise ValueError('Device node does not advertise pc_candidate_v1; upgrade its controlled runtime before candidate verification')
            request['input']['candidate']=candidate
    else:
        source = verify_source(json.loads(args.request.read_text()))
        request = build_request(args.root, args.round, args.node, source)
    request['input']['pc_issue_parent']={'execution_id':args.parent,'run_id':args.root}
    return {'execution_id': request['id'], 'receipt': client.submit(request)}


if __name__ == '__main__':
    try:
        print(json.dumps(run(parser().parse_args()), ensure_ascii=False))
    except Exception as error:
        print(json.dumps({'error': str(error)}, ensure_ascii=False), file=sys.stderr)
        sys.exit(1)
