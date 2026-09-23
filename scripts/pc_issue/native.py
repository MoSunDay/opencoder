#!/usr/bin/env python3
"""Candidate adapter over the frozen Native harness controller and recovery protocol.

Keeps candidate profiles per work, never edits the registered machine profile.
The launch transaction follows the installed harness; monitoring/restoration remain
owned by its existing Windows supervisor and scoped NodeAPI completion verifier.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time
sys.path.insert(0, '/opt/device-cases/harness')
from controller.storage import CODE, read, save, sha, ps_string, utc, exclusive, config
from controller.cases import freeze, select_sources
from controller.driver import Driver
from controller.fleet import reserve, release
from controller.package import launcher, payload
from controller.transport import termination
from controller.workflow import child, monitor, resume_owned
from controller.device.client import context as device_context

def launch_owned(args, settings):
    from controller.device.client import context as device_context
    assigned=device_context(settings)  # Reject legacy entry before creating a run or any remote action.
    if args.concurrency != 1:
        raise ValueError('One assigned device executes cases serially with concurrency=1')
    if args.case_id is None or len(args.case_id) != 1:
        raise ValueError('Each native work must bind exactly one case; invoke subsequent cases serially')
    if args.case_id[0] not in assigned['assignment']['case_ids']:
        raise ValueError('Case is outside the host device assignment')
    frozen_source=assigned.get('input',{}).get('case_source')
    if frozen_source and Path(args.case_source).resolve()!=Path(frozen_source).resolve():
        raise ValueError('Case source differs from frozen host input')
    if args.timeout_ms<1000:
        raise ValueError('Case timeout must be at least 1000 ms')
    from controller.device.ownership import native_id
    run_id=native_id(assigned,args.case_id[0])
    if args.run_id is not None and args.run_id!=run_id:
        raise ValueError('Native run ID must match its deterministic parent-assignment case identity: '+run_id)
    candidate = assigned.get('input', {}).get('candidate')
    root=args.home/'runs'/run_id
    if (root/'run.json').exists():
        prior=root/'sources'/args.case_id[0]/'case.json'
        selected=select_sources(args.case_source,args.case_id)
        if not prior.is_file() or read(prior)!=selected[0][1]:
            raise ValueError('Native retry changed its original case input')
        return resume_owned(args.home,settings,run_id)
    if root.exists():
        from controller.fleet import recover_unstarted
        return recover_unstarted(args.home,settings,run_id)
    from controller.device.ownership import assert_next_case
    assert_next_case(assigned,args.case_id[0])
    root.mkdir(parents=True,exist_ok=False)
    root.chmod(0o700)
    cases=freeze(args.case_source,args.case_id,root,args.asset_root)
    bundle=launcher(args.home)
    local_spec={'case_file':str(root/'cases-source.json'),'case_ids':[c['case_id'] for c in cases]}
    save(root/'validate.json',local_spec)
    validation=subprocess.run(['bun',str(bundle),'validate',str(root/'validate.json')],capture_output=True,text=True)
    (root/'validation.log').write_text(validation.stdout+validation.stderr)
    if validation.returncode:
        raise RuntimeError('V3 input validation failed; see '+str(root/'validation.log'))
    queue_deadline = time.monotonic()+getattr(args, 'wait_seconds', 0)
    while True:
        termination.checkpoint()
        try:
            machine,profile=reserve(args.home,settings,run_id,args.machine,
                                    expected_profiles=getattr(args,'expected_profiles',None))
            break
        except RuntimeError as error:
            if not str(error).startswith('No prepared idle machine:') or time.monotonic() >= queue_deadline:
                raise
            time.sleep(5)
    remote=settings['guest_root']+'\\'+run_id
    state={'run_id':run_id,'status':'preparing','created_at':utc(),'machine':machine,'guest_root':remote,
           'bundle_sha256':sha(bundle.read_bytes()),'task_start_attempted':False}
    save(root/'run.json',state)
    print(json.dumps({'run_id':run_id,'machine':machine['name'],'status':'preparing','directory':str(root)}),flush=True)
    driver=Driver(settings,machine,root)
    try:
        if candidate:
            from candidate import deploy_candidate
            profile=deploy_candidate(driver,root,candidate,profile)
        termination.checkpoint()
        driver.ps((CODE/'windows/Common.ps1').read_text()+'\n'
                  +(CODE/'windows/runtime/Occupancy.ps1').read_text()+'\n'
                  +'Assert-HomeOnly '+ps_string(profile['app_path']))
        from controller.inspection.occupancy import assert_idle
        assert_idle(driver,args.home,root,remote)
        spec={'run_id':run_id,'case_ids':local_spec['case_ids'],'case_file':remote+'\\cases.json',
              'job_root':remote+'\\jobs','output_root':remote+'\\results','app_path':profile['app_path'],
              'app_commit':profile['app_commit'],'concurrency':args.concurrency,'timeout_ms':args.timeout_ms,
              'host_ready_timeout_s':settings['host_ready_timeout_s']}
        if getattr(args, 'full_evidence', False):
            from controller.transport.network import start
            capture = start(driver, root, profile, args.timeout_ms)
            spec.update(observe_canvas=True, capture_port=capture['port'],
                        capture_guest_root=capture['guest_root'])
        spec['require_film']=getattr(args,'require_film',False)
        spec['inspect_ui']=getattr(args,'inspect_ui',False)
        save(root/'spec.json',spec);save(root/'profile.json',profile)
        archive=payload(root,bundle)
        driver.ps(f"if(Test-Path {ps_string(remote)}){{throw 'Guest campaign already exists'}};New-Item -ItemType Directory {ps_string(remote)}, {child(remote,'evidence')}|Out-Null")
        driver.upload(archive,remote+'\\deployment.zip')
        driver.ps(f"Expand-Archive {child(remote,'deployment.zip')} {ps_string(remote)}")
        termination.checkpoint()
        state['task_start_attempted']=True;save(root/'run.json',state)
        receipt=driver.json(f"& {child(remote,'tools/Start-Task.ps1')} -Root {ps_string(remote)}")
        save(root/'submission.json',receipt)
        state['status']='submitted';save(root/'run.json',state)
    except BaseException as error:
        state['error']=str(error) or 'Controller interrupted before task submission'
        state['status']='submission_uncertain' if state['task_start_attempted'] else 'prepare_failed'
        save(root/'run.json',state)
        if not state['task_start_attempted']:
            from controller.transport.network import stop
            stop(driver, root)
            save(root/'cleanup.json', {'restoration_verified':True,'original_untouched':True,
                 'reason':'No supervisor submission; capture cleanup completed','verified_at':utc()})
            release(args.home,settings,run_id,machine['name'])
        raise
    return {'run_id':run_id,'status':'submitted','resume':{'run_id':run_id,
            'device_context_required':True,'business_resubmission_allowed':False}} if args.detach else monitor(args.home,settings,run_id)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--home',type=Path,required=True)
    parser.add_argument('--device-context',type=Path,required=True)
    parser.add_argument('--case-source',type=Path,required=True)
    parser.add_argument('--case-id',action='append',required=True)
    parser.add_argument('--timeout-ms',type=int,default=900000)
    args=parser.parse_args()
    args.concurrency=1;args.machine=None;args.run_id=None;args.asset_root=None
    args.full_evidence=True;args.inspect_ui=True;args.detach=False
    settings=config(args.home)
    settings['device_context']=str(args.device_context.resolve())
    from controller.device.ownership import native_id
    ctx=device_context(settings)
    if len(args.case_id)!=1:raise ValueError('One case per owned native work')
    run_id=native_id(ctx,args.case_id[0])
    with termination.observe(), exclusive(args.home/'controllers'/(run_id+'.lock')):
        value=launch_owned(args,settings)
    print(json.dumps(value,ensure_ascii=False))
    if value.get("status") not in ("completed", "passed", "submitted"):
        raise SystemExit(2)

if __name__=='__main__':
    main()
