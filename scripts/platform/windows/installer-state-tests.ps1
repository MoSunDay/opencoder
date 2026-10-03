#Requires -Version 7.4
[CmdletBinding()]
param([switch]$Portable, [string]$ChildCase, [string]$StopAt)
$ErrorActionPreference = 'Stop'
trap { Write-Host $_.ScriptStackTrace; throw $_ }
. (Join-Path $PSScriptRoot 'installer/state.ps1')
if ($Portable) {
    # Exercise the same recovery transitions with ordinary temporary files on
    # Unix. Windows CI uses the creation-time ACL and Win32 durability helpers.
    function Assert-InstallPath([string]$Path) {
        if (Test-Path -LiteralPath $Path) {
            if ((Get-Item -LiteralPath $Path).LinkType) { throw 'Fixture link rejected.' }
        }
    }
    function New-InstallPrivateDirectory([string]$Path) { New-Item -ItemType Directory -Path $Path -Force | Out-Null }
    function Write-InstallJson([string]$Path, $Value, [switch]$CreateOnly) {
        if ($CreateOnly -and (Test-Path -LiteralPath $Path)) { throw 'Immutable before-image already exists.' }
        $temporary = "$Path.new"
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 20 -Compress))
        $file = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew)
        try { $file.Write($bytes); $file.Flush($true) } finally { $file.Dispose() }
        [IO.File]::Move($temporary, $Path, $true)
    }
    function Read-InstallJson([string]$Path) { Get-Content -Raw -LiteralPath $Path | ConvertFrom-Json -AsHashtable }
    function Move-InstallDirectory([string]$Source, [string]$Destination) { [IO.Directory]::Move($Source, $Destination) }
} else {
    if (-not $IsWindows) { throw 'Use -Portable for Unix recovery transition tests.' }
    . (Join-Path $PSScriptRoot 'installer/fs.ps1')
}
function Check([bool]$Condition, [string]$Message) { if (-not $Condition) { throw $Message } }
function New-TestBinaryGroup([string]$Directory, [string]$Content) {
    New-InstallPrivateDirectory $Directory
    foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
        [IO.File]::WriteAllText((Join-Path $Directory $name), "$Content-$name")
    }
}
function Stop-TestInstaller([string]$Point, $Attempt) {
    if ($Point -ne $StopAt) { return }
    $beforeHash = (Get-FileHash -LiteralPath (Join-Path $Attempt.directory 'before.json')).Hash
    $ready = Join-Path $ChildCase 'ready.json'
    if (Test-Path -LiteralPath $ready) { throw 'The interruption signal already exists.' }
    Write-InstallJson $ready @{ before_hash=$beforeHash }
    # The parent kills this process, bypassing all catch/finally compensation.
    [Threading.Thread]::Sleep([Threading.Timeout]::Infinite)
}
if ($ChildCase) {
    New-InstallPrivateDirectory $ChildCase
    $destination = Join-Path $ChildCase 'bin'
    New-TestBinaryGroup $destination 'old'
    $journal = "$destination.install-state"
    New-InstallPrivateDirectory $journal
    $lockPath = Join-Path $journal 'lock'
    $lock = if ($Portable) { [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None') } else { [OpenCoderInstallFiles]::Lock($lockPath) }
    try {
        $attempt = New-InstallAttempt $journal $destination $true
        New-TestBinaryGroup $attempt.before.stage 'new'
        Publish-InstallAttempt $journal $attempt (Get-InstallGroup $attempt.before.stage)
        Set-InstallPhase $attempt 'switching'
        Stop-TestInstaller 'prepared' $attempt
        Move-InstallDirectory $destination $attempt.before.backup
        Stop-TestInstaller 'old_moved' $attempt
        Move-InstallDirectory $attempt.before.stage $destination
        Stop-TestInstaller 'new_moved' $attempt
        Set-InstallPhase $attempt 'restoring'
        Move-InstallDirectory $destination $attempt.before.discard
        Stop-TestInstaller 'discarded' $attempt
        Move-InstallDirectory $attempt.before.backup $destination
        Stop-TestInstaller 'old_restored' $attempt
        throw 'Unknown interruption point.'
    } finally { $lock.Dispose() }
    return
}
$root = Join-Path ([IO.Path]::GetTempPath()) "opencoder-recovery-test-$([guid]::NewGuid())"
try {
    New-InstallPrivateDirectory $root
    foreach ($point in @('prepared', 'old_moved', 'new_moved', 'discarded', 'old_restored')) {
        $caseName = if ($point -eq 'prepared') { "$point-$('x' * 140)" } else { $point }
        $case = Join-Path $root $caseName
        New-InstallPrivateDirectory $case
        $destination = Join-Path $case 'bin'
        New-TestBinaryGroup $destination 'old'
        $journal = "$destination.install-state"
        New-InstallPrivateDirectory $journal
        $attempt = New-InstallAttempt $journal $destination $true
        New-TestBinaryGroup $attempt.before.stage 'new'
        $hashes = Get-InstallGroup $attempt.before.stage
        Publish-InstallAttempt $journal $attempt $hashes
        $beforeFile = Join-Path $attempt.directory 'before.json'
        if ($IsWindows -and $point -eq 'prepared') { Check ($beforeFile.Length -gt 260) 'Long-path recovery fixture is too short.' }
        $beforeHash = (Get-FileHash -LiteralPath $beforeFile).Hash
        Set-InstallPhase $attempt 'switching'
        if ($point -ne 'prepared') { Move-InstallDirectory $destination $attempt.before.backup }
        if ($point -in @('new_moved', 'discarded', 'old_restored')) { Move-InstallDirectory $attempt.before.stage $destination }
        if ($point -in @('discarded', 'old_restored')) {
            Set-InstallPhase $attempt 'restoring'
            Move-InstallDirectory $destination $attempt.before.discard
        }
        if ($point -eq 'old_restored') { Move-InstallDirectory $attempt.before.backup $destination }
        # Reload from disk: no flags from the failed installer survive.
        $loaded = Get-InstallAttempt $journal $destination
        Restore-InstallAttempt $loaded
        Check (Test-InstallGroup $destination $attempt.before.old_hashes) "Old binary group not restored at $point."
        Check ((Get-FileHash -LiteralPath $beforeFile).Hash -eq $beforeHash) 'Restoration overwrote the before-image.'
        Restore-InstallAttempt (Get-InstallAttempt $journal $destination)
        Check (Test-InstallGroup $destination $attempt.before.old_hashes) 'Repeated recovery changed the restored group.'
        Write-Host "Recovery transition passed: $point"
    }
    $fresh = Join-Path $root 'fresh'
    New-InstallPrivateDirectory $fresh
    $destination = Join-Path $fresh 'bin'
    $journal = "$destination.install-state"
    New-InstallPrivateDirectory $journal
    $attempt = New-InstallAttempt $journal $destination $true
    New-TestBinaryGroup $attempt.before.stage 'new'
    Publish-InstallAttempt $journal $attempt (Get-InstallGroup $attempt.before.stage)
    Move-InstallDirectory $attempt.before.stage $destination
    Set-InstallPhase $attempt 'committed'
    Restore-InstallAttempt (Get-InstallAttempt $journal $destination)
    Check (-not (Test-Path -LiteralPath $destination)) 'Fresh installation rollback retained the installed directory.'
    Check (Test-InstallGroup $attempt.before.discard $attempt.before.new_hashes) 'Fresh rollback lost the rejected binaries.'
    Check ((Remove-InstallPathEntry 'A;;D:\Apps\bin;B' 'D:\Apps\bin') -eq 'A;;B') 'PATH restoration changed unrelated entries.'
    foreach ($path in @('', ';A', 'A;;', 'A;;B;')) {
        Check ((Remove-InstallPathEntry (Add-InstallPathEntry $path 'D:\Apps\bin') 'D:\Apps\bin') -ceq $path) 'Installation and rollback changed existing PATH entries.'
    }
    Write-Host 'Fresh rollback and PATH entry preservation passed.'
    if ($IsWindows -and -not $Portable) {
        $directory = Join-Path $root 'native-lock'
        New-InstallPrivateDirectory $directory
        foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
            [OpenCoderInstallFiles]::Copy((Join-Path $env:WINDIR 'System32/whoami.exe'), (Join-Path $directory $name))
        }
        $leases = Lock-InstallGroup $directory (Get-InstallGroup $directory)
        try {
            foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
                $failed = $false
                $child = $null
                try {
                    $start = [Diagnostics.ProcessStartInfo]::new((Join-Path $directory $name))
                    $start.UseShellExecute = $false
                    $start.RedirectStandardOutput = $true
                    $child = [Diagnostics.Process]::Start($start)
                } catch [ComponentModel.Win32Exception] { $failed = $true }
                finally { if ($child) { $child.Kill(); $child.WaitForExit(); $child.Dispose() } }
                Check $failed 'An executable launched while the binary group was locked.'
            }
        } finally { foreach ($lease in $leases) { $lease.Dispose() } }
        $target = "$directory-renamed"
        Move-InstallDirectory $directory $target
        $start.FileName = Join-Path $target 'opencoder.exe'
        $child = [Diagnostics.Process]::Start($start)
        try {
            Check ($child.WaitForExit(10000)) 'Unlocked executable did not exit.'
            Check ($child.ExitCode -eq 0) 'Unlocked executable could not run.'
        } finally { $child.Dispose() }
        Write-Host 'Native binary locks block launches; directory rename succeeds after handles close.'
    }
    foreach ($point in @('prepared', 'old_moved', 'new_moved', 'discarded', 'old_restored')) {
        $case = Join-Path $root "crash-$point"
        $quote = { param($Value) "'$($Value.Replace("'", "''"))'" }
        $command = "& $(& $quote $PSCommandPath) -ChildCase $(& $quote $case) -StopAt '$point'"
        if ($Portable) { $command += ' -Portable' }
        $start = [Diagnostics.ProcessStartInfo]::new([Diagnostics.Process]::GetCurrentProcess().MainModule.FileName)
        $start.UseShellExecute = $false
        foreach ($argument in @('-NoProfile', '-NonInteractive', '-EncodedCommand', [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command)))) { $start.ArgumentList.Add($argument) }
        $child = [Diagnostics.Process]::Start($start)
        try {
            $deadline = [DateTime]::UtcNow.AddSeconds(30)
            $ready = Join-Path $case 'ready.json'
            while (-not (Test-Path -LiteralPath $ready)) {
                if ($child.HasExited -or [DateTime]::UtcNow -ge $deadline) { throw "Installer did not reach $point." }
                Start-Sleep -Milliseconds 50
            }
            $receipt = Read-InstallJson $ready
            $child.Kill($true)
            Check ($child.WaitForExit(10000)) 'Interrupted installer did not exit.'
        } finally {
            if (-not $child.HasExited) { $child.Kill($true); $child.WaitForExit() }
            $child.Dispose()
        }
        $destination = Join-Path $case 'bin'
        $journal = "$destination.install-state"
        $loaded = Get-InstallAttempt $journal $destination
        Restore-InstallAttempt $loaded
        Check (Test-InstallGroup $destination $loaded.before.old_hashes) "Killed installer did not restore at $point."
        Check ((Get-FileHash -LiteralPath (Join-Path $loaded.directory 'before.json')).Hash -eq $receipt.before_hash) 'Crash recovery changed the before-image.'
        Restore-InstallAttempt (Get-InstallAttempt $journal $destination)
        Write-Host "Process crash recovery passed: $point"
    }
} finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}
