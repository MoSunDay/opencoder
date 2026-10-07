param([string]$Parent = [IO.Path]::GetTempPath())
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'artifacts.ps1')
. (Join-Path $PSScriptRoot 'archives.ps1')
$testRoot = Join-Path $Parent ('opencoder-cua-paths-' + [guid]::NewGuid())
$installation = Join-Path $testRoot "安装 O'Brien"
$checks = [Collections.Generic.List[string]]::new()
function Check([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw "Failed: $Name" }
    $checks.Add($Name)
}
function Must-Reject([scriptblock]$Action, [string]$Name) {
    $failed = $false
    try { & $Action | Out-Null } catch { $failed = $true }
    Check $failed $Name
}
function Manifest {
    foreach ($name in @('uv.zip', 'python.tar.gz', 'cua_computer_server-0.3.46-py3-none-any.whl',
                       'windows-requirements.txt')) {
        [pscustomobject]@{file=$name;sha256=('a' * 64)}
    }
}
function New-TestZip([string]$Path, [string[]]$Names, [bool]$Link = $false) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew)
    $zip = $null
    try {
        $zip = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create)
        foreach ($name in $Names) {
            $entry = $zip.CreateEntry($name)
            if ($Link) { $entry.ExternalAttributes = 0xA000 -shl 16 }
        }
    } finally { if ($zip) { $zip.Dispose() }; $stream.Dispose() }
}
function Write-TarField([byte[]]$Header, [int]$Offset, [string]$Value) {
    $bytes = [Text.Encoding]::ASCII.GetBytes($Value)
    [Array]::Copy($bytes, 0, $Header, $Offset, $bytes.Length)
}
function New-TestTar([string]$Path, [string]$Name, [string]$Type = '0') {
    $header = [byte[]]::new(512)
    Write-TarField $header 0 $Name
    Write-TarField $header 100 '0000644'
    Write-TarField $header 108 '0000000'
    Write-TarField $header 116 '0000000'
    Write-TarField $header 124 '00000000000'
    Write-TarField $header 136 '00000000000'
    Write-TarField $header 148 '        '
    Write-TarField $header 156 $Type
    if ($Type -in @('1', '2')) { Write-TarField $header 157 '../outside.txt' }
    Write-TarField $header 257 'ustar'
    Write-TarField $header 263 '00'
    $sum = ($header | Measure-Object -Sum).Sum
    Write-TarField $header 148 ([Convert]::ToString([int]$sum, 8).PadLeft(6, '0') + "`0 ")
    $bytes = [byte[]]::new(1536)
    [Array]::Copy($header, $bytes, 512)
    [IO.File]::WriteAllBytes($Path, $bytes)
}
function Invoke-WebRequest($Uri, $OutFile, [switch]$UseBasicParsing) {
    [IO.File]::WriteAllBytes($OutFile, $script:DownloadBytes)
}
function Invoke-RestMethod($Uri) { return $script:InvalidManifest }
try {
    New-Item -ItemType Directory -Path $testRoot | Out-Null
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $valid = @(Manifest)
    $plan = @(Get-CuaArtifactPlan $installation $valid)
    Check ($plan.Count -eq 4 -and -not (Test-Path -LiteralPath $installation)) 'valid manifest is read only'
    $wheel = [pscustomobject]@{file='packages/pkg-1.0-py3-none-any.whl';sha256=('b' * 64)}
    Check (@(Get-CuaArtifactPlan $installation ($valid + $wheel)).Count -eq 5) 'offline wheels are allowed'
    foreach ($name in @('../outside.txt', '..\outside.txt', 'C:\outside.txt', '/outside.txt',
                        'uv.zip:stream', 'packages/CON.whl', 'packages/pkg.whl.', 'packages/pkg.whl ')) {
        $bad = [pscustomobject]@{file=$name;sha256=('a' * 64)}
        Must-Reject { Get-CuaArtifactPlan $installation ($valid + $bad) } "reject manifest $name"
    }
    Must-Reject { Get-CuaArtifactPlan $installation ($valid + $valid[0]) } 'reject duplicate destinations'
    Must-Reject { Get-CuaArtifactPlan $installation @($valid[0]) } 'reject incomplete manifest'
    $badHash = @(Manifest); $badHash[3].sha256 = 'wrong'
    Must-Reject { Get-CuaArtifactPlan $installation $badHash } 'reject invalid SHA256'
    $script:InvalidManifest = $valid + [pscustomobject]@{file='../outside.txt';sha256=('a' * 64)}
    Must-Reject {
        & (Join-Path $PSScriptRoot '../install-windows.ps1') -ArtifactBase 'http://fixture.invalid' -Root $installation
    } 'installer rejects invalid full manifest before writes'
    Check (-not (Test-Path -LiteralPath $installation)) 'invalid installer manifest produces zero files'

    New-Item -ItemType Directory -Path $installation | Out-Null
    $outside = Join-Path $testRoot 'outside'; New-Item -ItemType Directory -Path $outside | Out-Null
    $sentinel = Join-Path $outside 'keep.txt'; [IO.File]::WriteAllText($sentinel, 'unchanged')
    $junction = Join-Path $installation 'packages'
    try {
        New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
        Must-Reject { Get-CuaArtifactPlan $installation ($valid + $wheel) } 'reject destination junction'
    } finally { if (Test-Path -LiteralPath $junction) { [IO.Directory]::Delete($junction) } }
    $missingTarget = Join-Path $testRoot 'missing-target'
    New-Item -ItemType Directory -Path $missingTarget | Out-Null
    try {
        New-Item -ItemType Junction -Path $junction -Target $missingTarget | Out-Null
        [IO.Directory]::Delete($missingTarget)
        Must-Reject { Get-CuaArtifactPlan $installation ($valid + $wheel) } 'reject dangling destination junction'
    } finally { [IO.Directory]::Delete($junction) }

    $safeZip = Join-Path $testRoot 'safe.zip'; New-TestZip $safeZip @('uv.exe', 'licenses/', 'licenses/LICENSE')
    Assert-CuaZip $installation $safeZip 'uv'; $checks.Add('accept regular ZIP entries')
    foreach ($name in @('../outside.txt', '/outside.txt', 'C:\outside.txt', 'uv.exe:stream', 'NUL.txt')) {
        $archive = Join-Path $testRoot ([guid]::NewGuid().ToString() + '.zip')
        New-TestZip $archive @('uv.exe', $name)
        Must-Reject { Assert-CuaZip $installation $archive 'uv' } "reject ZIP $name"
    }
    $duplicate = Join-Path $testRoot 'duplicate.zip'; New-TestZip $duplicate @('uv.exe', 'UV.EXE')
    Must-Reject { Assert-CuaZip $installation $duplicate 'uv' } 'reject ZIP case aliases'
    $linked = Join-Path $testRoot 'linked.zip'; New-TestZip $linked @('link') $true
    Must-Reject { Assert-CuaZip $installation $linked 'uv' } 'reject ZIP symbolic links'
    $safeTar = Join-Path $testRoot 'safe.tar'; New-TestTar $safeTar './python/python.exe'
    Assert-CuaTar $installation $safeTar 'runtime'; $checks.Add('accept regular tar entries')
    foreach ($case in @(@('../outside.txt', '0'), @('link', '1'), @('link', '2'), @('pipe', '6'))) {
        $archive = Join-Path $testRoot ([guid]::NewGuid().ToString() + '.tar')
        New-TestTar $archive $case[0] $case[1]
        Must-Reject { Assert-CuaTar $installation $archive 'runtime' } "reject tar $($case -join ' ')"
    }
    $target = Join-Path $installation 'uv.zip'; [IO.File]::WriteAllText($target, 'old')
    $script:DownloadBytes = [Text.Encoding]::UTF8.GetBytes('wrong')
    Must-Reject { Save-CuaArtifact $installation 'http://fixture.invalid' $plan[0] } 'reject downloaded hash mismatch'
    Check ([IO.File]::ReadAllText($target) -eq 'old') 'failed download preserves existing artifact'
    Check (@(Get-ChildItem -LiteralPath $installation -Filter '.download-*').Count -eq 0) 'failed download cleans staging file'
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $plan[0].sha256 = [BitConverter]::ToString($sha.ComputeHash($script:DownloadBytes)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
    Save-CuaArtifact $installation 'http://fixture.invalid' $plan[0]
    Check ([IO.File]::ReadAllText($target) -eq 'wrong') 'verified download replaces one artifact'
    Check ([IO.File]::ReadAllText($sentinel) -eq 'unchanged') 'outside-root sentinel remains unchanged'
    Check ((Get-CuaLiteral "C:\O'Brien\python.exe") -eq "'C:\O''Brien\python.exe'") 'launcher paths preserve apostrophes'
    @{passed=$true;checks=@($checks);platform=[Environment]::OSVersion.VersionString} | ConvertTo-Json -Depth 5 -Compress
} finally {
    if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
