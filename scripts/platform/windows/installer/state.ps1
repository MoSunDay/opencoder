function Test-InstallPathEntry([string]$Path, [string]$Destination) {
    return [bool]@($Path -split ';' | Where-Object { $_.TrimEnd('\', '/').Equals($Destination, [StringComparison]::OrdinalIgnoreCase) }).Count
}
function Remove-InstallPathEntry([string]$Path, [string]$Destination) {
    return (($Path -split ';' | Where-Object { -not $_.TrimEnd('\', '/').Equals($Destination, [StringComparison]::OrdinalIgnoreCase) }) -join ';')
}
function Add-InstallPathEntry([string]$Path, [string]$Destination) {
    if (Test-InstallPathEntry $Path $Destination) { return $Path }
    if ($Path.Length -eq 0) { return $Destination }
    return "$Path;$Destination"
}
function Get-InstallGroup([string]$Directory) {
    Assert-InstallPath $Directory
    $hashes = [ordered]@{}
    foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
        $file = Join-Path $Directory $name
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Incomplete installation: $name" }
        Assert-InstallPath $file
        $hashes[$name] = (Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant()
    }
    return $hashes
}
function Test-InstallGroup([string]$Directory, $Hashes) {
    if (-not (Test-Path -LiteralPath $Directory -PathType Container)) { return $false }
    $actual = Get-InstallGroup $Directory
    foreach ($name in $actual.Keys) { if ($actual[$name] -ne $Hashes[$name]) { return $false } }
    return $true
}
function Lock-InstallGroup([string]$Directory, $ExpectedHashes) {
    $leases = [Collections.Generic.List[IO.FileStream]]::new()
    try {
        foreach ($name in @('opencoder.exe', 'opencoder-agent.exe')) {
            # Validate the complete group while excluding image loading and
            # writes. Close these handles before renaming a Windows directory.
            $file = [IO.File]::Open((Join-Path $Directory $name), [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite, ([IO.FileShare]::Read -bor [IO.FileShare]::Delete))
            $leases.Add($file)
            $digest = [Security.Cryptography.SHA256]::Create()
            try { $hash = [BitConverter]::ToString($digest.ComputeHash($file)).Replace('-', '').ToLowerInvariant() } finally { $digest.Dispose() }
            if ($hash -ne $ExpectedHashes[$name]) { throw 'Binary group changed before it could be locked.' }
        }
        return ,$leases
    } catch {
        foreach ($lease in $leases) { $lease.Dispose() }
        throw 'Exit OpenCoder and release the locked executables before installing or restoring.'
    }
}
function Get-InstallAttempt([string]$Root, [string]$Destination) {
    $active = Join-Path $Root 'active.json'
    if (-not (Test-Path -LiteralPath $active)) { return $null }
    $pointer = Read-InstallJson $active
    if ($pointer.id -notmatch '^[0-9a-f]{32}$') { throw 'Invalid installation journal identity.' }
    $directory = Join-Path $Root $pointer.id
    $before = Read-InstallJson (Join-Path $directory 'before.json')
    if ($before.id -ne $pointer.id -or $before.destination -ne $Destination -or
        $before.stage -ne (Join-Path $directory 'stage') -or
        $before.discard -ne (Join-Path $directory 'discard') -or
        $before.backup -ne "$Destination.previous-$($pointer.id)") { throw 'Installation journal paths do not match this destination.' }
    foreach ($path in @($directory, $before.stage, $before.discard, $before.backup)) { Assert-InstallPath $path }
    $progress = Join-Path $directory 'state.json'
    $state = if (Test-Path -LiteralPath $progress) { (Read-InstallJson $progress).phase } else { 'prepared' }
    if ($state -notin @('prepared', 'switching', 'installed', 'committed', 'restoring', 'restored')) { throw 'Unknown installation journal phase.' }
    return @{ directory=$directory; before=$before; phase=$state }
}
function Set-InstallPhase($Attempt, [string]$Phase) {
    Write-InstallJson (Join-Path $Attempt.directory 'state.json') @{ phase=$Phase }
    $Attempt.phase = $Phase
}
function New-InstallAttempt([string]$Root, [string]$Destination, [bool]$NoPath) {
    $id = [guid]::NewGuid().ToString('N')
    $directory = Join-Path $Root $id
    New-InstallPrivateDirectory $directory
    $exists = Test-Path -LiteralPath $Destination
    $before = [ordered]@{
        id=$id; destination=$Destination; stage=(Join-Path $directory 'stage');
        discard=(Join-Path $directory 'discard'); backup="$Destination.previous-$id";
        had_destination=[bool]$exists; old_hashes=$(if ($exists) { Get-InstallGroup $Destination } else { @{} });
        path_before=[Environment]::GetEnvironmentVariable('Path', 'User'); no_path=$NoPath
    }
    return @{ directory=$directory; before=$before; phase='prepared' }
}
function Publish-InstallAttempt([string]$Root, $Attempt, $Hashes) {
    $Attempt.before.new_hashes = $Hashes
    Write-InstallJson (Join-Path $Attempt.directory 'before.json') $Attempt.before -CreateOnly
    Write-InstallJson (Join-Path $Root 'active.json') @{ id=$Attempt.before.id }
}
function Confirm-InstallRestored($Before) {
    if ($Before.had_destination) {
        if (-not (Test-InstallGroup $Before.destination $Before.old_hashes)) { throw 'Restored installation verification failed.' }
    } elseif (Test-Path -LiteralPath $Before.destination) { throw 'Fresh installation restoration failed.' }
    if (-not $Before.no_path -and (Test-InstallPathEntry ([Environment]::GetEnvironmentVariable('Path', 'User')) $Before.destination) -ne
        (Test-InstallPathEntry $Before.path_before $Before.destination)) { throw 'Restored PATH verification failed.' }
}
function Restore-InstallAttempt($Attempt) {
    $before = $Attempt.before
    if ($Attempt.phase -eq 'restored') { Confirm-InstallRestored $before; return }
    Set-InstallPhase $Attempt 'restoring'
    $leases = [Collections.Generic.List[IO.FileStream]]::new()
    try {
        $backupExists = Test-Path -LiteralPath $before.backup
        $destinationExists = Test-Path -LiteralPath $before.destination
        if ($backupExists) {
            if (-not $before.had_destination -or -not (Test-InstallGroup $before.backup $before.old_hashes)) { throw 'Previous binaries changed; restoration stopped.' }
            foreach ($lease in (Lock-InstallGroup $before.backup $before.old_hashes)) { $leases.Add($lease) }
            if ($destinationExists) {
                if (-not (Test-InstallGroup $before.destination $before.new_hashes) -or (Test-Path -LiteralPath $before.discard)) { throw 'Current binaries changed; restoration stopped.' }
                foreach ($lease in (Lock-InstallGroup $before.destination $before.new_hashes)) { $leases.Add($lease) }
            }
            foreach ($lease in $leases) { $lease.Dispose() }
            $leases.Clear()
            if ($destinationExists) { Move-InstallDirectory $before.destination $before.discard }
            Move-InstallDirectory $before.backup $before.destination
        } elseif ($before.had_destination) {
            if (-not (Test-InstallGroup $before.destination $before.old_hashes)) { throw 'Previous installation is unavailable; restoration stopped.' }
        } elseif ($destinationExists) {
            if (-not (Test-InstallGroup $before.destination $before.new_hashes) -or (Test-Path -LiteralPath $before.discard)) { throw 'Unexpected installation; restoration stopped.' }
            foreach ($lease in (Lock-InstallGroup $before.destination $before.new_hashes)) { $leases.Add($lease) }
            foreach ($lease in $leases) { $lease.Dispose() }
            $leases.Clear()
            Move-InstallDirectory $before.destination $before.discard
        }
    } finally { foreach ($lease in $leases) { $lease.Dispose() } }
    if (-not $before.no_path -and -not (Test-InstallPathEntry $before.path_before $before.destination)) {
        $current = [Environment]::GetEnvironmentVariable('Path', 'User')
        $restored = Remove-InstallPathEntry $current $before.destination
        if ($restored.Length -eq 0 -and $null -eq $before.path_before) { $restored = $null }
        [Environment]::SetEnvironmentVariable('Path', $restored, 'User')
    }
    Confirm-InstallRestored $before
    Set-InstallPhase $Attempt 'restored'
}
