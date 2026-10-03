#Requires -Version 7.4
[CmdletBinding()]
param(
    [string]$Bundle = $PSScriptRoot,
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'OpenCoder/bin'),
    [switch]$NoPath,
    [switch]$Recover,
    [switch]$Rollback
)
$ErrorActionPreference = 'Stop'
if ($Recover -and $Rollback) { throw 'Choose either -Recover or -Rollback.' }
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne [Runtime.InteropServices.Architecture]::X64) { throw 'OpenCoder requires Windows 11 x64 and PowerShell 7.4 or newer.' }
if ($PSVersionTable.PSVersion.Major -ne 7) { throw 'Use PowerShell 7 (pwsh.exe).' }
$Bundle = (Resolve-Path -LiteralPath $Bundle).Path
$Destination = [IO.Path]::GetFullPath($Destination).TrimEnd('\', '/')
$names = @('opencoder.exe', 'opencoder-agent.exe')
$expectedFiles = @('bin/opencoder.exe', 'bin/opencoder-agent.exe', 'manifest.json', 'install.ps1', 'README.md', 'installer/fs.ps1', 'installer/state.ps1')
$checked = @{}
foreach ($line in Get-Content -LiteralPath (Join-Path $Bundle 'SHA256SUMS')) {
    if ($line -notmatch '^([0-9a-f]{64})  ([a-zA-Z0-9./_-]+)$') { throw 'Invalid package checksum entry.' }
    $hash, $relative = $Matches[1], $Matches[2]
    if ($relative -notin $expectedFiles -or $checked.ContainsKey($relative)) { throw "Unexpected package entry: $relative" }
    $file = Get-Item -LiteralPath (Join-Path $Bundle $relative)
    if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Package contains a link: $relative" }
    if ((Get-FileHash -Algorithm SHA256 $file.FullName).Hash.ToLowerInvariant() -ne $hash) { throw "Checksum mismatch: $relative" }
    $checked[$relative] = $hash
}
if ($checked.Count -ne $expectedFiles.Count) { throw 'Package checksums are incomplete.' }
$manifest = Get-Content -Raw -LiteralPath (Join-Path $Bundle 'manifest.json') | ConvertFrom-Json
if ($manifest.schema_version -ne 1 -or $manifest.platform -ne 'windows-x64' -or $manifest.binaries.Count -ne 2) { throw 'Unsupported package manifest.' }
foreach ($name in $names) {
    $entry = @($manifest.binaries | Where-Object name -EQ $name)
    if ($entry.Count -ne 1 -or $entry[0].sha256 -ne $checked["bin/$name"]) { throw "Manifest checksum mismatch: $name" }
}
$parent = Split-Path $Destination -Parent
if (-not $parent) { throw 'Choose an installation directory below a drive root.' }
. (Join-Path $Bundle 'installer/fs.ps1')
. (Join-Path $Bundle 'installer/state.ps1')
Assert-InstallPath $Destination
New-Item -ItemType Directory -Force $parent | Out-Null
$journalRoot = "$Destination.install-state"
New-InstallPrivateDirectory $journalRoot
$lockFile = Join-Path $journalRoot 'lock'
if (Test-Path -LiteralPath $lockFile) { Assert-InstallPrivate $lockFile }
$lock = [OpenCoderInstallFiles]::Lock($lockFile)
$attempt = $null
$published = $false
$leases = [Collections.Generic.List[IO.FileStream]]::new()
try {
    $previous = Get-InstallAttempt $journalRoot $Destination
    if ($Recover -or $Rollback) {
        if (-not $previous) { throw 'No installation recovery record exists.' }
        if ($Recover -and $previous.phase -eq 'committed') {
            if (-not (Test-InstallGroup $Destination $previous.before.new_hashes)) { throw 'Completed installation changed; recovery stopped.' }
            if (-not $previous.before.no_path -and -not (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $Destination)) { throw 'Completed installation PATH changed.' }
            Write-Host 'Installation is already complete.'
            return
        }
        Restore-InstallAttempt $previous
        Write-Host "Restored installation: $Destination"
        return
    }
    if ($previous -and $previous.phase -notin @('committed', 'restored')) {
        Restore-InstallAttempt $previous
    }
    $attempt = New-InstallAttempt $journalRoot $Destination ([bool]$NoPath)
    $stage = $attempt.before.stage
    New-InstallPrivateDirectory $stage
    foreach ($name in $names) { [OpenCoderInstallFiles]::Copy((Join-Path $Bundle "bin/$name"), (Join-Path $stage $name)) }
    $hashes = Get-InstallGroup $stage
    foreach ($name in $names) {
        if ($hashes[$name] -ne $checked["bin/$name"]) { throw "Staged binary checksum mismatch: $name" }
    }
    $expected = $manifest.build_info | ConvertTo-Json -Depth 20 -Compress
    foreach ($name in $names) {
        $raw = & (Join-Path $stage $name) --build-info
        if ($LASTEXITCODE -ne 0 -or (($raw | ConvertFrom-Json | ConvertTo-Json -Depth 20 -Compress) -ne $expected)) {
            throw "Installed binary metadata mismatch: $name"
        }
    }
    if ($attempt.before.had_destination) {
        foreach ($lease in (Lock-InstallGroup $Destination $attempt.before.old_hashes)) { $leases.Add($lease) }
    }
    Publish-InstallAttempt $journalRoot $attempt $hashes
    $published = $true
    Set-InstallPhase $attempt 'switching'
    # Windows rejects a directory rename with open descendant handles, even
    # with delete sharing. The installer mutex remains held throughout; a
    # racing program launch makes the directory rename fail without mixing EXEs.
    foreach ($lease in $leases) { $lease.Dispose() }
    $leases.Clear()
    if (Test-Path -LiteralPath $Destination) {
        Move-InstallDirectory $Destination $attempt.before.backup
    }
    Move-InstallDirectory $stage $Destination
    Set-InstallPhase $attempt 'installed'
    if (-not $NoPath) {
        [string]$current = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (-not (Test-InstallPathEntry $current $Destination)) {
            [Environment]::SetEnvironmentVariable('Path', (Add-InstallPathEntry $current $Destination), 'User')
        }
        if ($Destination -notin ($env:PATH -split ';')) { $env:PATH = "$Destination;$env:PATH" }
        if (-not (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $Destination)) { throw 'Installed PATH verification failed.' }
    }
    foreach ($lease in $leases) { $lease.Dispose() }
    $leases.Clear()
    if (-not (Test-InstallGroup $Destination $hashes)) { throw 'Installed binaries verification failed.' }
    Set-InstallPhase $attempt 'committed'
    Write-Host "Installed TUI and operator: $Destination"
    if ($attempt.before.had_destination) { Write-Host "Previous binaries: $($attempt.before.backup)" }
} catch {
    $failure = $_
    foreach ($lease in $leases) { $lease.Dispose() }
    $leases.Clear()
    if ($published) { Restore-InstallAttempt $attempt }
    throw $failure
} finally {
    try {
        foreach ($lease in $leases) { $lease.Dispose() }
        if ($attempt -and -not (Test-Path -LiteralPath (Join-Path $attempt.directory 'before.json'))) {
            Assert-InstallPath $attempt.directory
            Remove-Item -LiteralPath $attempt.directory -Recurse -Force
        }
    } finally { $lock.Dispose() }
}
