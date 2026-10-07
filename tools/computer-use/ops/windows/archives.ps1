# All members are checked before extracting any archive.
function Assert-CuaArchiveMembers([string]$Root, [string]$Directory, [object[]]$Names) {
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($name in $Names) {
        $relative = ([string]$name).Replace('\', '/').TrimEnd('/')
        while ($relative.StartsWith('./')) { $relative = $relative.Substring(2) }
        if ($relative -eq '.') { continue }
        $target = Get-CuaDestination $Root ($Directory + '/' + $relative)
        if (-not $seen.Add($target)) { throw 'Archive contains duplicate destinations' }
    }
}

function Assert-CuaZip([string]$Root, [string]$Archive, [string]$Directory) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $names = foreach ($entry in $zip.Entries) {
            if ((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
                throw 'Archive symbolic links are forbidden'
            }
            $entry.FullName
        }
        Assert-CuaArchiveMembers $Root $Directory @($names)
    } finally {
        $zip.Dispose()
    }
}

function Assert-CuaTar([string]$Root, [string]$Archive, [string]$Directory) {
    $names = @(& tar.exe -tf $Archive)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Python archive members' }
    $details = @(& tar.exe -tvf $Archive)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect Python archive entry types' }
    foreach ($line in $details) {
        if (-not $line -or $line[0] -cnotin @('-', 'd')) {
            throw 'Archive links and special files are forbidden'
        }
    }
    Assert-CuaArchiveMembers $Root $Directory $names
}
