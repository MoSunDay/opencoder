#Requires -Version 7.4
[CmdletBinding()]
param([Parameter(Mandatory)][string]$BinaryDirectory)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'installer/state.ps1')
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
$root = Join-Path ([IO.Path]::GetTempPath()) "opencoder-install-test-$([guid]::NewGuid())"
$bundle = Join-Path $root 'bundle'
$destination = Join-Path $root '安装 空格/bin'
$originalPath = [Environment]::GetEnvironmentVariable('Path', 'User')
function Check([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function Write-Sums {
    Get-ChildItem -File -Recurse $bundle | Where-Object Name -NE 'SHA256SUMS' | Sort-Object FullName | ForEach-Object {
        "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLowerInvariant())  $([IO.Path]::GetRelativePath($bundle, $_.FullName).Replace('\', '/'))"
    } | Set-Content -Encoding utf8NoBOM (Join-Path $bundle 'SHA256SUMS')
}
function Install { & (Join-Path $bundle 'install.ps1') -Bundle $bundle -Destination $destination -NoPath }
function Must-Fail([scriptblock]$Action) {
    $failed = $false
    try { & $Action } catch { $failed = $true }
    Check $failed 'Installer accepted an invalid or locked installation.'
}
try {
    New-Item -ItemType Directory (Join-Path $bundle 'bin') -Force | Out-Null
    $entries = foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
        Copy-Item -LiteralPath (Join-Path $BinaryDirectory $name) -Destination (Join-Path $bundle 'bin')
        @{ name=$name; sha256=(Get-FileHash -Algorithm SHA256 (Join-Path $bundle "bin/$name")).Hash.ToLowerInvariant() }
    }
    $info = & (Join-Path $bundle 'bin/opencoder.exe') --build-info | ConvertFrom-Json
    Check ($LASTEXITCODE -eq 0) 'Cannot query compiled TUI metadata.'
    $manifest = @{ schema_version=1; platform='windows-x64'; build_info=$info; binaries=@($entries) }
    $manifest | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8NoBOM (Join-Path $bundle 'manifest.json')
    Copy-Item (Join-Path $repo 'scripts/install.ps1') $bundle
    Copy-Item (Join-Path $repo 'scripts/platform/windows/installer') (Join-Path $bundle 'installer') -Recurse
    Copy-Item (Join-Path $repo 'docs/windows.md') (Join-Path $bundle 'README.md')
    Write-Sums
    Install
    $installed = Join-Path $destination 'opencoder.exe'
    $before = (Get-FileHash -Algorithm SHA256 $installed).Hash
    Check ((Get-FileHash -Algorithm SHA256 (Join-Path $bundle 'bin/opencoder.exe')).Hash -eq $before) 'Fresh install is not the package binary.'
    Install
    Check (@(Get-ChildItem (Split-Path $destination -Parent) -Directory -Filter 'bin.previous-*').Count -eq 1) 'Upgrade did not preserve the previous version.'
    Add-Content -LiteralPath (Join-Path $bundle 'README.md') 'checksum corruption'
    Must-Fail { Install }
    Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $before) 'Checksum failure changed the old installation.'
    Write-Sums
    $manifest.build_info.git_commit = 'mismatched-commit'
    $manifest | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8NoBOM (Join-Path $bundle 'manifest.json')
    Write-Sums
    Must-Fail { Install }
    Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $before) 'Metadata failure changed the old installation.'
    $manifest.build_info = & (Join-Path $bundle 'bin/opencoder.exe') --build-info | ConvertFrom-Json
    $manifest | ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8NoBOM (Join-Path $bundle 'manifest.json')
    Write-Sums
    $lock = [IO.File]::Open($installed, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try { Must-Fail { Install } } finally { $lock.Dispose() }
    Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $before) 'Locked executable failure lost the old installation.'
    foreach ($record in Get-ChildItem "$destination.install-state" -Directory) {
        Check (Test-Path -LiteralPath (Join-Path $record.FullName 'before.json')) 'Unpublished install directory leaked.'
    }
    Check ([Environment]::GetEnvironmentVariable('Path', 'User') -eq $originalPath) 'NoPath changed the user PATH.'
    $journalLock = [OpenCoderInstallFiles]::Lock((Join-Path "$destination.install-state" 'lock'))
    try { Must-Fail { Install } } finally { $journalLock.Dispose() }
    Check ((Get-FileHash -Algorithm SHA256 $installed).Hash -eq $before) 'Concurrent installation changed the binary group.'
    $pathDestination = Join-Path $root 'PATH 安装/bin'
    & (Join-Path $bundle 'install.ps1') -Bundle $bundle -Destination $pathDestination
    Check (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $pathDestination) 'Installation did not add its PATH entry.'
    & (Join-Path $bundle 'install.ps1') -Bundle $bundle -Destination $pathDestination -Recover
    $otherEntry = Join-Path $root 'other-application'
    $currentPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    [Environment]::SetEnvironmentVariable('Path', "$currentPath;$otherEntry", 'User')
    & (Join-Path $bundle 'install.ps1') -Bundle $bundle -Destination $pathDestination -Rollback
    Check (-not (Test-Path -LiteralPath $pathDestination)) 'Fresh rollback left the installation active.'
    Check (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $otherEntry) 'Rollback removed another application PATH entry.'
    Check (-not (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $pathDestination)) 'Rollback retained its own PATH entry.'
    & (Join-Path $bundle 'install.ps1') -Bundle $bundle -Destination $pathDestination -Recover
    Write-Host 'Windows installer: fresh install, upgrade, invalid package, locked EXE, concurrency, recovery and PATH rollback passed.'
} finally {
    if (Get-Command Remove-InstallPathEntry -ErrorAction SilentlyContinue) {
        $currentPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        foreach ($entry in @($pathDestination, $otherEntry)) {
            if ($entry) { $currentPath = Remove-InstallPathEntry $currentPath $entry }
        }
        [Environment]::SetEnvironmentVariable('Path', $currentPath, 'User')
    }
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
