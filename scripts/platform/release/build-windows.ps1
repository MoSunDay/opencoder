#Requires -Version 7.4
[CmdletBinding()]
param([string]$Output)
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne [Runtime.InteropServices.Architecture]::X64) { throw 'Build on Windows x64 with the MSVC Rust toolchain.' }
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../../..')).Path
Push-Location $repo
$stage = $null
$savedRustFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    $status = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Release build requires a clean Git worktree and index.' }
    $commit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read Git commit.' }
    if (-not $Output) { $Output = Join-Path $repo "dist/opencoder-windows-x64-$($commit.Substring(0, 8)).zip" }
    $Output = [IO.Path]::GetFullPath($Output)
    if ((Test-Path $Output) -or (Test-Path "$Output.sha256")) { throw "Release output exists: $Output" }
    # Keep the ZIP usable without a separate Visual C++ runtime installation.
    $env:CARGO_ENCODED_RUSTFLAGS = "-C$([char]31)target-feature=+crt-static"
    cargo build --release --locked --target x86_64-pc-windows-msvc -p opencoder -p opencoder-agent
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed.' }
    $metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve Cargo output directory.' }
    $currentCommit = (git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $currentCommit -ne $commit) { throw 'Git commit changed during the release build.' }
    $status = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $status) { throw 'Git worktree changed during the release build.' }
    $parent = Split-Path $Output -Parent
    New-Item -ItemType Directory -Force $parent | Out-Null
    $stage = Join-Path $parent "windows-package-$([guid]::NewGuid())"
    New-Item -ItemType Directory (Join-Path $stage 'bin') | Out-Null
    $binaries = @('opencoder.exe', 'opencoder-agent.exe')
    $info = $null
    $entries = foreach ($name in $binaries) {
        $source = Join-Path $metadata.target_directory "x86_64-pc-windows-msvc/release/$name"
        $destination = Join-Path $stage "bin/$name"
        Copy-Item -LiteralPath $source -Destination $destination
        $raw = & $destination --build-info
        if ($LASTEXITCODE -ne 0) { throw "Cannot verify $name" }
        $current = $raw | ConvertFrom-Json
        if ($current.git_commit -ne $commit -or $current.git_dirty) { throw "Build metadata does not identify the clean commit: $name" }
        $canonical = $current | ConvertTo-Json -Depth 20 -Compress
        if ($info -and $canonical -ne $info) { throw 'Release binaries have different build metadata.' }
        $info = $canonical
        @{ name = $name; sha256 = (Get-FileHash -Algorithm SHA256 $destination).Hash.ToLowerInvariant() }
    }
    @{ platform = 'windows-x64'; schema_version = 1; build_info = ($info | ConvertFrom-Json); binaries = @($entries) } |
        ConvertTo-Json -Depth 20 | Set-Content -Encoding utf8NoBOM (Join-Path $stage 'manifest.json')
    Copy-Item (Join-Path $repo 'scripts/install.ps1') $stage
    Copy-Item (Join-Path $repo 'scripts/platform/windows/installer') (Join-Path $stage 'installer') -Recurse
    Copy-Item (Join-Path $repo 'docs/windows.md') (Join-Path $stage 'README.md')
    $sums = Get-ChildItem -File -Recurse $stage | Sort-Object FullName | ForEach-Object {
        $relative = [IO.Path]::GetRelativePath($stage, $_.FullName).Replace('\', '/')
        "$((Get-FileHash -Algorithm SHA256 $_.FullName).Hash.ToLowerInvariant())  $relative"
    }
    $sums | Set-Content -Encoding utf8NoBOM (Join-Path $stage 'SHA256SUMS')
    $zip = "$Output.new.zip"
    Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
    Move-Item -LiteralPath $zip -Destination $Output
    "$((Get-FileHash -Algorithm SHA256 $Output).Hash.ToLowerInvariant())  $([IO.Path]::GetFileName($Output))" |
        Set-Content -Encoding utf8NoBOM "$Output.sha256"
    Write-Host "Windows package: $Output"
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $savedRustFlags
    if ($stage -and (Test-Path $stage)) { Remove-Item -LiteralPath $stage -Recurse -Force }
    Pop-Location
}
