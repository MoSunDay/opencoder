# This prefix belongs to the executor, never to the inspected user script.
$PSNativeCommandArgumentPassing = 'Standard'
# Remove inherited Git overrides, including Windows handle redirects and trace
# files. The changes apply only to this short-lived PowerShell process.
foreach ($key in @([Environment]::GetEnvironmentVariables().Keys)) {
    if ([string]$key -like 'GIT_*') { [Environment]::SetEnvironmentVariable([string]$key, $null, 'Process') }
}
$env:GIT_CONFIG_NOSYSTEM = '1'
$env:GIT_CONFIG_GLOBAL = '/dev/null'
$env:GIT_OPTIONAL_LOCKS = '0'
$env:GIT_NO_LAZY_FETCH = '1'
$env:RIPGREP_CONFIG_PATH = $null
function Get-OpenCoderNative([string]$Name) {
    $native = Microsoft.PowerShell.Core\Get-Command -Name $Name -CommandType Application -ErrorAction Stop |
        Microsoft.PowerShell.Utility\Select-Object -First 1
    if ($IsWindows -and [IO.Path]::GetExtension($native.Source) -ne '.exe') {
        throw "Read-only commands require the native $Name executable."
    }
    return $native.Source
}
function Invoke-OpenCoderGit {
    param([string[]]$Arguments)
    $native = Get-OpenCoderNative 'git'
    $prefix = @('--no-pager', '--no-optional-locks', '--no-replace-objects',
        '-c', 'core.fsmonitor=false', '-c', 'core.untrackedCache=false',
        '-c', 'log.showSignature=false', '-c', 'format.pretty=medium',
        '-c', 'diff.submodule=short', '-c', 'status.submoduleSummary=false',
        '-c', 'submodule.recurse=false', '-c', 'protocol.allow=never')
    # Content filters can run during ordinary status/diff, and partial clones
    # can fetch missing objects. Reject these configurations before a query.
    $unsupported = & $native @prefix config --name-only --get-regexp '^(filter\..*\.(clean|smudge|process|required)|extensions\.partialclone|remote\..*\.promisor)$' 2>$null
    if ($LASTEXITCODE -eq 0) { throw 'Read-only Git queries do not support content filters or partial clones.' }
    if ($LASTEXITCODE -ne 1) { throw 'Cannot inspect the repository configuration for a read-only Git query.' }
    $safe = @($Arguments[0])
    if ($Arguments[0] -in @('diff', 'show', 'log')) { $safe += @('--no-ext-diff', '--no-textconv') }
    if ($Arguments[0] -eq 'grep') { $safe += '--no-textconv' }
    if ($Arguments.Count -gt 1) { $safe += $Arguments[1..($Arguments.Count - 1)] }
    if ($MyInvocation.ExpectingInput) { $input | & $native @prefix @safe }
    else { & $native @prefix @safe }
}
function Invoke-OpenCoderRg {
    param([string[]]$Arguments)
    $native = Get-OpenCoderNative 'rg'
    if ($MyInvocation.ExpectingInput) { $input | & $native --no-config @Arguments }
    else { & $native --no-config @Arguments }
}
