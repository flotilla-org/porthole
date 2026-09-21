param([Parameter(Mandatory=$true)][string]$RunDirectory, [string]$CleatExecutable)
$ErrorActionPreference = 'Stop'
$config = Get-Content -LiteralPath (Join-Path $RunDirectory 'config.json') -Raw | ConvertFrom-Json
$env:CLEAT_RUNTIME_DIR = $config.runtime
if (-not $CleatExecutable) { $CleatExecutable = $config.cleat }
& $CleatExecutable --server $config.server attach $config.session --no-create --identity beaufort-local
exit $LASTEXITCODE
