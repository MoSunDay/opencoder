param(
    [Parameter(Mandatory=$true)][string]$ArtifactBase,
    [string]$Root = "$env:LOCALAPPDATA\OpenCoder\computer",
    [string]$HostAddress = '192.168.127.10',
    [string]$Gateway = '192.168.127.1'
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8
. (Join-Path $PSScriptRoot 'windows/artifacts.ps1')
. (Join-Path $PSScriptRoot 'windows/archives.ps1')
$Root = [IO.Path]::GetFullPath($Root)
Assert-CuaFilesystemPath $Root
$manifest = Invoke-RestMethod "$ArtifactBase/manifest.json"
$plan = @(Get-CuaArtifactPlan $Root @($manifest))
foreach ($name in @('install.log', 'install-result.json', 'serve.ps1', 'server.log')) {
    Assert-CuaFilesystemPath (Join-Path $Root $name)
}
New-Item -ItemType Directory -Force -Path $Root | Out-Null
Start-Transcript -Path (Join-Path $Root 'install.log') -Append | Out-Null
$receipt = Join-Path $Root 'install-result.json'
$rule = 'OpenCoder Cua computer server'
$taskName = 'OpenCoder Cua Computer'
$ruleCreated = $false
$taskCreated = $false
try {
    if ((Get-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue) -or
        (Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue)) {
        throw 'Cua startup task or firewall rule already exists; inspect before replacing'
    }
    foreach ($item in $plan) {
        Save-CuaArtifact $Root $ArtifactBase $item
    }
    Assert-CuaZip $Root (Join-Path $Root 'uv.zip') 'uv'
    Assert-CuaTar $Root (Join-Path $Root 'python.tar.gz') 'runtime'
    Expand-Archive (Join-Path $Root 'uv.zip') (Join-Path $Root 'uv') -Force
    $runtime = Join-Path $Root 'runtime'
    New-Item -ItemType Directory -Force -Path $runtime | Out-Null
    & tar.exe -xzf (Join-Path $Root 'python.tar.gz') -C $runtime
    if ($LASTEXITCODE) { throw 'Python extraction failed' }
    $python = (Get-ChildItem $runtime -Filter python.exe -Recurse | Select-Object -First 1).FullName
    $uv = (Get-ChildItem (Join-Path $Root 'uv') -Filter uv.exe -Recurse | Select-Object -First 1).FullName
    if (!(Test-Path (Join-Path $Root 'venv\Scripts\python.exe'))) {
        & $uv venv --python $python (Join-Path $Root 'venv')
        if ($LASTEXITCODE) { throw 'Python environment creation failed' }
    }
    $python = Join-Path $Root 'venv\Scripts\python.exe'
    $wheel = Join-Path $Root 'cua_computer_server-0.3.46-py3-none-any.whl'
    $packages = Join-Path $Root 'packages'
    if (Test-Path $packages) {
        & $uv pip install --python $python --no-index --find-links $packages `
            -r (Join-Path $Root 'windows-requirements.txt')
    } else {
        & $uv pip install --python $python $wheel
    }
    if ($LASTEXITCODE) { throw 'Cua installation failed' }
    New-NetFirewallRule -DisplayName $rule -Direction Inbound -Action Allow -Protocol TCP `
        -LocalAddress $HostAddress -LocalPort 8000 -RemoteAddress $Gateway | Out-Null
    $ruleCreated = $true
    $launcher = Join-Path $Root 'serve.ps1'
    Assert-CuaFilesystemPath $launcher
    $pythonLiteral = Get-CuaLiteral $python
    $addressLiteral = Get-CuaLiteral $HostAddress
    $logLiteral = Get-CuaLiteral (Join-Path $Root 'server.log')
    @"
`$ErrorActionPreference = 'Stop'
& $pythonLiteral -m computer_server --host $addressLiteral --port 8000 --backend native `
    >> $logLiteral 2>&1
exit `$LASTEXITCODE
"@ | Set-Content $launcher -Encoding UTF8
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' `
        -Argument "-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$launcher`""
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
    $settings = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) `
        -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal `
        -Trigger $trigger -Settings $settings | Out-Null
    $taskCreated = $true
    Start-ScheduledTask -TaskName $taskName
    @{status='installed';root=$Root;task=$taskName;session=(Get-Process -Id $PID).SessionId} |
        ConvertTo-Json -Compress | Set-Content $receipt -Encoding UTF8
} catch {
    $failure = $_
    if ($taskCreated) {
        Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
    }
    if ($ruleCreated) {
        Remove-NetFirewallRule -DisplayName $rule -ErrorAction SilentlyContinue
    }
    @{status='failed';error=$failure.Exception.Message} | ConvertTo-Json -Compress |
        Set-Content $receipt -Encoding UTF8
    throw $failure
} finally {
    Stop-Transcript | Out-Null
}
