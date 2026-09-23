"""Install an immutable candidate only inside an already registered device work."""
import json
import re
from pathlib import Path, PureWindowsPath
import stat
import zipfile
from evidence import evidence


def load_manifest(reference):
    actual=evidence(reference['manifest_path'])
    if actual['sha256']!=reference['sha256']:
        raise ValueError('Candidate manifest changed')
    manifest=json.loads(Path(actual['path']).read_text())
    archive=evidence(manifest['archive_path'])
    if archive['sha256']!=manifest['archive_sha256'] or archive['bytes']!=manifest['archive_bytes']:
        raise ValueError('Candidate archive differs from independently verified download')
    for key in ('app_relative_path','package_relative_path'):
        path=PureWindowsPath(manifest[key])
        if path.is_absolute() or path.drive or '..' in path.parts or ':' in str(path):
            raise ValueError('Candidate path must remain inside the isolated package')
    if manifest.get('download_sha256')!=archive['sha256'] or not manifest.get('build_execution_id'):
        raise ValueError('Verified download and build execution are required')
    if not re.fullmatch(r'[a-f0-9]{64}', str(manifest.get('app_sha256',''))) or not re.fullmatch(r'[a-f0-9]{40}', str(manifest.get('commit',''))):
        raise ValueError('Candidate executable hash and source commit required')
    validate_archive(manifest['archive_path'])
    return manifest


def validate_archive(path):
    if Path(path).suffix.lower()!='.zip':
        raise ValueError('Candidate must be a portable ZIP app package')
    with zipfile.ZipFile(path) as archive:
        total=0
        for info in archive.infolist():
            name=PureWindowsPath(info.filename)
            if name.is_absolute() or name.drive or '..' in name.parts or ':' in info.filename or stat.S_ISLNK(info.external_attr>>16):
                raise ValueError('Candidate archive contains an escaping path or symbolic link')
            total+=info.file_size
        if not total or total>20*1024**3:
            raise ValueError('Candidate archive is empty or exceeds 20 GiB unpacked')


def deploy_candidate(driver, root, reference, registered_profile):
    from controller.storage import CODE, ps_string, save, utc
    manifest=load_manifest(reference)
    # Driver itself verifies the active scoped work before every remote action.
    remote=str(PureWindowsPath(r'C:\VikingHarness\candidates')/root.name)
    archive=remote+r'\package.archive'
    target=remote+r'\app'
    driver.ps(f"if(Test-Path {ps_string(remote)}){{throw 'Candidate directory already exists; recover the same work'}};New-Item -ItemType Directory {ps_string(remote)}|Out-Null")
    driver.upload(manifest['archive_path'],archive)
    extract=f"Copy-Item {ps_string(archive)} {ps_string(archive+'.zip')};Expand-Archive {ps_string(archive+'.zip')} {ps_string(target)}"
    # load_manifest validates every archive entry before any remote mutation.
    driver.ps(extract)
    driver.ps(f"$r={ps_string(target)};if(@(Get-ChildItem $r -Recurse -Force|Where-Object{{$_.Attributes -band [IO.FileAttributes]::ReparsePoint}}).Count){{throw 'Candidate contains a reparse point'}}")
    app=str(PureWindowsPath(target)/manifest['app_relative_path'])
    package=str(PureWindowsPath(target)/manifest['package_relative_path'])
    common=(CODE/'windows/Common.ps1').read_text()
    inspect=(CODE/'windows/Inspect-Product.ps1').read_text().split('. "$PSScriptRoot\\Common.ps1"\n',1)[1]
    script=common+'\n$AppPath='+ps_string(app)+'\n$ExpectedAppSha='+ps_string(manifest['app_sha256'])+'\n$ExpectedCommit='+ps_string(manifest['commit'])+'\n$PackageRoot='+ps_string(package)+'\n'+inspect
    profile=driver.json(script)
    inventory=remote+r'\inventory.json'
    result=driver.json(f"$root={ps_string(target)};$files=@(Get-ChildItem $root -Recurse -File|ForEach-Object{{@{{path=$_.FullName;sha256=(Get-FileHash $_.FullName).Hash.ToLowerInvariant();bytes=$_.Length}}}});@{{files=$files}}|ConvertTo-Json -Depth 5|Set-Content -Encoding UTF8 {ps_string(inventory)};@{{path={ps_string(inventory)};sha256=(Get-FileHash {ps_string(inventory)}).Hash.ToLowerInvariant()}}|ConvertTo-Json")
    profile.update(machine=driver.machine['name'],native_source=manifest.get('source',{}),
                   registered_at=utc(),inventory=result)
    save(root/'candidate.json',{'manifest':manifest,'profile':profile,
                              'registered_profile_unchanged':registered_profile})
    return profile
