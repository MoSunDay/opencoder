$text = [Console]::In.ReadToEnd() | ConvertFrom-Json
$tokens = $null
$errors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseInput($text, [ref]$tokens, [ref]$errors)
$allowed = @('ScriptBlockAst', 'NamedBlockAst', 'PipelineAst', 'CommandAst', 'CommandParameterAst', 'StringConstantExpressionAst', 'ConstantExpressionAst')
$errorText = $null
if ($errors.Count -ne 0) { $errorText = 'invalid PowerShell syntax' }
$commands = @()
$aliases = @{ 'cat' = 'Get-Content'; 'type' = 'Get-Content'; 'ls' = 'Get-ChildItem'; 'dir' = 'Get-ChildItem'; 'pwd' = 'Get-Location'; 'echo' = 'Write-Output' }
foreach ($node in $ast.FindAll({ param($node) $true }, $true)) {
    if ($node.GetType().Name -notin $allowed) { $errorText = "unsupported read-only PowerShell syntax: $($node.GetType().Name)"; break }
    if ($node -is [System.Management.Automation.Language.CommandAst]) {
        if ($node.InvocationOperator -ne [System.Management.Automation.Language.TokenKind]::Unknown) { $errorText = 'dynamic or indirect command invocation is blocked'; break }
        $name = $node.GetCommandName()
        if (-not $name) { $errorText = 'dynamic command name is blocked'; break }
        if ($aliases.ContainsKey($name)) { $name = $aliases[$name] }
        $arguments = @()
        foreach ($element in $node.CommandElements | Select-Object -Skip 1) {
            if ($element -is [System.Management.Automation.Language.CommandParameterAst]) {
                if ($element.Argument) { $errorText = 'attached parameter expressions are blocked'; break }
                $arguments += "-$($element.ParameterName)"
            } elseif ($element -is [System.Management.Automation.Language.StringConstantExpressionAst] -or $element -is [System.Management.Automation.Language.ConstantExpressionAst]) {
                $arguments += [string]$element.Value
            } else { $errorText = 'dynamic command argument is blocked'; break }
        }
        $first = $node.CommandElements[0].Extent
        $commands += @{ name = $name; args = @($arguments); start = $node.Extent.StartOffset; end = $node.Extent.EndOffset; name_start = $first.StartOffset; name_end = $first.EndOffset }
    }
}
@{ commands = @($commands); error = $errorText } | ConvertTo-Json -Compress -Depth 8
