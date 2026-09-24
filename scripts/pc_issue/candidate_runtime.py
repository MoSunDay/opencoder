"""Require actual packaged sandbox readiness before a candidate's first job."""
import hashlib
import zipfile
from pathlib import Path

HOST_READY = ' $health=Wait-Host $profile $native.Id $spec.host_ready_timeout_s'
GUARD = r'''
 $sandboxHealth=Invoke-RestMethod "http://127.0.0.1:$($health.port)/.custom-agent/health" -TimeoutSec 4
 Write-Json "$Root\evidence\candidate-sandbox-health.json" $sandboxHealth
 $nativeMonitoring=$sandboxHealth.nativeMonitoring
 $sandbox=$nativeMonitoring.sandbox
 if(!$nativeMonitoring.loaded -or !$nativeMonitoring.sandboxBridgePublished -or !$sandbox.initialized -or !$sandbox.capabilities.supported -or !$sandbox.capabilities.initialized){
  throw "Candidate sandbox readiness failed; SDK code=$($sandbox.capabilities.lastError.sdkErrorCode)"
 }
'''


def guarded_payload(root, bundle):
    from controller.package import payload
    from controller.storage import save
    original = payload(root, bundle)
    target = Path(root) / 'candidate-deployment.zip'
    changed = set()
    with zipfile.ZipFile(original) as source, zipfile.ZipFile(target, 'w', zipfile.ZIP_DEFLATED) as out:
        for item in source.infolist():
            data = source.read(item)
            if item.filename == 'tools/Invoke-Campaign.ps1':
                text = data.decode('utf-8-sig')
                if text.count(HOST_READY) != 1:
                    raise ValueError('Frozen supervisor readiness hook changed')
                data = text.replace(HOST_READY, HOST_READY + '\n' + GUARD, 1).encode('utf-8-sig')
                changed.add('sandbox')
            if item.filename == 'tools/runtime/UI-Checkpoint.ps1':
                text = data.decode('utf-8-sig')
                deadline = '$deadline=(Get-Date).AddMinutes(20)'
                if text.count(deadline) != 1:
                    raise ValueError('Frozen UI review deadline changed')
                data = text.replace(deadline, '$deadline=(Get-Date).AddMinutes(75)', 1).encode('utf-8-sig')
                changed.add('review')
            out.writestr(item, data)
    if changed != {'sandbox', 'review'}:
        raise ValueError('Candidate supervisor absent')
    target.chmod(0o600)
    save(Path(root)/'candidate-supervisor.json', {
        'archive_sha256': hashlib.sha256(target.read_bytes()).hexdigest(),
        'sandbox_readiness_required': True,
        'product_modified': False,
        'ui_review_timeout_minutes': 75,
    })
    return target
