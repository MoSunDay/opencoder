# Destination checks shared by manifest downloads and archive extraction.
function Assert-CuaFilesystemPath([string]$Path) {
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        $entry = $null
        try { $entry = Get-Item -LiteralPath $cursor -Force -ErrorAction Stop }
        catch [Management.Automation.ItemNotFoundException] { }
        if ($entry) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw 'Artifact destination contains a reparse point'
            }
        }
        $parent = [IO.Path]::GetDirectoryName($cursor)
        if ($parent -eq $cursor) { break }
        $cursor = $parent
    }
}

function Get-CuaLiteral([string]$Value) { return "'" + $Value.Replace("'", "''") + "'" }

function Get-CuaDestination([string]$Root, [string]$Relative) {
    $relativePath = $Relative.Replace('\', '/')
    if (-not $relativePath -or $relativePath.StartsWith('/') -or $relativePath.Contains(':')) {
        throw 'Artifact path must be relative'
    }
    $parts = $relativePath.Split('/')
    foreach ($part in $parts) {
        if (-not $part -or $part -eq '.' -or $part -eq '..' -or
            $part.EndsWith('.') -or $part.EndsWith(' ') -or
            $part.IndexOfAny([IO.Path]::GetInvalidFileNameChars()) -ge 0 -or
            $part -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') {
            throw 'Artifact path contains an invalid Windows component'
        }
    }
    $base = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $target = [IO.Path]::GetFullPath((Join-Path $base ($parts -join [IO.Path]::DirectorySeparatorChar)))
    if (-not $target.StartsWith($base, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Artifact destination escapes the installation directory'
    }
    Assert-CuaFilesystemPath $target
    return $target
}

function Get-CuaArtifactPlan([string]$Root, [object[]]$Manifest) {
    Assert-CuaFilesystemPath $Root
    $required = @('uv.zip', 'python.tar.gz', 'cua_computer_server-0.3.46-py3-none-any.whl',
                  'windows-requirements.txt')
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $plan = foreach ($item in $Manifest) {
        if ($null -eq $item -or $item.file -isnot [string] -or $item.sha256 -isnot [string]) {
            throw 'Artifact manifest requires file and SHA256 strings'
        }
        $name = $item.file.Replace('\', '/')
        if ($name -cnotin $required -and $name -cnotmatch '^packages/[A-Za-z0-9][A-Za-z0-9_.+-]*\.whl$') {
            throw 'Artifact manifest contains an unexpected file'
        }
        if ($item.sha256 -notmatch '^[0-9a-fA-F]{64}$' -or -not $seen.Add($name)) {
            throw 'Artifact manifest contains an invalid hash or duplicate destination'
        }
        [pscustomobject]@{
            file = $name
            sha256 = $item.sha256.ToLowerInvariant()
            target = Get-CuaDestination $Root $name
        }
    }
    foreach ($name in $required) {
        if (-not $seen.Contains($name)) { throw 'Artifact manifest is missing a required file' }
    }
    return $plan
}

function Save-CuaArtifact([string]$Root, [string]$ArtifactBase, $Item) {
    $target = Get-CuaDestination $Root $Item.file
    if ((Test-Path -LiteralPath $target -PathType Leaf) -and
        (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -eq $Item.sha256) {
        return
    }
    $temporary = Join-Path $Root ('.download-' + [guid]::NewGuid().ToString('N'))
    try {
        New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
        Assert-CuaFilesystemPath $temporary
        Invoke-WebRequest "$ArtifactBase/$($Item.file)" -OutFile $temporary -UseBasicParsing
        if ((Get-FileHash -LiteralPath $temporary -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Item.sha256) {
            throw 'Artifact hash mismatch'
        }
        Assert-CuaFilesystemPath $target
        Move-Item -LiteralPath $temporary -Destination $target -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}
